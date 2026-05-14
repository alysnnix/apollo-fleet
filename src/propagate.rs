// File-watch the master seat's sunshine.conf and propagate shared settings to the others.
//
// When the user saves config via Apollo's web UI on the master, this module:
//  1. Parses the new master conf.
//  2. For each non-master seat, rewrites its sunshine.conf:
//      - Per-seat keys (port, name, audio_sink, paths, gamepad-only flags) preserved
//        from the seat's own current conf.
//      - Everything else taken from the master.
//      - Apollo Fleet's managed keys (system_tray=disabled, etc.) always re-applied.
//  3. Triggers a restart of that seat via the callback.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use notify::{Config, EventKind, RecommendedWatcher, RecursiveMode, Watcher};

/// Keys that are tied to a specific seat and must NEVER be copied from master.
const PER_SEAT_KEYS: &[&str] = &[
    "sunshine_name",
    "port",
    "audio_sink",
    "file_apps",
    "file_state",
    "log_path",
    "cert",
    "pkey",
    "credentials_file",
    "keyboard",
    "mouse",
    "controller",
];

/// Keys that Apollo Fleet manages — re-applied verbatim after every propagation so the
/// user can't accidentally turn them off via the master web UI.
const ALWAYS_OURS: &[(&str, &str)] = &[
    ("system_tray", "disabled"),
    ("headless_mode", "enabled"),
    ("dd_configuration_option", "ensure_active"),
];

const DEBOUNCE: Duration = Duration::from_millis(500);

#[derive(Debug, Clone)]
pub struct SeatRef {
    pub name: String,
    pub config_path: PathBuf,
}

pub struct PropagatorHandle {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl PropagatorHandle {
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.join.take() {
            let _ = h.join();
        }
    }
}

/// Start watching `master.config_path`. `on_propagate(seat_name)` is invoked after a
/// non-master seat's conf has been rewritten so the caller can restart that seat.
pub fn spawn<F>(
    master: SeatRef,
    others: Vec<SeatRef>,
    on_propagate: F,
) -> PropagatorHandle
where
    F: Fn(&str) + Send + 'static,
{
    let stop = Arc::new(AtomicBool::new(false));
    let stop_inner = stop.clone();

    let join = std::thread::spawn(move || {
        let (tx, rx) = std::sync::mpsc::channel::<notify::Result<notify::Event>>();
        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                log::error!("[propagate] could not create watcher: {e:#}");
                return;
            }
        };

        // notify expects the file to exist; loop until master conf is on disk.
        while !master.config_path.exists() {
            if stop_inner.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }

        if let Err(e) = watcher.watch(&master.config_path, RecursiveMode::NonRecursive) {
            log::error!(
                "[propagate] could not watch {}: {e:#}",
                master.config_path.display()
            );
            return;
        }
        log::info!(
            "[propagate] watching master '{}' at {}",
            master.name,
            master.config_path.display()
        );

        let mut last_apply = Instant::now() - DEBOUNCE * 2;
        while !stop_inner.load(Ordering::Relaxed) {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(Ok(event)) if is_content_change(&event.kind) => {
                    let now = Instant::now();
                    if now.duration_since(last_apply) < DEBOUNCE {
                        continue;
                    }
                    last_apply = now;
                    if let Err(e) = propagate_once(&master, &others, &on_propagate) {
                        log::warn!("[propagate] cycle failed: {e:#}");
                    }
                }
                Ok(Ok(_)) | Ok(Err(_)) => continue,
                Err(RecvTimeoutError::Timeout) => continue,
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    PropagatorHandle { stop, join: Some(join) }
}

fn is_content_change(kind: &EventKind) -> bool {
    matches!(
        kind,
        EventKind::Modify(_) | EventKind::Create(_) | EventKind::Any
    )
}

fn propagate_once<F>(
    master: &SeatRef,
    others: &[SeatRef],
    on_propagate: &F,
) -> anyhow::Result<()>
where
    F: Fn(&str),
{
    let master_text = std::fs::read_to_string(&master.config_path)?;
    let master_pairs = parse_conf(&master_text);

    for seat in others {
        if !seat.config_path.exists() {
            continue;
        }
        let seat_text = std::fs::read_to_string(&seat.config_path)?;
        let seat_pairs = parse_conf(&seat_text);
        let merged = merge(&master_pairs, &seat_pairs);
        let new_text = write_conf(&merged);
        if new_text == seat_text {
            continue;
        }
        std::fs::write(&seat.config_path, new_text)?;
        log::info!(
            "[propagate] applied master config to '{}', restarting",
            seat.name
        );
        on_propagate(&seat.name);
    }
    Ok(())
}

/// Parse Apollo's conf format: one `key = value` per line. Comments (`#`) and blank
/// lines are kept verbatim so user-edited annotations survive a round-trip.
fn parse_conf(text: &str) -> Vec<ConfLine> {
    let mut out = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            out.push(ConfLine::Raw(line.to_string()));
            continue;
        }
        match line.split_once('=') {
            Some((k, v)) => out.push(ConfLine::Pair(k.trim().to_string(), v.trim().to_string())),
            None => out.push(ConfLine::Raw(line.to_string())),
        }
    }
    out
}

