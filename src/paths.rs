use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub fn appdata_dir() -> PathBuf {
    dirs::config_dir()
        .or_else(dirs::data_dir)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("apollo-fleet")
}

pub fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

pub fn shared_state_path() -> PathBuf {
    appdata_dir().join("fleet-state.json")
}

pub fn resolve_config_path(cli_value: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = cli_value {
        return Ok(p.to_path_buf());
    }
    Ok(appdata_dir().join("seats.toml"))
}

pub fn ensure_user_config(target: &Path, embedded: &str) -> Result<()> {
    if target.exists() {
        return Ok(());
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create_dir_all({})", parent.display()))?;
    }
    fs::write(target, embedded)
        .with_context(|| format!("write {}", target.display()))?;
    Ok(())
}
