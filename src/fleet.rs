use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use parking_lot::Mutex;

use crate::config;
use crate::propagate::{self, PropagatorHandle, SeatRef};
use crate::seat::Seat;
use crate::shared_state::{self, SeatMonitor, SharedState};
use crate::win;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
const RESTART_BACKOFF: Duration = Duration::from_secs(3);
const MAX_FAILURES: usize = 3;
const FAILURE_WINDOW: Duration = Duration::from_secs(30);
const CONNECTED_DEBOUNCE: usize = 2;

pub fn build(config_path: &Path, skip_sink_check: bool) -> Result<Fleet> {
    let cfg = config::load(config_path)?;
    if !cfg.apollo_path.exists() {
        return Err(anyhow::anyhow!(
            "Apollo binary not found: {}",
            cfg.apollo_path.display()
        ));
    }
    if !skip_sink_check {
        for warning in win::audio::validate_sinks(&cfg.seats) {
            log::warn!("[fleet] {warning}");
        }
    }
    std::fs::create_dir_all(&cfg.state_dir)?;
    let mut seats: Vec<Seat> = Vec::with_capacity(cfg.seats.len());
    for s in cfg.seats {
        seats.push(Seat::new(s, cfg.apollo_path.clone(), &cfg.state_dir)?);
    }
    Ok(Fleet::new(seats))
}

pub struct Fleet {
    inner: Arc<Mutex<Inner>>,
    handle: Option<std::thread::JoinHandle<()>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    propagator: Option<PropagatorHandle>,
}

struct Inner {
    seats: Vec<Seat>,
    spawned_idx: Vec<usize>,
    dead: HashSet<String>,
    failures: HashMap<String, Vec<Instant>>,
    last_restart: HashMap<String, Instant>,
    client_history: HashMap<String, Vec<bool>>,
    was_connected: HashMap<String, bool>,
    seat_monitor: HashMap<String, (String, [i32; 4])>,
    known_monitor_names: HashSet<String>,
    initial_monitor_names: HashSet<String>,
}

impl Fleet {
    fn new(seats: Vec<Seat>) -> Self {
        Fleet {
            inner: Arc::new(Mutex::new(Inner {
                seats,
                spawned_idx: Vec::new(),
                dead: HashSet::new(),
                failures: HashMap::new(),
                last_restart: HashMap::new(),
                client_history: HashMap::new(),
                was_connected: HashMap::new(),
                seat_monitor: HashMap::new(),
                known_monitor_names: HashSet::new(),
                initial_monitor_names: HashSet::new(),
            })),
            handle: None,
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            propagator: None,
        }
    }

    pub fn seats(&self) -> Vec<SeatSnapshot> {
        let inner = self.inner.lock();
        inner
            .seats
            .iter()
            .enumerate()
            .map(|(i, s)| SeatSnapshot {
                name: s.cfg.name.clone(),
                port: s.cfg.port,
                state_dir: s.state_dir.clone(),
                config_file: s.config_file.clone(),
                apollo_path: s.apollo_path.clone(),
                spawned: inner.spawned_idx.contains(&i),
                dead: inner.dead.contains(&s.cfg.name),
                connected: connected_now(&inner, &s.cfg.name),
                virtual_monitor: inner.seat_monitor.get(&s.cfg.name).cloned(),
            })
            .collect()
    }

    pub fn seat_count(&self) -> usize {
        self.inner.lock().seats.len()
    }

    pub fn is_running(&self) -> bool {
        self.handle.as_ref().is_some_and(|h| !h.is_finished())
    }

    pub fn client_count(&self) -> usize {
        let inner = self.inner.lock();
        inner
            .spawned_idx
            .iter()
            .filter(|&&i| connected_now(&inner, &inner.seats[i].cfg.name))
            .count()
    }

    pub fn dead_count(&self) -> usize {
        self.inner.lock().dead.len()
    }

    pub fn spawned_count(&self) -> usize {
        self.inner.lock().spawned_idx.len()
    }

    pub fn start(&mut self) {
        if self.is_running() {
            return;
        }
        self.stop.store(false, std::sync::atomic::Ordering::Relaxed);
        let (master_ref, other_refs) = {
            let mut inner = self.inner.lock();
            let baseline: HashSet<String> =
                win::monitor::list_active().into_iter().map(|m| m.device).collect();
            inner.known_monitor_names = baseline.clone();
            inner.initial_monitor_names = baseline;
            inner.seat_monitor.clear();
            let state = build_shared_state(&inner);
            let _ = shared_state::write(&state);
            for idx in 0..inner.seats.len() {
                spawn_index(&mut inner, idx);
            }
            let master = inner.seats.first().map(|s| SeatRef {
                name: s.cfg.name.clone(),
                config_path: s.config_file.clone(),
            });
            let others: Vec<SeatRef> = inner
                .seats
                .iter()
                .skip(1)
                .map(|s| SeatRef {
                    name: s.cfg.name.clone(),
                    config_path: s.config_file.clone(),
                })
                .collect();
            (master, others)
        };
        let inner = self.inner.clone();
        let stop = self.stop.clone();
        self.handle = Some(std::thread::spawn(move || supervise_loop(inner, stop)));

        if let Some(master) = master_ref {
            if !other_refs.is_empty() {
                let restart_target = self.inner.clone();
                self.propagator = Some(propagate::spawn(master, other_refs, move |seat_name| {
                    restart_seat(&restart_target, seat_name);
                }));
            }
        }
    }

