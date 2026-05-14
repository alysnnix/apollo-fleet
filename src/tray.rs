use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::{Context, Result};
use parking_lot::Mutex;
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};
use winit::application::ApplicationHandler;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};

use crate::fleet::{self, Fleet};
use crate::paths;
use crate::win;

struct TrayApp {
    config_path: PathBuf,
    skip_sink_check: bool,
    fleet: Arc<Mutex<Option<Fleet>>>,
    last_error: Arc<Mutex<Option<String>>>,
    tray: Option<TrayIcon>,
    menu_ids: MenuIds,
    proxy: EventLoopProxy<UserEvent>,
    pending_events: Arc<Mutex<Vec<MenuEvent>>>,
}

#[derive(Default, Clone)]
struct MenuIds {
    start: String,
    stop: String,
    restart: String,
    set_creds: String,
    edit_config: String,
    install_vbcable: String,
    install_voicemeeter: String,
    show_diag: String,
    open_state: String,
    quit: String,
    seats: Vec<SeatMenuIds>,
}

#[derive(Clone)]
struct SeatMenuIds {
    name: String,
    open_web: String,
    move_foreground: String,
    open_logs: String,
}

#[derive(Debug)]
enum UserEvent {
    Refresh,
}

pub fn run(config_path: PathBuf, skip_sink_check: bool) -> Result<()> {
    let event_loop: EventLoop<UserEvent> = EventLoop::with_user_event().build()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    let proxy = event_loop.create_proxy();
    let pending_events: Arc<Mutex<Vec<MenuEvent>>> = Arc::new(Mutex::new(Vec::new()));

    // Capture menu clicks: push into our own queue AND wake winit. Setting a custom
    // handler bypasses tray-icon's internal channel, so we must store events ourselves.
    let menu_proxy = proxy.clone();
    let menu_queue = pending_events.clone();
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        menu_queue.lock().push(event);
        let _ = menu_proxy.send_event(UserEvent::Refresh);
    }));

    // Background refresh (status / menu rebuild) every 3 seconds.
    let refresh_proxy = proxy.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(3));
        if refresh_proxy.send_event(UserEvent::Refresh).is_err() {
            break;
        }
    });

    let mut app = TrayApp {
        config_path,
        skip_sink_check,
        fleet: Arc::new(Mutex::new(None)),
        last_error: Arc::new(Mutex::new(None)),
        tray: None,
        menu_ids: MenuIds::default(),
        proxy,
        pending_events,
    };

    // Eager-start the fleet so the user doesn't have to click Start manually.
    app.start_fleet();

    event_loop.run_app(&mut app)?;
    Ok(())
}

impl ApplicationHandler<UserEvent> for TrayApp {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {
        if self.tray.is_none() {
            if let Err(e) = self.build_tray() {
                log::error!("could not build tray icon: {e:#}");
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Refresh => {
                let needs_rebuild = self.dispatch_pending_menu_events();
                self.refresh_tray();
                if needs_rebuild {
                    self.rebuild_menu();
                }
                if self.should_exit() {
                    event_loop.exit();
                }
            }
        }
    }

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {}
}

impl TrayApp {
    fn build_tray(&mut self) -> Result<()> {
        let (menu, ids) = self.build_menu();
        self.menu_ids = ids;
        let icon = make_icon(self.is_running());
        let title = self.status_label();
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(format!("Apollo Fleet — {title}"))
            .with_icon(icon)
            .build()
            .context("build tray icon")?;
        self.tray = Some(tray);
        Ok(())
    }

    /// Periodic refresh: icon + tooltip only. Rebuilding the menu while it's open
    /// would dismiss any visible submenu (Windows tears down popups when SetMenu fires),
    /// so menu rebuilds are confined to `rebuild_menu`, which is only called after the
    /// user performs an action that changed state.
    fn refresh_tray(&mut self) {
        let Some(tray) = self.tray.as_ref() else { return };
        let icon = make_icon(self.is_running());
        let _ = tray.set_icon(Some(icon));
        let title = self.status_label();
        let _ = tray.set_tooltip(Some(format!("Apollo Fleet — {title}")));
    }

