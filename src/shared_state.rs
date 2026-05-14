use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::paths;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SharedState {
    #[serde(default)]
    pub baseline_monitors: Vec<String>,
    #[serde(default)]
    pub seats: BTreeMap<String, SeatMonitor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeatMonitor {
    pub monitor_device: String,
    pub monitor_rect: [i32; 4],
}

pub fn read() -> SharedState {
    let p = paths::shared_state_path();
    if !p.exists() {
        return SharedState::default();
    }
    match fs::read_to_string(&p) {
        Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
        Err(_) => SharedState::default(),
    }
}

pub fn write(state: &SharedState) -> std::io::Result<()> {
    let p = paths::shared_state_path();
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string(state)?;
    fs::write(&p, text)
}

pub fn path() -> PathBuf {
    paths::shared_state_path()
}