fn write_conf(pairs: &[ConfLine]) -> String {
    let mut body = String::new();
    for line in pairs {
        match line {
            ConfLine::Raw(s) => {
                body.push_str(s);
                body.push('\n');
            }
            ConfLine::Pair(k, v) => {
                body.push_str(k);
                body.push_str(" = ");
                body.push_str(v);
                body.push('\n');
            }
        }
    }
    body
}

#[derive(Debug, Clone)]
enum ConfLine {
    Raw(String),
    Pair(String, String),
}

/// Build the new seat conf:
///   - take master's pairs except PER_SEAT_KEYS
///   - reapply seat's own PER_SEAT_KEYS
///   - always-apply our managed keys at the end
fn merge(master: &[ConfLine], seat: &[ConfLine]) -> Vec<ConfLine> {
    let seat_keys: std::collections::HashMap<&str, &str> = seat
        .iter()
        .filter_map(|l| match l {
            ConfLine::Pair(k, v) if PER_SEAT_KEYS.contains(&k.as_str()) => {
                Some((k.as_str(), v.as_str()))
            }
            _ => None,
        })
        .collect();

    let mut out: Vec<ConfLine> = Vec::new();
    let mut seen_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    for line in master {
        match line {
            ConfLine::Raw(_) => continue, // drop master comments; seat conf is regenerated
            ConfLine::Pair(k, v) => {
                if PER_SEAT_KEYS.contains(&k.as_str()) {
                    continue;
                }
                if ALWAYS_OURS.iter().any(|(ak, _)| *ak == k.as_str()) {
                    continue; // applied below with our fixed value
                }
                seen_keys.insert(k.clone());
                out.push(ConfLine::Pair(k.clone(), v.clone()));
            }
        }
    }

    // Reapply per-seat keys from the seat's existing conf so port/name/audio stay.
    for key in PER_SEAT_KEYS {
        if let Some(v) = seat_keys.get(*key) {
            out.push(ConfLine::Pair((*key).to_string(), (*v).to_string()));
            seen_keys.insert((*key).to_string());
        }
    }

    // Apollo Fleet's managed keys win unconditionally.
    for (k, v) in ALWAYS_OURS {
        out.push(ConfLine::Pair((*k).to_string(), (*v).to_string()));
        seen_keys.insert((*k).to_string());
    }

    let _ = seen_keys;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn per_seat_keys_preserved() {
        let master = parse_conf("encoder = nvenc\nport = 47989\nfps = 60\n");
        let seat = parse_conf("port = 48029\naudio_sink = Voicemeeter Input\nencoder = sw\n");
        let merged = write_conf(&merge(&master, &seat));
        assert!(merged.contains("port = 48029"));
        assert!(merged.contains("audio_sink = Voicemeeter Input"));
        assert!(merged.contains("encoder = nvenc"));
        assert!(merged.contains("fps = 60"));
        assert!(merged.contains("system_tray = disabled"));
    }
}