    fn rebuild_menu(&mut self) {
        let Some(tray) = self.tray.as_ref() else { return };
        let (menu, ids) = self.build_menu();
        self.menu_ids = ids;
        let _ = tray.set_menu(Some(Box::new(menu)));
    }

    fn dispatch_pending_menu_events(&mut self) -> bool {
        let events: Vec<MenuEvent> = self.pending_events.lock().drain(..).collect();
        let mut needs_rebuild = false;
        for event in events {
            if self.handle_menu_event(event) {
                needs_rebuild = true;
            }
        }
        needs_rebuild
    }

    /// Returns true when the click changed state in a way that requires a menu rebuild
    /// (Start/Stop/Restart/Quit). Other clicks (open browser, edit file, etc.) don't
    /// affect the menu's labels.
    fn handle_menu_event(&mut self, event: MenuEvent) -> bool {
        let id = event.id.0.as_str();
        if id == self.menu_ids.start {
            self.start_fleet();
            true
        } else if id == self.menu_ids.stop {
            self.stop_fleet();
            true
        } else if id == self.menu_ids.restart {
            self.restart_fleet();
            true
        } else if id == self.menu_ids.set_creds {
            self.set_master_credentials();
            false
        } else if id == self.menu_ids.edit_config {
            open_in_editor(&self.config_path);
            false
        } else if id == self.menu_ids.install_vbcable {
            let _ = webbrowser::open("https://vb-audio.com/Cable/");
            self.notify("VB-Cable installer", "Install, reboot, restart Apollo Fleet.");
            false
        } else if id == self.menu_ids.install_voicemeeter {
            let _ = webbrowser::open("https://vb-audio.com/Voicemeeter/potato.htm");
            self.notify("Voicemeeter Potato installer", "Install, reboot, restart Apollo Fleet.");
            false
        } else if id == self.menu_ids.show_diag {
            self.show_diagnostics();
            false
        } else if id == self.menu_ids.open_state {
            self.open_state_dir();
            false
        } else if id == self.menu_ids.quit {
            self.stop_fleet();
            self.tray = None;
            self.last_error.lock().replace("__quit__".into());
            false
        } else {
            // Seat-scoped items.
            for seat in &self.menu_ids.seats.clone() {
                if id == seat.open_web {
                    if let Some(port) = self.seat_port(&seat.name) {
                        let _ = webbrowser::open(&format!("https://localhost:{}/", port + 1));
                    }
                } else if id == seat.move_foreground {
                    self.move_foreground_to_seat(&seat.name);
                } else if id == seat.open_logs {
                    if let Some(dir) = self.seat_state_dir(&seat.name) {
                        open_in_explorer(&dir);
                    }
                }
            }
            false
        }
    }

    fn should_exit(&self) -> bool {
        matches!(self.last_error.lock().as_deref(), Some("__quit__"))
    }

    fn is_running(&self) -> bool {
        let g = self.fleet.lock();
        g.as_ref().is_some_and(|f| f.is_running())
    }

    fn status_label(&self) -> String {
        if matches!(self.last_error.lock().as_deref(), Some(s) if s != "__quit__") {
            return "config error".to_string();
        }
        let g = self.fleet.lock();
        let Some(f) = g.as_ref() else {
            return "stopped".to_string();
        };
        if !f.is_running() {
            return "stopped".to_string();
        }
        let total = f.seat_count();
        let spawned = f.spawned_count();
        let connected = f.client_count();
        let dead = f.dead_count();
        let mut s = format!("running: {connected} connected / {spawned}/{total} active");
        if dead > 0 {
            s += &format!(" ({dead} dead)");
        }
        s
    }