    pub fn stop(&mut self) {
        if let Some(p) = self.propagator.take() {
            p.stop();
        }
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
        let mut inner = self.inner.lock();
        let order: Vec<usize> = inner.spawned_idx.iter().rev().copied().collect();
        for idx in order {
            let name = inner.seats[idx].cfg.name.clone();
            inner.seats[idx].stop();
            log::info!("[fleet] stopped '{name}'");
        }
        inner.spawned_idx.clear();
    }

    pub fn run_blocking(&mut self) -> Result<()> {
        self.start();
        // Block until Ctrl-C; std::thread::park is signal-aware on Unix only,
        // but on Windows the console Ctrl handler will terminate the process anyway.
        loop {
            std::thread::sleep(Duration::from_secs(60));
        }
    }

    pub fn virtual_monitor_for(&self, seat_name: &str) -> Option<(String, [i32; 4])> {
        self.inner.lock().seat_monitor.get(seat_name).cloned()
    }
}

#[derive(Debug, Clone)]
pub struct SeatSnapshot {
    pub name: String,
    pub port: u16,
    pub state_dir: std::path::PathBuf,
    pub config_file: std::path::PathBuf,
    pub apollo_path: std::path::PathBuf,
    pub spawned: bool,
    pub dead: bool,
    pub connected: bool,
    pub virtual_monitor: Option<(String, [i32; 4])>,
}

fn connected_now(inner: &Inner, name: &str) -> bool {
    let hist = match inner.client_history.get(name) {
        Some(h) => h,
        None => return false,
    };
    if hist.len() < CONNECTED_DEBOUNCE {
        return false;
    }
    hist.iter().rev().take(CONNECTED_DEBOUNCE).all(|&v| v)
}

fn build_shared_state(inner: &Inner) -> SharedState {
    let mut state = SharedState {
        baseline_monitors: inner.initial_monitor_names.iter().cloned().collect(),
        seats: Default::default(),
    };
    state.baseline_monitors.sort();
    for (name, (dev, rect)) in &inner.seat_monitor {
        state.seats.insert(
            name.clone(),
            SeatMonitor { monitor_device: dev.clone(), monitor_rect: *rect },
        );
    }
    state
}

fn record_failure(inner: &mut Inner, name: &str, now: Instant) -> bool {
    let history = inner.failures.entry(name.to_string()).or_default();
    history.push(now);
    let cutoff = now - FAILURE_WINDOW;
    history.retain(|&t| t >= cutoff);
    history.len() >= MAX_FAILURES
}

fn spawn_index(inner: &mut Inner, idx: usize) {
    if inner.spawned_idx.contains(&idx) {
        return;
    }
    let name = inner.seats[idx].cfg.name.clone();
    if inner.dead.contains(&name) {
        return;
    }
    let port = inner.seats[idx].cfg.port;
    match inner.seats[idx].start() {
        Ok(pid) => {
            inner.spawned_idx.push(idx);
            log::info!("[fleet] started '{name}' port={port} pid={pid}");
            std::thread::spawn(move || {
                if win::tray_icon::hide_for_pid_with_retry(pid, 15, Duration::from_secs(1)) {
                    log::info!("[fleet] hid Apollo tray icon for '{name}'");
                } else {
                    log::info!("[fleet] could not find tray icon to hide for '{name}'");
                }
            });
        }
        Err(e) => {
            log::error!("[fleet] failed to start '{name}': {e:#}");
            inner.dead.insert(name);
        }
    }
}

fn next_unspawned_index(inner: &Inner) -> Option<usize> {
    inner
        .seats
        .iter()
        .enumerate()
        .find(|(i, s)| !inner.spawned_idx.contains(i) && !inner.dead.contains(&s.cfg.name))
        .map(|(i, _)| i)
}

