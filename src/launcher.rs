use std::fs::OpenOptions;
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::paths;
use crate::seat::{DETACHED_PROCESS, NO_WINDOW};
use crate::shared_state;
use crate::win;

pub fn run(seat_name: &str, exec_args: &[String]) -> i32 {
    let log_path: PathBuf = paths::temp_dir().join("apollo-fleet-launcher.log");
    let log = |msg: &str| {
        if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&log_path) {
            let _ = writeln!(
                f,
                "{} [{seat_name}] {msg}",
                chrono_like_now(),
            );
        }
    };

    log(&format!("--launch invoked, exec_args={exec_args:?}"));

    if exec_args.is_empty() {
        log("error: no exec_args");
        return 2;
    }

    let mut cmd = Command::new(&exec_args[0]);
    cmd.args(&exec_args[1..])
        .creation_flags(DETACHED_PROCESS | NO_WINDOW);
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            log(&format!("failed to launch {}: {e}", exec_args[0]));
            return 2;
        }
    };
    let pid = child.id();
    log(&format!("spawned pid={pid}"));
    // We don't need the Child anymore — leak it (DETACHED).
    std::mem::forget(child);

    let mut rect: Option<[i32; 4]> = None;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Some(r) = resolve_target_rect(seat_name, &log) {
            rect = Some(r);
            break;
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    log(&format!("waiting for window from pid={pid} (or descendants)"));
    let hwnd = match win::window::wait_for_process_window(pid, Duration::from_secs(30)) {
        Some(h) => h,
        None => {
            log("timed out waiting for visible window");
            return 0;
        }
    };
    log(&format!("found window hwnd={hwnd}"));

    if rect.is_none() {
        for _ in 0..10 {
            if let Some(r) = resolve_target_rect(seat_name, &log) {
                rect = Some(r);
                break;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    let Some(rect) = rect else {
        log("no rect resolved -- skipping move");
        return 0;
    };
    win::window::move_to_rect(hwnd, rect);
    log(&format!("moved hwnd={hwnd} to rect={rect:?}"));
    0
}

fn resolve_target_rect(seat_name: &str, log: &dyn Fn(&str)) -> Option<[i32; 4]> {
    let state = shared_state::read();
    if let Some(entry) = state.seats.get(seat_name) {
        log(&format!(
            "resolved monitor for '{seat_name}' from per-seat state: {:?}",
            entry.monitor_rect
        ));
        return Some(entry.monitor_rect);
    }
    let baseline: std::collections::HashSet<String> = state.baseline_monitors.into_iter().collect();
    if baseline.is_empty() {
        log("no baseline_monitors in state file -- fleet not running?");
        return None;
    }
    let current = win::monitor::list_active();
    let new_devs: Vec<_> = current.iter().filter(|m| !baseline.contains(&m.device)).collect();
    if new_devs.is_empty() {
        log("no virtual displays detected (current == baseline)");
        return None;
    }
    if new_devs.len() > 1 {
        let names: Vec<&str> = new_devs.iter().map(|m| m.device.as_str()).collect();
        log(&format!("multiple new displays {names:?}; picking first"));
    }
    log(&format!(
        "resolved monitor for '{seat_name}' via baseline-delta: {} {:?}",
        new_devs[0].device, new_devs[0].rect
    ));
    Some(new_devs[0].rect)
}

fn chrono_like_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Minimal "yyyy-mm-dd HH:MM:SS" without pulling chrono — close enough for a launcher log.
    let (year, month, day, hour, min, sec) = epoch_to_civil(secs);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02}")
}

// Howard Hinnant's days-from-civil algorithm, inverted.
fn epoch_to_civil(epoch_secs: u64) -> (i32, u32, u32, u32, u32, u32) {
    let secs_per_day: u64 = 86_400;
    let days = (epoch_secs / secs_per_day) as i64;
    let rem = epoch_secs % secs_per_day;
    let hour = (rem / 3600) as u32;
    let min = ((rem % 3600) / 60) as u32;
    let sec = (rem % 60) as u32;
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { (mp + 3) as u32 } else { (mp - 9) as u32 };
    let year = if m <= 2 { y + 1 } else { y };
    (year as i32, m, d, hour, min, sec)
}