    fn build_menu(&self) -> (Menu, MenuIds) {
        let menu = Menu::new();
        let mut ids = MenuIds::default();

        let status = MenuItem::new(format!("Status: {}", self.status_label()), false, None);
        let _ = menu.append(&status);
        let _ = menu.append(&PredefinedMenuItem::separator());

        let running = self.is_running();
        let start = MenuItem::new("Start", !running, None);
        let stop = MenuItem::new("Stop", running, None);
        let restart = MenuItem::new("Restart", running, None);
        ids.start = start.id().0.clone();
        ids.stop = stop.id().0.clone();
        ids.restart = restart.id().0.clone();
        let _ = menu.append(&start);
        let _ = menu.append(&stop);
        let _ = menu.append(&restart);
        let _ = menu.append(&PredefinedMenuItem::separator());

        // Seats submenu
        let seats_submenu = Submenu::new("Seats", true);
        let snapshot = {
            let g = self.fleet.lock();
            g.as_ref().map(|f| f.seats()).unwrap_or_default()
        };
        if snapshot.is_empty() {
            let placeholder = MenuItem::new("(no seats loaded yet)", false, None);
            let _ = seats_submenu.append(&placeholder);
        } else {
            for (i, seat) in snapshot.iter().enumerate() {
                let suffix = if seat.connected {
                    "— connected"
                } else if seat.spawned {
                    "— idle"
                } else if seat.dead {
                    "— dead"
                } else {
                    "— pending"
                };
                let role = if i == 0 { " [master]" } else { "" };
                let label = format!("{}{role} (port {}) {suffix}", seat.name, seat.port);
                let inner = Submenu::new(label, true);
                let open_web = MenuItem::new("Open web UI", seat.spawned, None);
                let move_fg = MenuItem::new(
                    "Move foreground game here",
                    seat.connected,
                    None,
                );
                let open_logs = MenuItem::new("Open logs folder", true, None);
                let smids = SeatMenuIds {
                    name: seat.name.clone(),
                    open_web: open_web.id().0.clone(),
                    move_foreground: move_fg.id().0.clone(),
                    open_logs: open_logs.id().0.clone(),
                };
                let _ = inner.append(&open_web);
                let _ = inner.append(&move_fg);
                let _ = inner.append(&open_logs);
                let _ = seats_submenu.append(&inner);
                ids.seats.push(smids);
            }
        }
        let _ = menu.append(&seats_submenu);

        let set_creds = MenuItem::new("Set master credentials...", running, None);
        ids.set_creds = set_creds.id().0.clone();
        let _ = menu.append(&set_creds);

        let edit_config = MenuItem::new("Edit seats.toml", true, None);
        ids.edit_config = edit_config.id().0.clone();
        let _ = menu.append(&edit_config);

        let audio_submenu = Submenu::new("Install audio drivers", true);
        let vbcable = MenuItem::new("VB-Cable (free, +1 sink)", true, None);
        let voicemeeter = MenuItem::new("Voicemeeter Potato (free, +3 sinks)", true, None);
        ids.install_vbcable = vbcable.id().0.clone();
        ids.install_voicemeeter = voicemeeter.id().0.clone();
        let _ = audio_submenu.append(&vbcable);
        let _ = audio_submenu.append(&voicemeeter);
        let _ = menu.append(&audio_submenu);

        let show_diag = MenuItem::new("Show audio diagnostics", true, None);
        ids.show_diag = show_diag.id().0.clone();
        let _ = menu.append(&show_diag);

        let open_state = MenuItem::new("Open state folder", true, None);
        ids.open_state = open_state.id().0.clone();
        let _ = menu.append(&open_state);

        let _ = menu.append(&PredefinedMenuItem::separator());

        let quit = MenuItem::new("Quit", true, None);
        ids.quit = quit.id().0.clone();
        let _ = menu.append(&quit);

        (menu, ids)
    }

    fn start_fleet(&mut self) {
        let mut guard = self.fleet.lock();
        if guard.is_none() {
            match fleet::build(&self.config_path, self.skip_sink_check) {
                Ok(f) => {
                    self.last_error.lock().take();
                    *guard = Some(f);
                }
                Err(e) => {
                    let msg = format!("{e:#}");
                    self.last_error.lock().replace(msg.clone());
                    drop(guard);
                    self.notify("Fleet config error", &msg);
                    return;
                }
            }
        }
        if let Some(f) = guard.as_mut() {
            if !f.is_running() {
                let count = f.seat_count();
                f.start();
                drop(guard);
                self.notify("Apollo Fleet", &format!("Started {count} seat(s)"));
            }
        }
    }

