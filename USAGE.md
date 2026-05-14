# Apollo Fleet — Usage Guide

Step-by-step setup and operation. For an overview, see [README.md](README.md). For security reports, see [SECURITY.md](SECURITY.md).

## Table of contents

- [What it does](#what-it-does)
- [Requirements](#requirements)
- [Installation](#installation)
- [First run](#first-run)
- [Configuring seats](#configuring-seats)
- [Audio sinks](#audio-sinks)
- [Running with Administrator privileges](#running-with-administrator-privileges)
- [Auto-start on login](#auto-start-on-login)
- [Tray menu reference](#tray-menu-reference)
- [Master config propagation](#master-config-propagation)
- [Per-seat credentials](#per-seat-credentials)
- [Pairing Moonlight clients](#pairing-moonlight-clients)
- [Logs and diagnostics](#logs-and-diagnostics)
- [Troubleshooting](#troubleshooting)
- [Uninstall](#uninstall)
- [Building from source](#building-from-source)

## What it does

Apollo Fleet runs N isolated Apollo (Sunshine) instances on a single Windows host so multiple Moonlight clients can connect as if they were talking to separate machines. Each "seat" has its own port, certificate, virtual display, audio sink, and state directory. The fleet:

- Starts the first seat eagerly so a Moonlight client can find the host on the LAN.
- Spawns the next seat only when every running seat already has a client connected. The fleet always has exactly one idle seat ready for the next device, never more.
- Restarts a seat if Apollo crashes; gives up after 3 exits in 30 seconds (circuit breaker).
- Hides Apollo's own tray icon (one tray icon per fleet, not per seat).
- Auto-moves the host's foreground window onto a seat's virtual display when a client connects.
- Propagates master-seat config changes to the rest of the fleet (see [Master config propagation](#master-config-propagation)).

## Requirements

- **Windows 10 build 1903 or later, or Windows 11.** Older builds lack APIs Apollo Fleet uses.
- **Apollo (Sunshine fork) installed.** Default path: `C:\Program Files\Apollo\sunshine.exe`. Download from [github.com/ClassicOldSong/Apollo](https://github.com/ClassicOldSong/Apollo).
- **Administrator account** for running Apollo Fleet (Apollo needs admin to create virtual displays).
- **Optional: virtual audio sinks** if you want per-seat audio. See [Audio sinks](#audio-sinks).
- **GPU** with NVENC, Quick Sync, or AMF (any encoder Apollo supports).

## Installation

1. Download the latest `ApolloFleet.exe` from [Releases](https://github.com/alysnnix/apollo-fleet/releases/latest).
2. Place it anywhere you like, for example `C:\Tools\ApolloFleet\ApolloFleet.exe`.
3. There is no installer — the binary is fully self-contained. No DLLs, no `.NET`, no Python runtime needed.

### Optional: pin to Start

Right-click `ApolloFleet.exe` → **Pin to Start** or **Pin to taskbar** for one-click launch.

## First run

1. Right-click `ApolloFleet.exe` → **Run as administrator**.
2. The Apollo Fleet icon appears in the system tray (notification area). The icon adapts to your Windows light/dark theme.
3. On the first run, Apollo Fleet creates:
    - `%APPDATA%\apollo-fleet\seats.toml` — your editable config, seeded from a template.
    - `%APPDATA%\apollo-fleet\fleet-state.json` — runtime state, do not edit.
4. The fleet starts the first seat automatically. The tray tooltip shows `Apollo Fleet — running: 0 connected / 1/N active` once it's up.
5. Right-click the tray icon to see the full menu.

If you see `Status: config error` in the tooltip, click the tray, choose **Show audio diagnostics**, and check the file it opens for the actual error.

## Configuring seats

`%APPDATA%\apollo-fleet\seats.toml` controls everything. Right-click tray → **Edit seats.toml** to open it in Notepad.

Minimal example:

```toml
apollo_path = 'C:\Program Files\Apollo\sunshine.exe'
state_dir   = 'C:\Users\<you>\.apollo-fleet'

[[seat]]
name         = "desk-1"
port         = 47989
audio_sink   = "Steam Streaming Speakers"
gamepad_only = false

[[seat]]
name         = "desk-2"
port         = 48029
audio_sink   = "CABLE Input (VB-Audio Virtual Cable)"
gamepad_only = true
```

Keys per seat:

| Key            | Required | Description                                                                                                                                                          |
|----------------|----------|----------------------------------------------------------------------------------------------------------------------------------------------------------------------|
| `name`         | yes      | Unique. Appears in Moonlight as the host name. Used as folder name under `state_dir`.                                                                                |
| `port`         | yes      | Apollo's primary HTTPS port. Apollo derives ~25 additional ports from this base, so leave at least **40 ports between seats** to avoid range collisions.             |
| `audio_sink`   | no       | The exact friendly name of the Windows playback endpoint to capture. Use **Show audio diagnostics** in the tray to list candidates. If empty, Apollo uses the default sink. |
| `gamepad_only` | no       | If `true`, the streamed client can only use gamepad input. Keyboard/mouse from the client are blocked so they can't steal focus from the host's input devices.       |

You can also include any Apollo-native config key in `[[seat]]`; unknown keys are passed through to that seat's `sunshine.conf`. Example: `min_log_level = "warning"`.

After editing, right-click tray → **Restart** to apply.

## Audio sinks

Apollo limits 4 simultaneous instances. Each seat ideally captures from a distinct virtual playback sink so clients don't share audio.

Free options:

| Driver                   | Sinks added                                       | Notes                                                                |
|--------------------------|---------------------------------------------------|----------------------------------------------------------------------|
| Steam Streaming Speakers | 1 (already present if Steam is installed)         | Default, no install needed.                                          |
| [VB-Cable](https://vb-audio.com/Cable/)             | 1 (`CABLE Input (VB-Audio Virtual Cable)`)        | Run installer as admin, reboot.                                      |
| [Voicemeeter Potato](https://vb-audio.com/Voicemeeter/potato.htm) | 3 (Voicemeeter Input, AUX Input, VAIO3 Input) | Run installer as admin, reboot. Bonus: per-app routing in Windows.   |

To see your active endpoints, right-click tray → **Show audio diagnostics**. Copy the exact name you see there into `audio_sink`.

To route a specific game's audio to a virtual sink, use Windows **Sound settings → App volume and device preferences**.

## Running with Administrator privileges

Apollo needs admin to create virtual displays via the IDD (Indirect Display Driver) included in its installer. Without admin, seats start but no virtual display appears and Moonlight shows a black screen.

Two ways:

1. **Right-click → Run as administrator** every time.
2. **Task Scheduler** — create a task that runs `ApolloFleet.exe` with highest privileges at logon. See [Auto-start on login](#auto-start-on-login).

## Auto-start on login

The cleanest way is Task Scheduler (so it can request admin without a UAC prompt):

1. Open **Task Scheduler** → **Create Task...**.
2. **General**: name `Apollo Fleet`. Check **Run with highest privileges**.
3. **Triggers**: New → **At log on**.
4. **Actions**: New → Start a program → `C:\Tools\ApolloFleet\ApolloFleet.exe`.
5. **Conditions**: uncheck **Start the task only if the computer is on AC power** (laptops).
6. **Settings**: uncheck **Stop the task if it runs longer than**.

Sign out and back in to verify.

## Tray menu reference

| Item                         | What it does                                                                                                       |
|------------------------------|--------------------------------------------------------------------------------------------------------------------|
| `Apollo Fleet v<version>`    | Version header. Disabled, informational.                                                                           |
| `Status: ...`                | Current state (`stopped`, `config error`, or `running: X connected / Y/N active`).                                 |
| **Start / Stop / Restart**   | Lifecycle of the whole fleet.                                                                                      |
| **Seats** > `<name> [master]`| Submenu per seat: open Apollo's web UI, move the foreground host window onto that seat's virtual display, open the seat's logs folder. `[master]` tags seat[0]. |
| **Set master credentials...**| Prompt for a username/password and apply it to every seat's Apollo. Use this after first install before pairing.   |
| **Edit seats.toml**          | Opens the config in your default editor.                                                                           |
| **Install audio drivers**    | Opens the VB-Cable / Voicemeeter Potato download pages.                                                            |
| **Show audio diagnostics**   | Writes a temp file with the list of active audio endpoints + last fleet error and opens it.                        |
| **Open state folder**        | Opens the fleet's state root (`state_dir` from `seats.toml`), where per-seat sub-folders live.                     |
| **Quit**                     | Stops the fleet and exits.                                                                                         |

## Master config propagation

The first seat in `seats.toml` is the **master**. Edit its config via Apollo's web UI (right-click tray → Seats → `<master>` → Open web UI). When you save, Apollo Fleet detects the change and rewrites every non-master seat's `sunshine.conf`, preserving each seat's per-seat keys:

- `sunshine_name`, `port`, `audio_sink`
- `file_apps`, `file_state`, `log_path`, `cert`, `pkey`, `credentials_file`
- `keyboard`, `mouse`, `controller` (when `gamepad_only = true`)

Everything else (encoder, bitrate, fps, qp, hevc_mode, etc.) is copied from master. Apollo Fleet's managed keys (`system_tray = disabled`, `headless_mode = enabled`, `dd_configuration_option = ensure_active`) are always re-applied so they can't be turned off via the web UI by accident.

Affected non-master seats are restarted automatically; the master is left alone. A connected master client sees no interruption.

## Per-seat credentials

The first time you pair, Apollo asks for a username and password. Use **Set master credentials...** to apply the same login to every seat at once. The credentials are stored in `credentials.json` inside each seat's state folder. You can also set them per-seat by clicking **Open web UI** on a single seat and following Apollo's normal flow.

## Pairing Moonlight clients

For each seat:

1. From the tray, right-click → **Seats** → choose the seat → **Open web UI**.
2. Log in with the credentials you set above.
3. In Moonlight, scan for hosts. The seat shows up with its `sunshine_name` (the seat name) and the host's hostname. Click **Add**.
4. Moonlight shows a 4-digit PIN. Enter the PIN in the seat's web UI under **PIN Pairing**.
5. Done. Repeat for the next seat (use a different Moonlight client or unpair-rescan if testing from the same device).

Each seat keeps its own pairing list. A pairing on one seat does not transfer to another.

## Logs and diagnostics

| File                                              | Purpose                                                                  |
|---------------------------------------------------|--------------------------------------------------------------------------|
| `<state_dir>/<seat>/sunshine.log`                 | Apollo's own log for that seat. Most useful for stream / encoder issues. |
| `<state_dir>/<seat>/fleet.log`                    | Apollo's stdout/stderr captured by Apollo Fleet. Less verbose.           |
| `%TEMP%/apollo-fleet-launcher.log`                | Output of the `--launch` callback flow (when Apollo runs an app like Steam Big Picture). |
| `%TEMP%/apollo-fleet-diag.txt`                    | One-shot diagnostics file. Generated by tray → **Show audio diagnostics**. |

To get more detail from the Apollo Fleet binary itself, run it from a terminal:

```powershell
cd C:\Tools\ApolloFleet
$env:RUST_LOG = "debug"
.\ApolloFleet.exe --supervisor --config "$env:APPDATA\apollo-fleet\seats.toml"
```

`--supervisor` runs the CLI supervisor without the tray, so logs print to the console.

## Troubleshooting

### Tray icon doesn't appear

- Make sure you ran `ApolloFleet.exe` as administrator (without admin, Apollo can't create virtual displays and Apollo Fleet may exit early).
- Some Windows versions hide tray icons by default. Click the chevron (^) in the notification area and drag the Apollo Fleet icon to the always-visible row.

### `Status: config error`

Right-click tray → **Show audio diagnostics**. Read `Last error:` in the file. Common causes:
- `Apollo binary not found: ...` — wrong `apollo_path` in seats.toml.
- `config has no [[seat]] entries` — empty config.
- `duplicate seat name: ...` / `duplicate seat port: ...` — fix the conflict.

### Seat exits immediately

Check `<state_dir>/<seat>/sunshine.log`. Common causes:
- **Port already in use** — another seat or another app holds the port. Increase the gap to 40+ between seats.
- **Missing IDD driver** — install Apollo (which bundles the IDD).
- **Audio sink not found** — wrong friendly name. Use **Show audio diagnostics** to copy the exact string.

After 3 exits in 30 seconds the seat is marked **dead** (circuit breaker). It stays dead until you fix the issue and **Restart** the fleet.

### Moonlight finds the host but the stream is black

The most common cause is Apollo running without admin — no virtual display was created. Quit Apollo Fleet, right-click → **Run as administrator**, try again.

### The exe is flagged by antivirus

Rust binaries from cold caches occasionally trip generic heuristics. If your AV blocks it:
1. Verify the binary is the official release from [github.com/alysnnix/apollo-fleet/releases](https://github.com/alysnnix/apollo-fleet/releases). Check the file size against the release page.
2. Submit to your vendor as a false positive.
3. If you want extra assurance, [build from source](#building-from-source) yourself.

### Tray menu items don't do anything

Make sure you're on v0.2.3 or newer. Earlier builds had a bug where `set_event_handler` swallowed clicks.

### `Video encoder has a maximum capacity of simultaneous encoding streams` in Apollo's log

This is a **GPU/driver** limit, not an Apollo Fleet or Apollo limit. The video encoder on your GPU only allows N simultaneous hardware-encoded streams:

| GPU                          | Typical concurrent encode sessions                        |
|------------------------------|-----------------------------------------------------------|
| NVIDIA GeForce (older driver, pre-R555) | 3                                              |
| NVIDIA GeForce (driver R555+, 2024 onward) | 8                                           |
| NVIDIA RTX Pro / Quadro      | unlimited (bounded by VRAM)                               |
| AMD Radeon (AMF)             | 2-4                                                       |
| Intel Arc / iGPU (QSV)       | 2-4                                                       |

Idle seats don't consume encoder slots — only seats with a Moonlight client actively streaming do. So defining 6 seats on a 3-session GPU works fine until a 4th client tries to connect; the lazy-spawn logic doesn't help you past the hardware cap.

Options to raise the cap:
- **Update your NVIDIA driver** to R555 or later.
- **Switch to a software encoder** by editing the master seat's web UI → set encoder to `Software` (`x264` / `libx264`). Costs CPU instead of GPU.
- **NVENC patch** (community, modifies the driver to remove the consumer cap). Unofficial; not endorsed.
- **Cut the seat count** to match your GPU's limit.

### Master config changes don't propagate

- Confirm the seat you're editing is **seat[0]** in `seats.toml`. Only that one acts as master.
- Look in the running console (run with `--supervisor` for log output) for `[propagate]` lines after you click Save in Apollo's web UI.
- File-watcher events can be missed if `%APPDATA%` is on a filesystem that doesn't support `ReadDirectoryChangesW` properly (rare).

## Uninstall

1. Right-click tray → **Quit**.
2. Delete `ApolloFleet.exe`.
3. Optionally remove state:
    - `%APPDATA%\apollo-fleet\` (your config and shared fleet state)
    - The folder pointed to by `state_dir` in `seats.toml` (per-seat state, certs, pairings, logs)
4. If you set up a Task Scheduler entry for auto-start, delete it.

## Building from source

Requires Rust stable (1.75+) on Windows.

```powershell
git clone https://github.com/alysnnix/apollo-fleet.git
cd apollo-fleet
cargo build --release
# output: target\release\apollo-fleet.exe
```

The build embeds:
- The Apollo logo `.ico` via `winresource` (Explorer / Alt-Tab / taskbar icon).
- A manifest declaring Common Controls v6 (needed by `rfd`'s TaskDialogIndirect).
- The example `seats.toml` and the monochrome tray SVG.

CI (`.github/workflows/release.yml`) builds release binaries on every `v*` tag push and attaches `ApolloFleet.exe` to the GitHub release.