fn supervise_loop(inner: Arc<Mutex<Inner>>, stop: Arc<std::sync::atomic::AtomicBool>) {
    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
        let now = Instant::now();

        // Restart-on-crash pass.
        {
            let mut g = inner.lock();
            let spawned: Vec<usize> = g.spawned_idx.clone();
            for idx in spawned {
                let name = g.seats[idx].cfg.name.clone();
                if g.dead.contains(&name) {
                    continue;
                }
                if g.seats[idx].alive() {
                    continue;
                }
                if let Some(t) = g.last_restart.get(&name) {
                    if now - *t < RESTART_BACKOFF {
                        continue;
                    }
                }
                let code = g.seats[idx].exit_code();
                if record_failure(&mut g, &name, now) {
                    g.dead.insert(name.clone());
                    let log_path = g.seats[idx].state_dir.join("sunshine.log");
                    log::error!(
                        "[fleet] '{name}' exited {}x in {}s (last code={:?}); giving up. Check {}",
                        MAX_FAILURES,
                        FAILURE_WINDOW.as_secs(),
                        code,
                        log_path.display(),
                    );
                    continue;
                }
                g.last_restart.insert(name.clone(), now);
                log::warn!("[fleet] '{name}' exited (code={code:?}); restarting");
                if let Err(e) = g.seats[idx].start() {
                    log::error!("[fleet] restart failed for '{name}': {e:#}");
                }
            }
        }

        // Sample raw connection state, detect transitions, lazy spawn.
        let transitions: Vec<(String, bool, Option<u16>)> = {
            let mut g = inner.lock();
            let alive_idxs: Vec<usize> = {
                let spawned = g.spawned_idx.clone();
                spawned
                    .into_iter()
                    .filter(|&i| {
                        let name = g.seats[i].cfg.name.clone();
                        g.seats[i].alive() && !g.dead.contains(&name)
                    })
                    .collect()
            };

            for &idx in &alive_idxs {
                let port = g.seats[idx].cfg.port;
                let name = g.seats[idx].cfg.name.clone();
                let raw = win::tcp::has_external_client(port).unwrap_or(false);
                let hist = g.client_history.entry(name).or_default();
                hist.push(raw);
                if hist.len() > CONNECTED_DEBOUNCE {
                    hist.remove(0);
                }
            }

            let mut transitions = Vec::new();
            for &idx in &alive_idxs {
                let name = g.seats[idx].cfg.name.clone();
                let connected = connected_now(&g, &name);
                let was = *g.was_connected.get(&name).unwrap_or(&false);
                g.was_connected.insert(name.clone(), connected);
                if connected != was {
                    transitions.push((name, connected, Some(g.seats[idx].cfg.port)));
                }
            }

            let all_busy = !alive_idxs.is_empty()
                && alive_idxs
                    .iter()
                    .all(|&i| connected_now(&g, &g.seats[i].cfg.name));
            if all_busy {
                if let Some(next) = next_unspawned_index(&g) {
                    log::info!(
                        "[fleet] all {} active seat(s) busy; spawning next seat",
                        alive_idxs.len()
                    );
                    spawn_index(&mut g, next);
                }
            }

            transitions
        };

        for (name, connected, _port) in transitions {
            if connected {
                let inner2 = inner.clone();
                let name2 = name.clone();
                std::thread::spawn(move || on_client_connected(inner2, &name2));
            } else {
                let mut g = inner.lock();
                g.seat_monitor.remove(&name);
                let state = build_shared_state(&g);
                drop(g);
                let _ = shared_state::write(&state);
            }
        }

        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Stop and immediately re-start a single seat. Holds the inner lock for the duration so
/// the supervisor thread can't fire its restart-on-crash logic while the seat is briefly
/// down.
fn restart_seat(inner: &Arc<Mutex<Inner>>, seat_name: &str) {
    let mut g = inner.lock();
    let Some(idx) = g.seats.iter().position(|s| s.cfg.name == seat_name) else {
        return;
    };
    if g.dead.contains(seat_name) {
        return;
    }
    g.seats[idx].stop();
    // Clear failure history so this intentional restart doesn't push the seat over
    // the circuit breaker threshold.
    g.failures.remove(seat_name);
    g.last_restart.remove(seat_name);
    if let Err(e) = g.seats[idx].start() {
        log::error!("[propagate] restart of '{seat_name}' failed: {e:#}");
    } else {
        log::info!("[propagate] restarted '{seat_name}' after config propagation");
    }
}

fn on_client_connected(inner: Arc<Mutex<Inner>>, seat_name: &str) {
    // Apollo activates its virtual display ~1-3s after the client handshakes.
    std::thread::sleep(Duration::from_secs(2));
    let current = win::monitor::list_active();

    let (new_dev, new_rect) = {
        let g = inner.lock();
        let claimed: HashSet<String> =
            g.seat_monitor.values().map(|(d, _)| d.clone()).collect();
        let candidate = current.iter().find(|m| {
            !g.known_monitor_names.contains(&m.device) && !claimed.contains(&m.device)
        });
        match candidate {
            Some(m) => (m.device.clone(), m.rect),
            None => {
                log::info!(
                    "[fleet] '{seat_name}' client connected, but no new display detected"
                );
                return;
            }
        }
    };

    {
        let mut g = inner.lock();
        g.seat_monitor
            .insert(seat_name.to_string(), (new_dev.clone(), new_rect));
        g.known_monitor_names.insert(new_dev.clone());
        let state = build_shared_state(&g);
        drop(g);
        let _ = shared_state::write(&state);
    }
    log::info!(
        "[fleet] '{seat_name}' assigned virtual display '{new_dev}' rect={:?}",
        new_rect
    );

    let hwnd = win::window::foreground_window();
    if hwnd == 0 {
        log::info!("[fleet] no foreground window to move for '{seat_name}'");
        return;
    }
    win::window::move_to_rect(hwnd, new_rect);
    log::info!(
        "[fleet] moved foreground window {hwnd} to '{new_dev}' for '{seat_name}'"
    );
}