    fn stop_fleet(&mut self) {
        let mut guard = self.fleet.lock();
        if let Some(f) = guard.as_mut() {
            if f.is_running() {
                f.stop();
                drop(guard);
                self.notify("Apollo Fleet", "Stopped");
            }
        }
    }

    fn restart_fleet(&mut self) {
        self.stop_fleet();
        self.fleet.lock().take();
        self.start_fleet();
    }

    fn seat_port(&self, name: &str) -> Option<u16> {
        let g = self.fleet.lock();
        let f = g.as_ref()?;
        f.seats().into_iter().find(|s| s.name == name).map(|s| s.port)
    }

    fn seat_state_dir(&self, name: &str) -> Option<std::path::PathBuf> {
        let g = self.fleet.lock();
        let f = g.as_ref()?;
        f.seats().into_iter().find(|s| s.name == name).map(|s| s.state_dir)
    }

    fn move_foreground_to_seat(&self, name: &str) {
        let g = self.fleet.lock();
        let Some(f) = g.as_ref() else { return };
        let Some((dev, rect)) = f.virtual_monitor_for(name) else {
            drop(g);
            self.notify(
                "Move failed",
                &format!("No virtual display tracked for '{name}'. Connect a client first."),
            );
            return;
        };
        let hwnd = win::window::foreground_window();
        if hwnd == 0 {
            self.notify("Move failed", "No foreground window.");
            return;
        }
        win::window::move_to_rect(hwnd, rect);
        self.notify("Window moved", &format!("Moved to '{dev}'."));
    }

    fn set_master_credentials(&self) {
        let fleet = self.fleet.clone();
        std::thread::spawn(move || {
            let user = match rfd_input("Apollo Fleet — Set credentials", "Username (applies to all seats):") {
                Some(u) if !u.is_empty() => u,
                _ => return,
            };
            let pw = match rfd_input("Apollo Fleet — Set credentials", "Password:") {
                Some(p) if !p.is_empty() => p,
                _ => return,
            };

            let snapshots = {
                let g = fleet.lock();
                g.as_ref().map(|f| f.seats()).unwrap_or_default()
            };
            if snapshots.is_empty() {
                return;
            }

            let was_running = {
                let mut g = fleet.lock();
                match g.as_mut() {
                    Some(f) if f.is_running() => {
                        f.stop();
                        true
                    }
                    _ => false,
                }
            };

            let mut failed: Vec<(String, String)> = Vec::new();
            for snap in &snapshots {
                use std::os::windows::process::CommandExt;
                use std::process::Command;
                let mut cmd = Command::new(&snap.apollo_path);
                cmd.arg(&snap.config_file)
                    .arg("--creds")
                    .arg(&user)
                    .arg(&pw)
                    .creation_flags(0x0800_0000);
                if let Some(parent) = snap.apollo_path.parent() {
                    cmd.current_dir(parent);
                }
                match cmd.output() {
                    Ok(out) if out.status.success() => {}
                    Ok(out) => {
                        let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
                        failed.push((
                            snap.name.clone(),
                            if msg.is_empty() {
                                format!("exit {:?}", out.status.code())
                            } else {
                                msg
                            },
                        ));
                    }
                    Err(e) => failed.push((snap.name.clone(), e.to_string())),
                }
            }

            if was_running {
                let mut g = fleet.lock();
                if let Some(f) = g.as_mut() {
                    f.start();
                }
            }

            if failed.is_empty() {
                rfd::MessageDialog::new()
                    .set_title("Apollo Fleet")
                    .set_description(format!(
                        "Credentials applied to {} seat(s).",
                        snapshots.len()
                    ))
                    .show();
            } else {
                let detail = failed
                    .iter()
                    .map(|(n, m)| format!("  - {n}: {m}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("Apollo Fleet")
                    .set_description(format!(
                        "Failed to set credentials on {} seat(s):\n{detail}",
                        failed.len()
                    ))
                    .show();
            }
        });
    }

    fn open_state_dir(&self) {
        let snapshot = {
            let g = self.fleet.lock();
            g.as_ref().and_then(|f| f.seats().into_iter().next())
        };
        let dir = match snapshot {
            Some(s) => s.state_dir.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| paths::appdata_dir()),
            None => dirs::home_dir().unwrap_or_default().join(".apollo-fleet"),
        };
        open_in_explorer(&dir);
    }

    fn show_diagnostics(&self) {
        let endpoints = win::audio::list_audio_endpoints().unwrap_or_default();
        let out = paths::temp_dir().join("apollo-fleet-diag.txt");
        let mut body = String::new();
        body.push_str("Apollo Fleet — diagnostics\n");
        body.push_str(&"=".repeat(40));
        body.push('\n');
        body.push_str(&format!("Config:       {}\n", self.config_path.display()));
        let last_err = self.last_error.lock();
        body.push_str(&format!(
            "Last error:   {}\n\n",
            last_err.as_deref().unwrap_or("(none)")
        ));
        drop(last_err);
        body.push_str("Active Windows audio endpoints (use any of these as audio_sink):\n");
        if endpoints.is_empty() {
            body.push_str("  (none detected)\n");
        } else {
            for n in &endpoints {
                body.push_str(&format!("  - {n}\n"));
            }
        }
        body.push_str(
            "\nIf you don't see a 'virtual' sink (Voicemeeter / VB-Cable / Steam Streaming Speakers),\n\
             install one of these to enable per-seat audio:\n\
             \n\
             - Voicemeeter Potato: https://vb-audio.com/Voicemeeter/potato.htm\n\
             - VB-Cable:           https://vb-audio.com/Cable/\n",
        );
        if let Err(e) = std::fs::write(&out, body) {
            self.notify("Diagnostics", &format!("could not write file: {e}"));
            return;
        }
        let _ = open::that(&out);
    }

    fn notify(&self, _title: &str, _body: &str) {
        // tray-icon doesn't expose toast notifications directly. Keep this as a no-op for now;
        // user feedback is conveyed via the icon tooltip and dialog boxes.
    }
}

const APOLLO_SVG: &str = include_str!("../resources/apollo-icon.svg");

fn rasterized_icon_rgba() -> &'static (Vec<u8>, u32) {
    static CACHE: OnceLock<(Vec<u8>, u32)> = OnceLock::new();
    CACHE.get_or_init(|| {
        let size: u32 = 32;
        let tree = resvg::usvg::Tree::from_str(APOLLO_SVG, &resvg::usvg::Options::default())
            .expect("parse apollo-icon.svg");
        let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).expect("alloc pixmap");
        let tree_size = tree.size();
        let transform = resvg::tiny_skia::Transform::from_scale(
            size as f32 / tree_size.width(),
            size as f32 / tree_size.height(),
        );
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        (pixmap.take(), size)
    })
}

