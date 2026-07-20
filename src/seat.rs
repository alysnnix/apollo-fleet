use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command};

use anyhow::{Context, Result};
use serde_json::json;

use crate::config::SeatCfg;
use crate::win::registry;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
pub const DETACHED_PROCESS: u32 = 0x0000_0008;
pub const NO_WINDOW: u32 = CREATE_NO_WINDOW;

pub struct Seat {
    pub cfg: SeatCfg,
    pub apollo_path: PathBuf,
    pub state_dir: PathBuf,
    pub config_file: PathBuf,
    pub fleet_log: PathBuf,
    pub proc: Option<Child>,
}

impl Seat {
    pub fn new(cfg: SeatCfg, apollo_path: PathBuf, state_root: &std::path::Path) -> Result<Self> {
        let state_dir = state_root.join(&cfg.name);
        fs::create_dir_all(&state_dir)
            .with_context(|| format!("create_dir_all({})", state_dir.display()))?;
        let config_file = state_dir.join("sunshine.conf");
        let fleet_log = state_dir.join("fleet.log");
        let seat = Seat {
            cfg,
            apollo_path,
            state_dir,
            config_file,
            fleet_log,
            proc: None,
        };
        seat.write_config()?;
        seat.write_apps_json(None)?;
        Ok(seat)
    }

    pub fn write_apps_json(&self, fleet_exe: Option<&std::path::Path>) -> Result<()> {
        let apps_path = self.state_dir.join("apps.json");
        let fleet_exe: PathBuf = match fleet_exe {
            Some(p) => p.to_path_buf(),
            None => std::env::current_exe().unwrap_or_else(|_| PathBuf::from("apollo-fleet.exe")),
        };
        let mut apps = Vec::new();
        if let Some(steam) = registry::find_steam_install() {
            apps.push(json!({
                "name": "Steam",
                "image-path": "steam.png",
                "auto-detach": true,
                "wait-all": false,
                "exit-timeout": 5,
                "cmd": format!(
                    "\"{}\" --launch {} -- \"{}\" -gamepadui",
                    fleet_exe.display(),
                    self.cfg.name,
                    steam.display(),
                ),
            }));
        }
        apps.push(json!({"name": "Desktop", "image-path": "desktop.png"}));
        let body = json!({"env": {}, "apps": apps});
        fs::write(&apps_path, serde_json::to_string_pretty(&body)?)?;
        Ok(())
    }

    fn write_config(&self) -> Result<()> {
        let s = &self.state_dir;
        let dd_option = if self.cfg.primary {
            "ensure_primary"
        } else {
            "ensure_active"
        };
        let mut opts: Vec<(String, String)> = vec![
            ("sunshine_name".into(), self.cfg.name.clone()),
            ("port".into(), self.cfg.port.to_string()),
            ("file_apps".into(), s.join("apps.json").display().to_string()),
            ("file_state".into(), s.join("sunshine_state.json").display().to_string()),
            ("log_path".into(), s.join("sunshine.log").display().to_string()),
            ("cert".into(), s.join("cert.pem").display().to_string()),
            ("pkey".into(), s.join("pkey.pem").display().to_string()),
            ("credentials_file".into(), s.join("credentials.json").display().to_string()),
            ("headless_mode".into(), "enabled".into()),
            ("dd_configuration_option".into(), dd_option.into()),
            ("system_tray".into(), "disabled".into()),
        ];
        if !self.cfg.audio_sink.is_empty() {
            opts.push(("audio_sink".into(), self.cfg.audio_sink.clone()));
        }
        if self.cfg.gamepad_only {
            opts.push(("keyboard".into(), "disabled".into()));
            opts.push(("mouse".into(), "disabled".into()));
            opts.push(("controller".into(), "enabled".into()));
        }
        for (k, v) in &self.cfg.extra {
            opts.push((k.clone(), value_to_string(v)));
        }
        let body: String = opts
            .into_iter()
            .map(|(k, v)| format!("{k} = {v}\n"))
            .collect();
        fs::write(&self.config_file, body)?;
        Ok(())
    }

    pub fn start(&mut self) -> Result<u32> {
        let log_file: File = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.fleet_log)
            .with_context(|| format!("open {}", self.fleet_log.display()))?;
        let stderr_clone = log_file.try_clone()?;
        let cwd = self
            .apollo_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        let mut cmd = Command::new(&self.apollo_path);
        cmd.arg(&self.config_file)
            .current_dir(cwd)
            .stdout(log_file)
            .stderr(stderr_clone)
            .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        let child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", self.apollo_path.display()))?;
        let pid = child.id();
        self.proc = Some(child);
        Ok(pid)
    }

    pub fn stop(&mut self) {
        let Some(mut child) = self.proc.take() else { return };
        // Try CTRL_BREAK first (matches Python's behavior; Apollo handles it cleanly).
        let _ = send_ctrl_break(&child);
        match child.wait_timeout(std::time::Duration::from_secs(5)) {
            Ok(true) => return,
            _ => {}
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    pub fn alive(&mut self) -> bool {
        let Some(child) = self.proc.as_mut() else { return false };
        match child.try_wait() {
            Ok(None) => true,
            _ => false,
        }
    }

    pub fn exit_code(&mut self) -> Option<i32> {
        let child = self.proc.as_mut()?;
        match child.try_wait().ok()?? {
            status => status.code(),
        }
    }
}

fn value_to_string(v: &toml::Value) -> String {
    match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(i) => i.to_string(),
        toml::Value::Float(f) => f.to_string(),
        toml::Value::Boolean(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn send_ctrl_break(child: &Child) -> std::io::Result<()> {
    use windows::Win32::System::Console::{GenerateConsoleCtrlEvent, CTRL_BREAK_EVENT};
    unsafe {
        let pid = child.id();
        GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid)
            .map_err(|e| std::io::Error::other(e.to_string()))
    }
}

// Tiny wait_timeout shim — std doesn't expose one but we don't want an extra crate dependency
// just for this. Polls every 100ms until the process exits or the deadline passes.
trait WaitTimeoutExt {
    fn wait_timeout(&mut self, dur: std::time::Duration) -> std::io::Result<bool>;
}

impl WaitTimeoutExt for Child {
    fn wait_timeout(&mut self, dur: std::time::Duration) -> std::io::Result<bool> {
        let deadline = std::time::Instant::now() + dur;
        loop {
            match self.try_wait()? {
                Some(_) => return Ok(true),
                None => {
                    if std::time::Instant::now() >= deadline {
                        return Ok(false);
                    }
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
    }
}

// We use `write!` only in error paths; this keeps the import alive without warnings.
#[allow(dead_code)]
fn _retain_write_import(_: &mut Vec<u8>) {
    let _ = write!(Vec::new(), "");
}
