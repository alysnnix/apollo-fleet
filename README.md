<p align="center">
  <img src="resources/apollo-icon.svg" alt="Apollo Fleet" width="128" height="128" />
</p>

# Apollo Fleet

Supervise N isolated [Apollo / Sunshine](https://github.com/ClassicOldSong/Apollo) instances on a single Windows host so multiple Moonlight clients can connect to the same machine as if it were several hosts.

Each "seat" advertises itself in Moonlight with its own mDNS name, port range, state directory, and audio sink. Pair each Moonlight client to the seat it should use.

Written in Rust (single static `.exe`, ~3 MB) using the official `windows-rs` bindings, `tray-icon` from the Tauri ecosystem, and `winit` for the event loop. No Python, no PyInstaller bundle, no AV false-positives.

## Layout

```
apollo-fleet/
├── src/
│   ├── main.rs          # CLI dispatch (tray, --launch, --list-sinks, --dry-run)
│   ├── config.rs        # TOML loading + validation
│   ├── seat.rs          # generates sunshine.conf + apps.json, spawn/stop
│   ├── fleet.rs         # supervisor loop, lazy spawn, circuit breaker
│   ├── launcher.rs      # --launch mode (called by Apollo's apps.json)
│   ├── tray.rs          # tray-icon menu + actions
│   ├── shared_state.rs  # JSON state in %APPDATA%/apollo-fleet
│   ├── singleton.rs     # bind 127.0.0.1:47999 to enforce single instance
│   ├── paths.rs         # %APPDATA% / %TEMP% helpers
│   └── win/             # Win32 calls (windows-rs)
├── resources/
│   └── seats.toml.example
├── config/
│   └── seats.toml.example   # mirrored for hand-editing without rebuilding
├── scripts/
│   └── check-audio-sinks.ps1
└── .github/workflows/release.yml
```

## Quick start

```powershell
cargo build --release
.\target\release\apollo-fleet.exe                       # tray mode
.\target\release\apollo-fleet.exe --list-sinks          # audio endpoints
.\target\release\apollo-fleet.exe --supervisor          # CLI supervisor (no tray)
.\target\release\apollo-fleet.exe --dry-run             # generate per-seat configs only
```

On first run the binary copies `seats.toml.example` to `%APPDATA%\apollo-fleet\seats.toml`. Edit that file:

- `apollo_path`: full path to `sunshine.exe`
- `state_dir`: where per-seat state goes (keep it on a fast disk)
- One `[[seat]]` per Moonlight client, each with a unique `port` (40+ apart) and ideally a distinct `audio_sink`

Run as Administrator so Apollo can create virtual displays.

## Releases

CI builds a Windows binary on every `v*` tag and attaches `ApolloFleet.exe` to the resulting GitHub release. Local cut:

```powershell
git tag -a v0.2.0 -m "v0.2.0"
git push origin v0.2.0
```

## Audio sinks

Apollo limits 4 simultaneous instances. Each seat should capture from a distinct virtual playback sink so clients don't share audio.

- Steam Streaming Speakers (bundled with Steam)
- [VB-Cable](https://vb-audio.com/Cable/) — 1 extra sink (free)
- [Voicemeeter Potato](https://vb-audio.com/Voicemeeter/potato.htm) — 3 extra sinks (free)

## How it works

- Starts the first seat eagerly. Spawns the next seat only when every currently-spawned seat has an active client (debounced). The fleet always has exactly one idle seat ready, never more.
- Restarts a seat if Apollo exits unexpectedly. After 3 exits in 30 seconds the seat is marked dead (circuit breaker).
- Suppresses Apollo's own tray icon by locating its zserge/tray hidden window via `EnumWindows`/`GetClassNameW` and calling `Shell_NotifyIconW(NIM_DELETE)`.
- Detects active clients via `GetTcpTable2` (no shelling to `netstat`).
- When a client connects, identifies the newly-created virtual display (set difference vs. baseline) and moves the host's foreground window onto it.

## Master web UI

The first seat in `seats.toml` is the **master**. Its Apollo web UI at `https://localhost:{port+1}/` is the source of truth for shared settings (encoder, bitrate, fps, etc.). When you save changes there, Apollo Fleet:

1. Detects the change to the master's `sunshine.conf` via a file watcher.
2. Rewrites every non-master seat's `sunshine.conf`: master's keys, except per-seat ones (`port`, `sunshine_name`, `audio_sink`, paths, gamepad-only flags), which are preserved from the seat's existing conf.
3. Restarts the affected non-master seats (master stays running).

So one save in the master web UI propagates to the whole fleet. Per-seat customizations stay isolated.