fn make_icon(_running: bool) -> Icon {
    // Icon is the apollo logo; status is conveyed in the tooltip. Status-tinting
    // would distort the brand mark, and the tray icon size (16-32px) is too small
    // for a meaningful color signal next to artwork.
    let (rgba, size) = rasterized_icon_rgba();
    Icon::from_rgba(rgba.clone(), *size, *size).expect("build icon")
}

fn open_in_editor(path: &std::path::Path) {
    let _ = open::that(path);
}

fn open_in_explorer(path: &std::path::Path) {
    if !path.exists() {
        return;
    }
    let _ = std::process::Command::new("explorer")
        .arg(path)
        .spawn();
}

// Small text-input dialog. rfd doesn't provide one natively, so we shell out to PowerShell
// which has VB.InputBox built in. Returns None on cancel.
fn rfd_input(title: &str, prompt: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    let script = format!(
        "Add-Type -AssemblyName Microsoft.VisualBasic; \
         [Microsoft.VisualBasic.Interaction]::InputBox(\"{}\", \"{}\")",
        prompt.replace('"', "''"),
        title.replace('"', "''"),
    );
    let out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

// `open` crate is small but it's another dep — wrap it locally in case we want to remove.
mod open {
    pub fn that<P: AsRef<std::path::Path>>(path: P) -> std::io::Result<()> {
        use std::os::windows::process::CommandExt;
        use std::process::Command;
        Command::new("cmd")
            .args(["/C", "start", ""])
            .arg(path.as_ref().as_os_str())
            .creation_flags(0x0800_0000)
            .spawn()
            .map(|_| ())
    }
}
