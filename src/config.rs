use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct RawConfig {
    pub apollo_path: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    #[serde(default, rename = "seat")]
    pub seats: Vec<RawSeat>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct RawSeat {
    pub name: String,
    pub port: u16,
    #[serde(default)]
    pub audio_sink: String,
    #[serde(default)]
    pub gamepad_only: bool,
    #[serde(default)]
    pub primary: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone)]
pub struct SeatCfg {
    pub name: String,
    pub port: u16,
    pub audio_sink: String,
    pub gamepad_only: bool,
    pub primary: bool,
    pub extra: BTreeMap<String, toml::Value>,
}

#[derive(Debug, Clone)]
pub struct LoadedConfig {
    pub apollo_path: PathBuf,
    pub state_dir: PathBuf,
    pub seats: Vec<SeatCfg>,
}

pub fn load(path: &Path) -> Result<LoadedConfig> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("read {}", path.display()))?;
    let raw: RawConfig = toml::from_str(&text)
        .with_context(|| format!("parse {}", path.display()))?;

    if raw.seats.is_empty() {
        return Err(anyhow!("config has no [[seat]] entries"));
    }

    let apollo_path = raw
        .apollo_path
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files\Apollo\sunshine.exe"));
    let state_dir = raw.state_dir.unwrap_or_else(|| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".apollo-fleet")
    });

    let known: HashSet<&str> =
        ["name", "port", "audio_sink", "gamepad_only", "primary"].into_iter().collect();
    let mut seen_names: HashSet<String> = HashSet::new();
    let mut seen_ports: HashSet<u16> = HashSet::new();
    let mut seats = Vec::with_capacity(raw.seats.len());

    for s in raw.seats {
        if !seen_names.insert(s.name.clone()) {
            return Err(anyhow!("duplicate seat name: {}", s.name));
        }
        if !seen_ports.insert(s.port) {
            return Err(anyhow!("duplicate seat port: {}", s.port));
        }
        let extra: BTreeMap<String, toml::Value> = s
            .extra
            .into_iter()
            .filter(|(k, _)| !known.contains(k.as_str()))
            .collect();
        seats.push(SeatCfg {
            name: s.name,
            port: s.port,
            audio_sink: s.audio_sink,
            gamepad_only: s.gamepad_only,
            primary: s.primary,
            extra,
        });
    }

    let mut ports: Vec<u16> = seats.iter().map(|s| s.port).collect();
    ports.sort_unstable();
    for w in ports.windows(2) {
        if w[1] - w[0] < 40 {
            eprintln!(
                "[fleet] WARNING: seats at port {} and {} are <40 apart; ranges may collide",
                w[0], w[1]
            );
        }
    }

    Ok(LoadedConfig { apollo_path, state_dir, seats })
}
