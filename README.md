# Apollo Fleet

Supervise N isolated [Apollo / Sunshine](https://github.com/ClassicOldSong/Apollo) instances on a single Windows host so multiple Moonlight clients can connect to the same machine as if it were several hosts.

Each "seat" advertises itself in Moonlight with its own mDNS name, port range, state directory, and audio sink. Pair each Moonlight client to the seat it should use.

## Layout

```
apollo-fleet/
├── src/apollo_fleet/
│   ├── supervisor.py    # fleet supervisor (process lifecycle, restart, circuit breaker)
│   └── tray.py          # pystray icon for the bundled Windows app
├── scripts/
│   └── check-audio-sinks.ps1
├── packaging/
│   └── ApolloFleet.spec # PyInstaller spec
├── config/
│   └── seats.toml.example
├── pyproject.toml
└── requirements.txt
```

## Quick start (dev)

```powershell
python -m venv .venv
.venv\Scripts\activate
pip install -e .

copy config\seats.toml.example config\seats.toml
# edit config\seats.toml: apollo_path, state_dir, and one [[seat]] per Moonlight client

python -m apollo_fleet.supervisor --config config\seats.toml     # CLI supervisor
python -m apollo_fleet.tray       --config config\seats.toml     # tray app
```

Run as Administrator so Apollo can create virtual displays.

## Build the Windows executable

```powershell
pip install pyinstaller
pyinstaller packaging\ApolloFleet.spec
# output: dist\ApolloFleet.exe
```

On first run, the exe copies `seats.toml.example` to `%APPDATA%\apollo-fleet\seats.toml`.

## Audio sinks

Apollo limits 4 simultaneous instances. Each seat should capture from a distinct virtual playback sink so clients don't share audio.

- Steam Streaming Speakers (bundled with Steam)
- [VB-Cable](https://vb-audio.com/Cable/) — 1 extra sink (free)
- [Voicemeeter Potato](https://vb-audio.com/Voicemeeter/potato.htm) — 3 extra sinks (free)

List the active playback endpoints on the host:

```powershell
scripts\check-audio-sinks.ps1
# or:
python -m apollo_fleet.supervisor --list-sinks
```

## How it works

- Starts the first seat eagerly. Spawns the next seat only when every currently-spawned seat has an active client. The fleet always has exactly one idle seat ready for the next device, and never more.
- Restarts a seat if Apollo exits unexpectedly. After `MAX_FAILURES` exits in `FAILURE_WINDOW_S` seconds the seat is marked dead (circuit breaker).
- Suppresses Apollo's own tray icon by locating the zserge/tray hidden window and removing the icon via `Shell_NotifyIcon(NIM_DELETE)`.

## Requirements

- Windows 10/11
- Python 3.11+
- Apollo / Sunshine installed (default path: `C:\Program Files\Apollo\sunshine.exe`)
