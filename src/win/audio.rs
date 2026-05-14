use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::Duration;

use anyhow::Result;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Active render endpoint friendly names. Shells out to PowerShell — the COM path
/// (IMMDeviceEnumerator + PROPVARIANT extraction) is brittle across windows-rs
/// versions and this list is queried rarely, so the subprocess cost is irrelevant.
pub fn list_audio_endpoints() -> Result<Vec<String>> {
    let script =
        "Get-PnpDevice -Class AudioEndpoint -Status OK -ErrorAction SilentlyContinue \
         | ForEach-Object { $_.FriendlyName }";
    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()?;
    let _ = Duration::from_secs(10); // keep this expressive: PS is the slow path
    let text = String::from_utf8_lossy(&output.stdout);
    Ok(text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

pub fn validate_sinks(seats: &[crate::config::SeatCfg]) -> Vec<String> {
    let needed: Vec<(&str, &str)> = seats
        .iter()
        .filter(|s| !s.audio_sink.is_empty())
        .map(|s| (s.name.as_str(), s.audio_sink.as_str()))
        .collect();
    if needed.is_empty() {
        return Vec::new();
    }
    let endpoints = match list_audio_endpoints() {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => return vec!["could not enumerate audio endpoints (empty list)".to_string()],
        Err(e) => return vec![format!("could not enumerate audio endpoints: {e:#}")],
    };
    let lower: Vec<String> = endpoints.iter().map(|e| e.to_lowercase()).collect();
    let mut errors = Vec::new();
    for (name, sink) in needed {
        let needle = sink.to_lowercase();
        if !lower.iter().any(|e| e.contains(&needle)) {
            errors.push(format!(
                "seat '{name}' wants audio_sink '{sink}' -- not found among active endpoints"
            ));
        }
    }
    errors
}
