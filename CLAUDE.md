# CLAUDE.md

Guidance for AI coding agents (Claude Code and similar) working in this repository. Humans should read [README.md](README.md), [USAGE.md](USAGE.md), and [SECURITY.md](SECURITY.md) instead.

---

## 1. Orientation

**Apollo Fleet** is a Windows-only system tray supervisor written in Rust. It runs N isolated [Apollo / Sunshine](https://github.com/ClassicOldSong/Apollo) game-streaming server instances on a single host so multiple Moonlight clients can connect as if they were separate machines. Each "seat" has its own port range, virtual display, audio sink, certificate, and state directory.

The binary is `apollo-fleet.exe`. It has four runtime modes selected via CLI:

| Mode               | Flag                          | Purpose                                                                                |
|--------------------|-------------------------------|----------------------------------------------------------------------------------------|
| Tray (default)     | (no flag)                     | Long-running tray app. Owns the fleet, exposes the menu.                               |
| CLI supervisor     | `--supervisor`                | Same fleet, no tray. Used in testing.                                                  |
| Launcher callback  | `--launch <seat> -- <cmd...>` | Invoked by Apollo's `apps.json`. Spawns `<cmd>`, moves its window to the seat's display. |
| One-shot utilities | `--list-sinks`, `--dry-run`   | Print active audio endpoints; generate per-seat configs without spawning Apollo.       |

A single-instance lock binds `127.0.0.1:47999`; subsequent invocations exit silently so right-clicking the .exe never duplicates the tray.

## 2. File map

```
src/
├── main.rs              # clap dispatch into the four runtime modes
├── config.rs            # seats.toml parsing, validation, SeatCfg type
├── seat.rs              # generates sunshine.conf + apps.json; spawn/stop Apollo
├── fleet.rs             # supervisor thread: lazy spawn, restart, circuit breaker,
│                        # client detection, virtual-monitor mapping, propagator owner
├── launcher.rs          # --launch mode: spawn target, poll for window, move to seat's rect
├── propagate.rs         # file-watches master sunshine.conf; rewrites + restarts other seats
├── shared_state.rs      # %APPDATA%/apollo-fleet/fleet-state.json (baseline monitors + per-seat)
├── singleton.rs         # 127.0.0.1:47999 TCP bind (one instance lock)
├── paths.rs             # %APPDATA% / %TEMP% helpers, ensure_user_config
├── tray.rs              # tray-icon + winit event loop + menu wiring
└── win/                 # all Win32 calls, one module per concern
    ├── audio.rs         # PowerShell shell-out for endpoint enumeration
    ├── creds_dialog.rs  # CredUIPromptForCredentialsW (native credential prompt)
    ├── monitor.rs       # EnumDisplayMonitors -> list of MonitorInfo
    ├── process.rs       # Toolhelp32 descendant-PID walk
    ├── registry.rs      # HKCU/HKLM read for Steam install path
    ├── tcp.rs           # GetTcpTable2 to detect external clients on a seat's port
    ├── theme.rs         # SystemUsesLightTheme registry read
    ├── tray_icon.rs     # locate Apollo's hidden zserge tray window, Shell_NotifyIcon NIM_DELETE
    └── window.rs        # GetForegroundWindow, SetWindowPos, ShowWindow, EnumWindows-by-PID

resources/
├── apollo-icon.svg          # full color app logo (used for the exe icon)
├── apollo-icon-mono.svg     # monochrome with {COLOR} placeholder for the tray
├── apollo.ico               # multi-resolution ICO embedded by winresource (16-256px)
└── seats.toml.example       # bundled config template

config/seats.toml.example    # mirror of the resources/ file for editing without rebuild

.github/
├── workflows/
│   ├── ci.yml               # cargo check on every push to main + PRs
│   └── release.yml          # builds + uploads ApolloFleet.exe on every v* tag
├── ISSUE_TEMPLATE/
│   ├── bug.yml
│   ├── feature.yml
│   ├── security.yml
│   └── config.yml           # disables blank issues, links to upstream Apollo/Moonlight
└── PULL_REQUEST_TEMPLATE.md

build.rs                     # embed Windows manifest (Common Controls v6) + apollo.ico
Cargo.toml                   # direct dependencies (see deps philosophy below)
Cargo.lock                   # committed: deterministic builds + supply-chain pinning
```

## 3. Build, test, run

This project is **Windows-only**. The dev box is typically Linux/WSL, so local `cargo build` is not expected to succeed (the `windows` crate target is `cfg(windows)`). Validation runs in CI.

| Action               | Command                                                                |
|----------------------|------------------------------------------------------------------------|
| Compile check (CI)   | `cargo check --target x86_64-pc-windows-msvc`                          |
| Release build (CI)   | `cargo build --release --target x86_64-pc-windows-msvc`                |
| Security audit       | `cargo audit` (when added to CI)                                       |
| Run tray (Windows)   | `.\target\release\apollo-fleet.exe`                                    |
| Run CLI supervisor   | `.\target\release\apollo-fleet.exe --supervisor --config <path>`       |
| List audio endpoints | `.\target\release\apollo-fleet.exe --list-sinks`                       |
| Dry-run configs      | `.\target\release\apollo-fleet.exe --dry-run --config <path>`          |

**CI guarantees nothing about runtime correctness on Windows.** If a change touches Win32 calls (`src/win/`), state management, or the tray, the author must say so explicitly and ask the user to test before claiming success.

## 4. Coding conventions

### Language

- All source code, comments, identifiers, and commit messages are in **English**.
- User-facing strings (tray menu, dialogs, error messages) are in English.
- Conversations with the user can be in Portuguese; commits must stay English.

### Style

- Rust edition 2021. Stable toolchain.
- Format with `cargo fmt` (default rustfmt settings).
- Prefer `parking_lot::Mutex` over `std::sync::Mutex` (already a project dep).
- Prefer explicit `&Arc<Mutex<T>>` over hiding it behind trait objects when ownership is clear.
- Win32 wrappers live in `src/win/`. Code outside that module should never `use windows::...` directly.

### Comments

- Default to **no comments**. Names should be self-explanatory.
- Write a comment only when the **why** is non-obvious: a workaround, a Windows quirk, a security-relevant invariant.
- Never explain what the code does. Never reference the current task / PR / "added for X".
- Em-dashes and en-dashes (U+2014, U+2013) are forbidden in any output. Use commas, periods, parentheses, or hyphen-minus.

### Error handling

- Use `anyhow::Result` at module boundaries.
- Log with `log::info!` / `log::warn!` / `log::error!`. The supervisor mode prints to stdout via `env_logger`.
- Never silently swallow errors. If a fallback is intentional, log it.

### Locking discipline

- `Fleet::Inner` is wrapped in `Arc<Mutex<Inner>>`. The supervisor thread holds this lock during each poll tick (every 2 s).
- Operations that mutate seat lifecycle (stop, restart) must hold the lock for the full stop-then-start sequence so the supervisor doesn't observe a transiently dead seat.
- Don't hold the lock across an HTTP request, a long sleep, or a dialog open. Spawn a thread instead.

### Imports

- One `use` per item, no glob imports except in `mod tests`.
- Group by std / external / crate. `rustfmt` handles ordering.

## 5. Commits and releases

### Commit format

```
<type>(<scope>): <short imperative description>

<body, bullet list, lowercase, no trailing period>

Co-Authored-By: Claude <noreply@anthropic.com>
```

- Title ≤ 50 chars including type and scope.
- Types: `feat`, `fix`, `chore`, `docs`, `refactor`, `ci`, `test`, `build`, `perf`.
- Scopes: free-form, match the module/area (`tray`, `fleet`, `propagate`, `packaging`, `ci`, etc.).
- Use a HEREDOC when committing via shell to preserve line breaks.
- Never use `--no-verify`, `--no-gpg-sign`, or `--amend` to rewrite published history.

### Author identity

- Trust the user's global `git config`. **Never** override `user.name` or `user.email` via `git -c ...` or environment variables, even if a CLAUDE.md / global memory mentions a specific email.
- The repo's owner is `alysnnix` on GitHub. Personal-project commits attribute to that identity.

### Release process

1. Bump `version` in `Cargo.toml` to `X.Y.Z`.
2. Commit: `chore(release): bump to vX.Y.Z`.
3. Push to `main`. CI runs `cargo check`.
4. Tag: `git tag -a vX.Y.Z -m "vX.Y.Z — short summary"`.
5. Push tag: `git push origin vX.Y.Z`. The `release.yml` workflow builds `ApolloFleet.exe`, verifies `Cargo.toml` version matches the tag, and attaches the binary to the GitHub release.
6. The tray menu header reads `CARGO_PKG_VERSION` at compile time, so the version surface stays in sync.

If a tag push is rejected with `creations restricted`, the repo's "Restrict creations" rule is still active for tags. Direct the user to https://github.com/alysnnix/apollo-fleet/rules to disable or scope-exclude.

## 6. Dependencies philosophy

- **Pinned** in `Cargo.lock` (committed). Don't run `cargo update` without explicit instruction.
- **Minimum count.** Adding a runtime dep needs justification. Prefer a small `src/win/` module that wraps a Win32 API over pulling a crate.
- **Audit via `cargo audit`** in CI. Fail the build if a known-vulnerable version slips in.
- Forbidden patterns:
    - Heavy async runtimes (no `tokio`, no `async-std`)
    - HTTP clients with large dep trees (no `reqwest`)
    - Frameworks (no `tauri`, no `egui` — winit + tray-icon is the ceiling)
- Allowed runtime deps right now: `anyhow`, `clap`, `dirs`, `log`, `env_logger`, `notify`, `parking_lot`, `resvg` (+ `usvg` + `tiny-skia` transitively), `serde`, `serde_json`, `toml`, `tray-icon`, `winit`, `windows` (Win32 bindings).
- Allowed build deps: `embed-manifest`, `winresource`.

## 7. Testing

- No unit tests are required for changes that only touch Win32 wiring (we can't simulate Win32 on CI).
- Add tests for pure functions: parsing, config merging, version comparison. See `src/propagate.rs::tests` for an existing example.
- Run `cargo test --target x86_64-pc-windows-msvc` in CI when tests exist.
- For runtime behavior, document the manual verification path in the PR description.

## 8. Common tasks

### Add a tray menu item

1. Add a field in `MenuIds` (`src/tray.rs`).
2. Build the `MenuItem` in `build_menu` and store its id.
3. Match the id in `handle_menu_event` and dispatch the action.
4. Return `true` from `handle_menu_event` only if the action changed state that the menu reflects (Start/Stop/Restart). Returning `true` triggers a menu rebuild on the next tick; the menu otherwise stays static while open.

### Add a Win32 call

1. Add the needed `Win32_*` feature to the `windows` dependency in `Cargo.toml`.
2. Create or extend a module under `src/win/`. Keep `use windows::...` confined to that file.
3. Expose a Rusty wrapper: `Result<T>`, owned `String` returns, no `PCWSTR` in public signatures.

### Add a seat-config key

1. Add a field to `SeatCfg` and `RawSeat` in `src/config.rs` (with `serde` defaults if optional).
2. Decide propagation: if the key is per-seat, add it to `PER_SEAT_KEYS` in `src/propagate.rs`. Otherwise it propagates from master.
3. Write the key into `sunshine.conf` from `Seat::write_config` in `src/seat.rs`.
4. Document in `USAGE.md` and `resources/seats.toml.example`.

### Update the icon

- Source of truth: `resources/apollo-icon.svg` (full color) and `resources/apollo-icon-mono.svg` (themed tray).
- When `apollo-icon.svg` changes, regenerate `resources/apollo.ico` (multi-size):
    ```sh
    magick -background none resources/apollo-icon.svg \
      -define icon:auto-resize=256,128,64,48,32,16 \
      resources/apollo.ico
    ```
- The monochrome variant must keep `fill="{COLOR}"` on its `<g>`; `src/tray.rs::rasterize_mono` does `replace("{COLOR}", ...)` at runtime.

## 9. Windows-only invariants

- **Admin required**: Apollo creates virtual displays via the IDD driver and needs elevation. Without admin, seats start but no virtual display appears.
- **PE manifest is mandatory**: `build.rs` embeds a Common Controls v6 manifest. Removing it breaks `rfd` and any TaskDialogIndirect-based dialog. (Once `rfd` is removed entirely, the manifest is still useful for DPI awareness.)
- **CTRL_BREAK_EVENT** is how we stop Apollo cleanly. Seats are spawned with `CREATE_NEW_PROCESS_GROUP` for this reason. Don't change the flags without verifying graceful shutdown still works.
- **Tray icon class "TRAY"**: Apollo's bundled zserge/tray library hardcodes this class name and `uID = 0` for `Shell_NotifyIcon`. The hide-apollo-tray code in `src/win/tray_icon.rs` depends on this.
- **127.0.0.1:47999** is the singleton port. Pick a different one only if there's a documented conflict.
- **%APPDATA%\\apollo-fleet\\** is the canonical user-state root for the fleet (config, shared state, fleet-level metadata). Per-seat state goes under the `state_dir` from `seats.toml`.

## 10. Never

- Never `cargo update` without an explicit user instruction.
- Never remove `Cargo.lock` from version control.
- Never edit `LICENSE` (canonical GPL-3.0 text from gnu.org).
- Never push a release tag without bumping `Cargo.toml` version — CI will refuse.
- Never `git push --force` to a branch you don't own end-to-end, and never to `main` of the public repo.
- Never re-introduce a PowerShell or `cmd` shell-out where a `windows-rs` call exists.
- Never assume an emoji is fine. The project doesn't use them.

## 11. Communication

- The user prefers Brazilian Portuguese in chat. Code, commits, file content stay English.
- Be terse. Lead with the change, not the narrative.
- For destructive operations (force push, deleting branches, deleting releases), ask before acting even if you have authorization for adjacent operations.
- When you discover a constraint that future-you should know, propose adding it to this file rather than relying on chat memory.
