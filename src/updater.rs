// GitHub release auto-updater. No new dependencies: uses `curl.exe` (bundled in
// Windows 10 1803+ and 11) for HTTP, serde_json (already a dep) for parsing.
//
// Flow:
//   1. check_latest() hits the GitHub Releases API, returns None if up-to-date
//      or Some(AvailableUpdate) if a newer ApolloFleet.exe asset exists.
//   2. The user clicks "Install update" in the tray. We:
//        a. Download the new exe to `<current_exe>.new`
//        b. Verify the PE header (MZ magic) before trusting it
//        c. Write a small .bat to %TEMP% that waits for our PID, swaps the
//           file, and re-launches
//        d. Spawn the .bat detached, then exit our own process

use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{anyhow, Context, Result};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const RELEASES_URL: &str =
    "https://api.github.com/repos/alysnnix/apollo-fleet/releases/latest";
const ASSET_NAME: &str = "ApolloFleet.exe";

#[derive(Debug, Clone)]
pub struct AvailableUpdate {
    pub version: String,
    pub asset_url: String,
}

/// Returns `Some(update)` if GitHub's latest release tag is newer than the running
/// binary's CARGO_PKG_VERSION and ships an `ApolloFleet.exe` asset. Returns `Ok(None)`
/// if up-to-date. Network failures bubble up so the caller can decide whether to
/// surface them or stay quiet.
pub fn check_latest() -> Result<Option<AvailableUpdate>> {
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "10",
            "-H",
            "Accept: application/vnd.github+json",
            "-A",
            user_agent().as_str(),
            RELEASES_URL,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("spawn curl")?;
    if !output.status.success() {
        return Err(anyhow!(
            "curl exited {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).context("parse releases JSON")?;

    let tag = json
        .get("tag_name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow!("no tag_name in release"))?;

    let current = env!("CARGO_PKG_VERSION");
    if !is_newer(tag, current) {
        return Ok(None);
    }

    let asset_url = json
        .get("assets")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter().find(|a| {
                a.get("name").and_then(|n| n.as_str()) == Some(ASSET_NAME)
            })
        })
        .and_then(|a| a.get("browser_download_url"))
        .and_then(|u| u.as_str())
        .ok_or_else(|| anyhow!("release {tag} has no {ASSET_NAME} asset"))?;

    Ok(Some(AvailableUpdate {
        version: tag.trim_start_matches('v').to_string(),
        asset_url: asset_url.to_string(),
    }))
}

/// Download the asset to `<current_exe>.new` and validate it's a real PE.
/// Returns the path of the staged new exe on success.
pub fn download_and_stage(update: &AvailableUpdate) -> Result<PathBuf> {
    let current_exe = std::env::current_exe().context("current_exe")?;
    let mut new_exe = current_exe.clone();
    new_exe.set_extension("exe.new");

    let output = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "120",
            "-A",
            user_agent().as_str(),
            "-o",
            new_exe.to_str().ok_or_else(|| anyhow!("bad path"))?,
            update.asset_url.as_str(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .context("spawn curl download")?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&new_exe);
        return Err(anyhow!(
            "download failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    // Sanity: every Win32 PE starts with "MZ".
    let bytes = std::fs::read(&new_exe).context("read staged exe")?;
    if bytes.len() < 64 || &bytes[..2] != b"MZ" {
        let _ = std::fs::remove_file(&new_exe);
        return Err(anyhow!("downloaded file is not a valid PE executable"));
    }
    Ok(new_exe)
}

/// Write a batch file that waits for our PID to exit, swaps `staged` over the
/// current exe, and launches the new one. Spawn it with CREATE_NO_WINDOW (a
/// hidden console; `timeout` and console-attached helpers need a real console,
/// so DETACHED_PROCESS is avoided) and return Ok. The caller is expected to
/// clean-stop the fleet and exit shortly afterward.
///
/// All batch output is captured to `%TEMP%\apollo-fleet-update.log` so failed
/// swaps are diagnosable.
pub fn launch_apply(staged: &Path) -> Result<()> {
    let current_exe = std::env::current_exe().context("current_exe")?;
    let pid = std::process::id();
    let batch_path = std::env::temp_dir().join("apollo-fleet-update.bat");
    let log_path = std::env::temp_dir().join("apollo-fleet-update.log");

    let script = format!(
        "@echo off\r\n\
         setlocal\r\n\
         set \"LOG={log}\"\r\n\
         set \"STAGED={staged}\"\r\n\
         set \"TARGET={current}\"\r\n\
         set \"PARENT_PID={pid}\"\r\n\
         echo [%date% %time%] updater start pid=%PARENT_PID% > \"%LOG%\"\r\n\
         echo [%date% %time%] staged=%STAGED% >> \"%LOG%\"\r\n\
         echo [%date% %time%] target=%TARGET% >> \"%LOG%\"\r\n\
         set /a waits=0\r\n\
         :wait_parent\r\n\
         tasklist /FI \"PID eq %PARENT_PID%\" /NH 2>nul | find \"%PARENT_PID%\" >nul\r\n\
         if errorlevel 1 goto parent_gone\r\n\
         set /a waits+=1\r\n\
         if %waits% GEQ 60 (\r\n\
             echo [%date% %time%] parent still alive after %waits% checks, proceeding >> \"%LOG%\"\r\n\
             goto parent_gone\r\n\
         )\r\n\
         ping -n 2 127.0.0.1 >nul\r\n\
         goto wait_parent\r\n\
         :parent_gone\r\n\
         echo [%date% %time%] parent exited after %waits% cycles >> \"%LOG%\"\r\n\
         ping -n 2 127.0.0.1 >nul\r\n\
         set /a tries=0\r\n\
         :try_move\r\n\
         set /a tries+=1\r\n\
         echo [%date% %time%] move attempt %tries% >> \"%LOG%\"\r\n\
         move /Y \"%STAGED%\" \"%TARGET%\" >> \"%LOG%\" 2>&1\r\n\
         if not errorlevel 1 goto move_ok\r\n\
         if %tries% LSS 15 (\r\n\
             ping -n 3 127.0.0.1 >nul\r\n\
             goto try_move\r\n\
         )\r\n\
         echo [%date% %time%] move failed after %tries% attempts, giving up >> \"%LOG%\"\r\n\
         exit /b 1\r\n\
         :move_ok\r\n\
         echo [%date% %time%] move succeeded on attempt %tries% >> \"%LOG%\"\r\n\
         start \"\" \"%TARGET%\"\r\n\
         echo [%date% %time%] new process launched >> \"%LOG%\"\r\n\
         del \"%~f0\"\r\n",
        log = log_path.display(),
        pid = pid,
        staged = staged.display(),
        current = current_exe.display(),
    );
    std::fs::write(&batch_path, script).context("write update batch")?;

    Command::new("cmd")
        .args([
            "/C",
            batch_path.to_str().ok_or_else(|| anyhow!("bad path"))?,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .context("spawn update batch")?;
    Ok(())
}

fn user_agent() -> String {
    format!("apollo-fleet/{}", env!("CARGO_PKG_VERSION"))
}

fn is_newer(latest: &str, current: &str) -> bool {
    let Some(l) = parse_version(latest) else {
        return false;
    };
    let Some(c) = parse_version(current) else {
        return false;
    };
    l > c
}

fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim().trim_start_matches('v');
    let mut parts = s.split('.');
    let major = parts.next()?.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?;
    let minor = parts.next()?.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?;
    let patch_str = parts.next()?;
    let patch: u32 = patch_str.chars().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_tags() {
        assert_eq!(parse_version("v0.4.2"), Some((0, 4, 2)));
        assert_eq!(parse_version("0.4.2"), Some((0, 4, 2)));
        assert_eq!(parse_version("v1.10.3"), Some((1, 10, 3)));
        assert_eq!(parse_version("v0.4.2-rc1"), Some((0, 4, 2)));
    }

    #[test]
    fn newer_detection() {
        assert!(is_newer("v0.5.0", "0.4.2"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(!is_newer("v0.4.2", "0.4.2"));
        assert!(!is_newer("v0.4.1", "0.4.2"));
        assert!(!is_newer("bogus", "0.4.2"));
    }
}
