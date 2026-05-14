"""
Apollo Fleet — supervisor that runs N isolated Apollo instances on one Windows host.

Each "seat" advertises itself in Moonlight as a separate host (different mDNS name)
and binds its own port range, state directory, and audio sink. Pair each Moonlight
client to the seat it should use.

Usage:
    python -m apollo_fleet.supervisor --config config/seats.toml

Stop with Ctrl-C. Seats are restarted automatically if Apollo exits unexpectedly.
"""

import argparse
import json
import os
import signal
import subprocess
import sys
import threading
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path

POLL_INTERVAL_S = 2
RESTART_BACKOFF_S = 3
# Circuit breaker: if a seat exits this many times within FAILURE_WINDOW_S
# seconds, mark it dead and stop trying. Prevents infinite restart loops when
# the underlying problem (port in use, missing driver, etc.) won't fix itself.
MAX_FAILURES = 3
FAILURE_WINDOW_S = 30
# Debounce: a seat is only counted as "connected" if has_external_tcp_client
# returns True for this many consecutive polls. Filters out transient probes
# (Moonlight discovery scans, etc.) that briefly hit ESTABLISHED state.
CONNECTED_DEBOUNCE = 2

# subprocess flag to suppress console windows (no terminal flash from a frozen
# windowed app when shelling out to powershell/netstat/sunshine).
_NO_WINDOW = 0x08000000  # CREATE_NO_WINDOW


# ---- Hide Apollo's own system tray icon ------------------------------------
# Apollo v0.4.x compiles in zserge/tray and creates a tray icon unconditionally;
# `system_tray = disabled` is a no-op until master is released. We work around
# it by finding Apollo's hidden tray window (class "TRAY") and calling
# Shell_NotifyIcon(NIM_DELETE) ourselves to remove the icon.

def hide_apollo_tray_for_pid(pid: int) -> bool:
    """Locate the zserge/tray hidden window for this process and remove its
    tray icon. Returns True on success."""
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.windll.user32
    shell32 = ctypes.windll.shell32

    NIM_DELETE = 0x2

    class NOTIFYICONDATA(ctypes.Structure):
        _fields_ = [
            ("cbSize", wintypes.DWORD),
            ("hWnd", wintypes.HWND),
            ("uID", wintypes.UINT),
            ("uFlags", wintypes.UINT),
            ("uCallbackMessage", wintypes.UINT),
            ("hIcon", wintypes.HICON),
            ("szTip", ctypes.c_wchar * 128),
            ("dwState", wintypes.DWORD),
            ("dwStateMask", wintypes.DWORD),
            ("szInfo", ctypes.c_wchar * 256),
            ("uVersion", wintypes.UINT),
            ("szInfoTitle", ctypes.c_wchar * 64),
            ("dwInfoFlags", wintypes.DWORD),
            ("guidItem", ctypes.c_byte * 16),
            ("hBalloonIcon", wintypes.HICON),
        ]

    found: list[int] = []

    @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    def enum_proc(hwnd, _lparam):
        cls = ctypes.create_unicode_buffer(64)
        if user32.GetClassNameW(hwnd, cls, 64) <= 0 or cls.value != "TRAY":
            return True
        wpid = wintypes.DWORD()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(wpid))
        if wpid.value == pid:
            found.append(hwnd)
        return True

    user32.EnumWindows(enum_proc, 0)

    removed = False
    for hwnd in found:
        nid = NOTIFYICONDATA()
        nid.cbSize = ctypes.sizeof(nid)
        nid.hWnd = hwnd
        nid.uID = 0  # zserge/tray hardcodes uID = 0 in Shell_NotifyIcon NIM_ADD
        if shell32.Shell_NotifyIconW(NIM_DELETE, ctypes.byref(nid)):
            removed = True
    return removed


# ---- Display enumeration & window movement --------------------------------

def get_active_monitors() -> list[tuple[str, tuple[int, int, int, int]]]:
    """Return [(device_name, (left, top, right, bottom)), ...] for active monitors."""
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.windll.user32

    class MONITORINFOEX(ctypes.Structure):
        _fields_ = [
            ("cbSize", wintypes.DWORD),
            ("rcMonitor", wintypes.RECT),
            ("rcWork", wintypes.RECT),
            ("dwFlags", wintypes.DWORD),
            ("szDevice", ctypes.c_wchar * 32),
        ]

    monitors: list[tuple[str, tuple[int, int, int, int]]] = []

    @ctypes.WINFUNCTYPE(
        wintypes.BOOL, wintypes.HMONITOR, wintypes.HDC,
        ctypes.POINTER(wintypes.RECT), wintypes.LPARAM,
    )
    def cb(hmon, _hdc, _rect, _lparam):
        info = MONITORINFOEX()
        info.cbSize = ctypes.sizeof(info)
        if user32.GetMonitorInfoW(hmon, ctypes.byref(info)):
            r = info.rcMonitor
            monitors.append((info.szDevice, (r.left, r.top, r.right, r.bottom)))
        return True

    user32.EnumDisplayMonitors(0, None, cb, 0)
    return monitors


def get_foreground_window() -> int:
    import ctypes
    return ctypes.windll.user32.GetForegroundWindow() or 0


def move_window_to_rect(hwnd: int, rect: tuple[int, int, int, int]) -> None:
    """Move and resize a window to fill a monitor rect, then maximize it."""
    import ctypes
    user32 = ctypes.windll.user32
    SW_RESTORE = 9
    SW_MAXIMIZE = 3
    SWP_NOZORDER = 0x0004
    SWP_NOACTIVATE = 0x0010
    left, top, right, bottom = rect
    # Restore first so we can move (maximized windows can't be moved with SetWindowPos).
    user32.ShowWindow(hwnd, SW_RESTORE)
    user32.SetWindowPos(
        hwnd, 0, left, top, right - left, bottom - top,
        SWP_NOZORDER | SWP_NOACTIVATE,
    )
    user32.ShowWindow(hwnd, SW_MAXIMIZE)


def find_steam_install() -> Path | None:
    """Locate steam.exe via the registry, falling back to common paths."""
    import winreg
    for hive, key in [
        (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam"),
        (winreg.HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam"),
        (winreg.HKEY_CURRENT_USER, r"SOFTWARE\Valve\Steam"),
    ]:
        try:
            with winreg.OpenKey(hive, key) as k:
                path, _ = winreg.QueryValueEx(k, "InstallPath")
                exe = Path(path) / "steam.exe"
                if exe.exists():
                    return exe
        except OSError:
            continue
    for p in [
        Path(r"C:\Program Files (x86)\Steam\steam.exe"),
        Path(r"C:\Program Files\Steam\steam.exe"),
    ]:
        if p.exists():
            return p
    return None


def wait_for_process_window(pid: int, timeout_s: float = 30.0,
                             poll_s: float = 0.3) -> int | None:
    """Poll for the first visible top-level window owned by `pid` (or any of
    its descendants — covers Steam, which spawns a relauncher)."""
    import ctypes
    from ctypes import wintypes

    user32 = ctypes.windll.user32

    deadline = time.monotonic() + timeout_s

    while time.monotonic() < deadline:
        # Build set of related PIDs (the launched proc + any children we can see).
        try:
            related = _descendant_pids(pid)
        except Exception:
            related = {pid}

        candidate: list[int] = []

        @ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
        def cb(hwnd, _lparam):
            if not user32.IsWindowVisible(hwnd):
                return True
            wpid = wintypes.DWORD()
            user32.GetWindowThreadProcessId(hwnd, ctypes.byref(wpid))
            if wpid.value not in related:
                return True
            length = user32.GetWindowTextLengthW(hwnd)
            if length <= 0:
                return True
            candidate.append(hwnd)
            return True

        user32.EnumWindows(cb, 0)
        if candidate:
            return candidate[0]
        time.sleep(poll_s)
    return None


def _descendant_pids(root_pid: int) -> set[int]:
    """Return root_pid plus all PIDs whose parent chain leads back to it."""
    import ctypes
    from ctypes import wintypes

    TH32CS_SNAPPROCESS = 0x2

    class PROCESSENTRY32(ctypes.Structure):
        _fields_ = [
            ("dwSize", wintypes.DWORD),
            ("cntUsage", wintypes.DWORD),
            ("th32ProcessID", wintypes.DWORD),
            ("th32DefaultHeapID", ctypes.c_void_p),
            ("th32ModuleID", wintypes.DWORD),
            ("cntThreads", wintypes.DWORD),
            ("th32ParentProcessID", wintypes.DWORD),
            ("pcPriClassBase", wintypes.LONG),
            ("dwFlags", wintypes.DWORD),
            ("szExeFile", ctypes.c_char * 260),
        ]

    kernel32 = ctypes.windll.kernel32
    snap = kernel32.CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
    parents: dict[int, int] = {}
    try:
        pe = PROCESSENTRY32()
        pe.dwSize = ctypes.sizeof(pe)
        if kernel32.Process32First(snap, ctypes.byref(pe)):
            while True:
                parents[pe.th32ProcessID] = pe.th32ParentProcessID
                if not kernel32.Process32Next(snap, ctypes.byref(pe)):
                    break
    finally:
        kernel32.CloseHandle(snap)

    result = {root_pid}
    changed = True
    while changed:
        changed = False
        for child, parent in parents.items():
            if parent in result and child not in result:
                result.add(child)
                changed = True
    return result


def hide_apollo_tray_with_retry(pid: int, attempts: int = 15, interval_s: float = 1.0) -> bool:
    """Apollo registers its tray icon a few seconds after launch (after encoder
    discovery). Poll for it and remove as soon as it appears."""
    for _ in range(attempts):
        if hide_apollo_tray_for_pid(pid):
            return True
        time.sleep(interval_s)
    return False


@dataclass
class SeatCfg:
    name: str
    port: int
    audio_sink: str = ""
    gamepad_only: bool = False
    extra: dict = None


class Seat:
    def __init__(self, cfg: SeatCfg, apollo_path: Path, state_root: Path):
        self.cfg = cfg
        self.apollo_path = apollo_path
        self.state_dir = state_root / cfg.name
        self.state_dir.mkdir(parents=True, exist_ok=True)
        self.config_file = self.state_dir / "sunshine.conf"
        self.fleet_log = self.state_dir / "fleet.log"
        self.proc: subprocess.Popen | None = None
        self._write_config()
        self.write_apps_json()

    def write_apps_json(self, fleet_exe: Path | None = None) -> None:
        """Generate apps.json so Apollo's app picker shows the launchable
        targets we want (e.g. Steam Big Picture) instead of just the default."""
        apps_path = self.state_dir / "apps.json"
        if fleet_exe is None:
            fleet_exe = Path(sys.executable)
        apps: list[dict] = []
        steam = find_steam_install()
        if steam is not None:
            apps.append({
                "name": "Steam",
                "image-path": "steam.png",
                "auto-detach": True,
                "wait-all": False,
                "exit-timeout": 5,
                "cmd": (
                    f'"{fleet_exe}" --launch {self.cfg.name} '
                    f'-- "{steam}" -gamepadui'
                ),
            })
        # Always keep a desktop-fallback entry so the user can drop into the
        # virtual display without launching anything.
        apps.append({
            "name": "Desktop",
            "image-path": "desktop.png",
        })
        apps_path.write_text(json.dumps({"env": {}, "apps": apps}, indent=2),
                              encoding="utf-8")

    def _write_config(self) -> None:
        # Apollo accepts `key = value` lines. State paths are isolated per seat so
        # pairings, certs, and apps don't collide between instances.
        opts = {
            "sunshine_name": self.cfg.name,
            "port": self.cfg.port,
            "file_apps": self.state_dir / "apps.json",
            "file_state": self.state_dir / "sunshine_state.json",
            "log_path": self.state_dir / "sunshine.log",
            "cert": self.state_dir / "cert.pem",
            "pkey": self.state_dir / "pkey.pem",
            "credentials_file": self.state_dir / "credentials.json",
            "headless_mode": "enabled",
            "dd_configuration_option": "ensure_active",
            "system_tray": "disabled",
        }
        if self.cfg.audio_sink:
            opts["audio_sink"] = self.cfg.audio_sink
        if self.cfg.gamepad_only:
            # Block keyboard/mouse from the streamed client so it can't steal
            # input from the host's physical keyboard/mouse.
            opts["keyboard"] = "disabled"
            opts["mouse"] = "disabled"
            opts["controller"] = "enabled"
        if self.cfg.extra:
            opts.update(self.cfg.extra)

        lines = [f"{k} = {v}" for k, v in opts.items()]
        self.config_file.write_text("\n".join(lines) + "\n", encoding="utf-8")

    def start(self) -> None:
        log = open(self.fleet_log, "ab")
        self.proc = subprocess.Popen(
            [str(self.apollo_path), str(self.config_file)],
            stdout=log,
            stderr=log,
            # Apollo loads its shaders/assets relative to CWD — point it at the
            # install dir so files like assets/shaders/directx/*.hlsl resolve.
            cwd=str(self.apollo_path.parent),
            creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | _NO_WINDOW,
        )

    def stop(self) -> None:
        if not self.proc or self.proc.poll() is not None:
            return
        try:
            self.proc.send_signal(signal.CTRL_BREAK_EVENT)
            self.proc.wait(timeout=5)
            return
        except (subprocess.TimeoutExpired, OSError):
            pass
        self.proc.terminate()
        try:
            self.proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            self.proc.kill()
            self.proc.wait(timeout=3)

    def alive(self) -> bool:
        return self.proc is not None and self.proc.poll() is None

    def exit_code(self) -> int | None:
        return self.proc.poll() if self.proc else None


def _is_loopback_addr(host: str) -> bool:
    return host.startswith("127.") or host in {"::1", "[::1]", "0.0.0.0", "[::]", "::"}


def has_external_tcp_client(port: int) -> bool:
    """Return True if `port` has an ESTABLISHED TCP connection where neither
    side is loopback. Suppresses the netstat console window."""
    try:
        out = subprocess.run(
            ["netstat", "-an", "-p", "TCP"],
            capture_output=True, text=True, timeout=5, check=False,
            creationflags=_NO_WINDOW,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return False
    needle = f":{port}"
    for line in out.stdout.splitlines():
        if needle not in line or "ESTABLISHED" not in line:
            continue
        parts = line.split()
        if len(parts) < 4:
            continue
        local, foreign = parts[1], parts[2]
        if not local.endswith(needle):
            continue
        local_host = local.rsplit(":", 1)[0]
        foreign_host = foreign.rsplit(":", 1)[0]
        # Skip loopback-to-loopback (Apollo's internal IPC, web UI to itself).
        if _is_loopback_addr(local_host) or _is_loopback_addr(foreign_host):
            continue
        return True
    return False


def list_audio_endpoints() -> list[str]:
    """Return active Windows audio render endpoint friendly names."""
    ps = (
        "Get-PnpDevice -Class AudioEndpoint -Status OK -ErrorAction SilentlyContinue | "
        "ForEach-Object { $_.FriendlyName }"
    )
    try:
        out = subprocess.run(
            ["powershell", "-NoProfile", "-NonInteractive", "-Command", ps],
            capture_output=True, text=True, timeout=10, check=False,
            creationflags=_NO_WINDOW,
        )
    except (FileNotFoundError, subprocess.TimeoutExpired):
        return []
    return [line.strip() for line in out.stdout.splitlines() if line.strip()]


def validate_audio_sinks(seats: list[SeatCfg]) -> list[str]:
    """Return a list of error strings for seats whose audio_sink isn't present."""
    needed = [(s.name, s.audio_sink) for s in seats if s.audio_sink]
    if not needed:
        return []
    endpoints = list_audio_endpoints()
    if not endpoints:
        return ["could not enumerate audio endpoints (powershell unavailable?)"]
    lower = [e.lower() for e in endpoints]
    errors = []
    for seat_name, sink in needed:
        if not any(sink.lower() in e for e in lower):
            errors.append(f"seat '{seat_name}' wants audio_sink '{sink}' -- not found among active endpoints")
    return errors


def load_config(path: Path) -> tuple[Path, Path, list[SeatCfg]]:
    raw = tomllib.loads(path.read_text(encoding="utf-8"))
    apollo_path = Path(raw.get("apollo_path", r"C:\Program Files\Apollo\sunshine.exe"))
    state_dir = Path(raw.get("state_dir", str(Path.home() / ".apollo-fleet")))

    seats_raw = raw.get("seat", [])
    if not seats_raw:
        raise SystemExit("config has no [[seat]] entries")

    seats = []
    seen_names, seen_ports = set(), set()
    for s in seats_raw:
        name = s["name"]
        port = int(s["port"])
        if name in seen_names:
            raise SystemExit(f"duplicate seat name: {name}")
        if port in seen_ports:
            raise SystemExit(f"duplicate seat port: {port}")
        seen_names.add(name)
        seen_ports.add(port)
        known = {"name", "port", "audio_sink", "gamepad_only"}
        extra = {k: v for k, v in s.items() if k not in known}
        seats.append(SeatCfg(
            name=name,
            port=port,
            audio_sink=s.get("audio_sink", ""),
            gamepad_only=bool(s.get("gamepad_only", False)),
            extra=extra,
        ))

    # Warn if seats are too close — Apollo derives ~25 ports from the base.
    sorted_ports = sorted(seen_ports)
    for a, b in zip(sorted_ports, sorted_ports[1:]):
        if b - a < 40:
            print(f"[fleet] WARNING: seats at port {a} and {b} are <40 apart; ranges may collide", file=sys.stderr)

    return apollo_path, state_dir, seats


class Fleet:
    """Manages a pool of Apollo seats with lazy spawn.

    Seats are configured ahead of time but spawned one at a time: the first seat
    is started eagerly so a client can discover/connect; the next seat in the
    pool is spawned only when the previously-spawned seats all have an external
    client connected. This way the fleet always has exactly one idle seat ready
    for the next device, and never spawns more than needed.
    """

    def __init__(self, seats: list[Seat], log=print):
        self.seats = seats  # full template pool, ordered
        self.log = log
        self._stop = threading.Event()
        self._thread: threading.Thread | None = None
        self._failures: dict[str, list[float]] = {}
        self._dead: set[str] = set()
        # Indices of seats that have been started (not stopped or dead). A seat
        # in this list may or may not have an active client — see has_client().
        self._spawned_idx: list[int] = []
        # Debounce buffer: per-seat list of last N raw connection observations.
        # Seat is "connected" only when ALL recent observations were True.
        self._client_history: dict[str, list[bool]] = {}
        # Track the last debounced "connected" state per seat to detect transitions.
        self._was_connected: dict[str, bool] = {}
        # Per-seat: the virtual monitor's device name and rect, once discovered.
        self._seat_monitor: dict[str, tuple[str, tuple[int, int, int, int]]] = {}
        # Set of monitor device names ever seen — used to identify new (virtual)
        # monitors as seats connect.
        self._known_monitor_names: set[str] = set()
        # Frozen baseline (only the physical monitors, captured at start()) so
        # launcher processes can compute "current - baseline" as a fallback.
        self._initial_monitor_names: set[str] = set()

    def is_running(self) -> bool:
        return self._thread is not None and self._thread.is_alive()

    @property
    def spawned_seats(self) -> list[Seat]:
        return [self.seats[i] for i in self._spawned_idx]

    def _record_client_observation(self, seat: Seat) -> None:
        """Sample current raw connection state and append to history buffer."""
        hist = self._client_history.setdefault(seat.cfg.name, [])
        hist.append(has_external_tcp_client(seat.cfg.port))
        if len(hist) > CONNECTED_DEBOUNCE:
            del hist[0]

    def _state_file(self) -> Path:
        """Shared file consumed by ApolloFleet --launch instances."""
        base = Path(os.environ.get("APPDATA", str(Path.home()))) / "apollo-fleet"
        base.mkdir(parents=True, exist_ok=True)
        return base / "fleet-state.json"

    def _write_shared_state(self) -> None:
        """Persist baseline + per-seat virtual monitor info so a launcher
        process (started by Apollo when a Moonlight client picks an app) can
        find the right display to move the launched window onto.

        Baseline is the set of monitors present BEFORE any seat opened a
        client session, so a launcher can compute "current - baseline" as
        a fallback if its seat's per-seat entry hasn't been populated yet.
        """
        state = {
            "baseline_monitors": sorted(self._initial_monitor_names),
            "seats": {
                name: {"monitor_device": dev, "monitor_rect": list(rect)}
                for name, (dev, rect) in self._seat_monitor.items()
            },
        }
        try:
            self._state_file().write_text(json.dumps(state), encoding="utf-8")
        except OSError as e:
            self.log(f"[fleet] could not write shared state: {e}")

    def virtual_monitor_for(self, seat: Seat) -> tuple[str, tuple[int, int, int, int]] | None:
        """Return (device_name, rect) of the virtual display tracked for this
        seat, or None if no client has connected yet."""
        return self._seat_monitor.get(seat.cfg.name)

    def has_client(self, seat: Seat) -> bool:
        """Debounced: only True if last CONNECTED_DEBOUNCE observations were all True."""
        hist = self._client_history.get(seat.cfg.name, [])
        if len(hist) < CONNECTED_DEBOUNCE:
            return False
        return all(hist[-CONNECTED_DEBOUNCE:])

    def client_count(self) -> int:
        return sum(1 for s in self.spawned_seats if self.has_client(s))

    def start(self) -> None:
        if self.is_running():
            return
        self._stop.clear()
        # Snapshot baseline monitors so we can detect virtual displays added by
        # Apollo when clients connect.
        baseline = {dev for dev, _ in get_active_monitors()}
        self._known_monitor_names = set(baseline)
        self._initial_monitor_names = set(baseline)
        # Reset shared state on every fleet start so launcher processes don't
        # read stale data from a previous session.
        self._seat_monitor.clear()
        self._write_shared_state()
        # Eager-spawn the first seat so the fleet is immediately discoverable.
        if self.seats:
            self._spawn_index(0)
        self._thread = threading.Thread(target=self._supervise, daemon=True)
        self._thread.start()

    def stop(self, timeout: float = 10) -> None:
        self._stop.set()
        if self._thread:
            self._thread.join(timeout=timeout)
            self._thread = None
        # Stop in reverse spawn order.
        for idx in reversed(self._spawned_idx):
            s = self.seats[idx]
            s.stop()
            self.log(f"[fleet] stopped '{s.cfg.name}'")
        self._spawned_idx.clear()

    def _spawn_index(self, idx: int) -> None:
        s = self.seats[idx]
        if idx in self._spawned_idx or s.cfg.name in self._dead:
            return
        s.start()
        self._spawned_idx.append(idx)
        self.log(f"[fleet] started '{s.cfg.name}' port={s.cfg.port} pid={s.proc.pid}")
        # Apollo creates its tray icon a few seconds after launch (after encoder
        # discovery). Apollo v0.4.x ignores `system_tray = disabled`, so we hunt
        # the icon down ourselves in a background thread.
        threading.Thread(
            target=self._strip_apollo_tray, args=(s.cfg.name, s.proc.pid), daemon=True,
        ).start()

    def _strip_apollo_tray(self, name: str, pid: int) -> None:
        if hide_apollo_tray_with_retry(pid):
            self.log(f"[fleet] hid Apollo tray icon for '{name}'")
        else:
            self.log(f"[fleet] could not find tray icon to hide for '{name}'")

    def dead_seats(self) -> list[str]:
        return sorted(self._dead)

    def _record_failure(self, name: str, now: float) -> bool:
        """Record an exit and return True if the seat should be marked dead."""
        history = self._failures.setdefault(name, [])
        history.append(now)
        # Drop entries outside the window.
        cutoff = now - FAILURE_WINDOW_S
        self._failures[name] = [t for t in history if t >= cutoff]
        return len(self._failures[name]) >= MAX_FAILURES

    def _supervise(self) -> None:
        last_restart: dict[str, float] = {}
        while not self._stop.is_set():
            # Restart-on-crash for already-spawned seats.
            for idx in self._spawned_idx:
                s = self.seats[idx]
                if s.cfg.name in self._dead or s.alive():
                    continue
                now = time.monotonic()
                if now - last_restart.get(s.cfg.name, 0) < RESTART_BACKOFF_S:
                    continue
                code = s.exit_code()
                if self._record_failure(s.cfg.name, now):
                    self._dead.add(s.cfg.name)
                    self.log(
                        f"[fleet] '{s.cfg.name}' exited {MAX_FAILURES}x in {FAILURE_WINDOW_S}s "
                        f"(last code={code}); giving up. Check {s.state_dir / 'sunshine.log'}"
                    )
                    continue
                last_restart[s.cfg.name] = now
                self.log(f"[fleet] '{s.cfg.name}' exited (code={code}); restarting")
                s.start()

            # Sample raw connection state for each spawned-and-alive seat.
            alive_spawned = [self.seats[i] for i in self._spawned_idx
                             if self.seats[i].alive() and self.seats[i].cfg.name not in self._dead]
            for s in alive_spawned:
                self._record_client_observation(s)

            # Detect "client just connected" transition (debounced) and trigger
            # the auto-move flow on first connect of each session.
            for s in alive_spawned:
                connected_now = self.has_client(s)
                was = self._was_connected.get(s.cfg.name, False)
                self._was_connected[s.cfg.name] = connected_now
                if connected_now and not was:
                    threading.Thread(
                        target=self._on_client_connected, args=(s,), daemon=True,
                    ).start()
                elif not connected_now and was:
                    # Forget seat<->monitor mapping when the client disconnects so
                    # a fresh monitor is picked up on next connect (Apollo can
                    # recreate displays with different device names).
                    self._seat_monitor.pop(s.cfg.name, None)
                    self._write_shared_state()

            # Lazy spawn: spawn the next configured seat only when every active
            # seat has had a sustained client connection (debounced).
            if alive_spawned and all(self.has_client(s) for s in alive_spawned):
                next_idx = self._next_unspawned_index()
                if next_idx is not None:
                    self.log(
                        f"[fleet] all {len(alive_spawned)} active seat(s) busy; "
                        f"spawning next seat to keep an idle slot available"
                    )
                    self._spawn_index(next_idx)

            self._stop.wait(POLL_INTERVAL_S)

    def _on_client_connected(self, seat: Seat) -> None:
        """When a client first connects to `seat`, identify the new virtual
        display Apollo created for it and move the host's foreground window
        onto that display so the streamer sees it immediately."""
        # Apollo activates its virtual display ~1-3s after the client handshakes.
        time.sleep(2)

        current = get_active_monitors()
        new_monitors = [(dev, rect) for dev, rect in current
                        if dev not in self._known_monitor_names
                        and dev not in (m for m, _ in self._seat_monitor.values())]
        if not new_monitors:
            self.log(
                f"[fleet] '{seat.cfg.name}' client connected, but no new "
                f"display detected -- skipping auto-move"
            )
            return

        device, rect = new_monitors[0]
        self._seat_monitor[seat.cfg.name] = (device, rect)
        self._known_monitor_names.add(device)
        self._write_shared_state()
        self.log(f"[fleet] '{seat.cfg.name}' assigned virtual display '{device}' rect={rect}")

        hwnd = get_foreground_window()
        if not hwnd:
            self.log(f"[fleet] no foreground window to move for '{seat.cfg.name}'")
            return
        try:
            move_window_to_rect(hwnd, rect)
            self.log(f"[fleet] moved foreground window {hwnd} to '{device}' for '{seat.cfg.name}'")
        except Exception as e:
            self.log(f"[fleet] move failed for '{seat.cfg.name}': {e}")

    def _next_unspawned_index(self) -> int | None:
        for i, s in enumerate(self.seats):
            if i in self._spawned_idx:
                continue
            if s.cfg.name in self._dead:
                continue
            return i
        return None


def _shared_state_path() -> Path:
    return (Path(os.environ.get("APPDATA", str(Path.home())))
            / "apollo-fleet" / "fleet-state.json")


def _read_shared_state() -> dict:
    p = _shared_state_path()
    if not p.exists():
        return {}
    try:
        return json.loads(p.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return {}


def _resolve_target_rect(seat_name: str, log_fn) -> tuple[int, int, int, int] | None:
    """Determine the virtual-display rect this seat's launched app should land
    on. Tries the per-seat entry first; falls back to "current monitors minus
    baseline" if the supervisor hasn't populated the per-seat entry yet."""
    state = _read_shared_state()
    seat_entry = (state.get("seats") or {}).get(seat_name)
    if seat_entry:
        rect = seat_entry.get("monitor_rect")
        if rect and len(rect) == 4:
            log_fn(f"resolved monitor for '{seat_name}' from per-seat state: {rect}")
            return tuple(rect)  # type: ignore[return-value]

    baseline = set(state.get("baseline_monitors") or [])
    if not baseline:
        log_fn("no baseline_monitors in state file -- fleet not running?")
        return None

    current = get_active_monitors()
    new_devices = [(dev, rect) for dev, rect in current if dev not in baseline]
    if not new_devices:
        log_fn("no virtual displays detected (current == baseline)")
        return None
    if len(new_devices) > 1:
        log_fn(f"multiple new displays found {[d for d,_ in new_devices]}; "
               f"picking first")
    dev, rect = new_devices[0]
    log_fn(f"resolved monitor for '{seat_name}' via baseline-delta: {dev} {rect}")
    return rect


def run_launcher(seat_name: str, exec_args: list[str]) -> int:
    """Spawn an external program, then move its window onto the seat's virtual
    display. Invoked by Apollo when a Moonlight client picks an app — the cmd
    in apps.json calls back into this binary in --launch mode."""
    log_path = Path(os.environ.get("TEMP", str(Path.home()))) / "apollo-fleet-launcher.log"

    def log(msg: str) -> None:
        try:
            with open(log_path, "a", encoding="utf-8") as f:
                f.write(f"{time.strftime('%Y-%m-%d %H:%M:%S')} [{seat_name}] {msg}\n")
        except OSError:
            pass

    log(f"--launch invoked, exec_args={exec_args}")

    if not exec_args:
        log("error: no exec_args")
        return 2

    DETACHED = 0x00000008  # DETACHED_PROCESS
    try:
        proc = subprocess.Popen(
            exec_args,
            creationflags=DETACHED | _NO_WINDOW,
            close_fds=True,
        )
    except OSError as e:
        log(f"failed to launch {exec_args[0]}: {e}")
        return 2
    log(f"spawned pid={proc.pid}")

    # Race the supervisor: poll the state file for up to 15s while the spawned
    # app initializes. Fall back to baseline-delta if per-seat info never lands.
    rect: tuple[int, int, int, int] | None = None
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        rect = _resolve_target_rect(seat_name, log)
        if rect is not None:
            break
        time.sleep(0.5)

    log(f"waiting for window from pid={proc.pid} (or descendants)")
    hwnd = wait_for_process_window(proc.pid, timeout_s=30.0)
    if hwnd is None:
        log("timed out waiting for visible window")
        return 0
    log(f"found window hwnd={hwnd}")

    if rect is None:
        # One more chance after the window is up.
        for _ in range(10):
            rect = _resolve_target_rect(seat_name, log)
            if rect is not None:
                break
            time.sleep(0.5)

    if rect is None:
        log("no rect resolved -- skipping move")
        return 0
    try:
        move_window_to_rect(hwnd, rect)
        log(f"moved hwnd={hwnd} to rect={rect}")
    except Exception as e:
        log(f"move failed: {e}")
    return 0


def build_fleet(config_path: Path, skip_sink_check: bool = False, log=print) -> Fleet:
    """Load config, instantiate seats, return a non-running Fleet.

    Sink validation is non-blocking: missing sinks are logged as warnings and
    Apollo will fall back to the system default device. The fleet still starts.
    """
    apollo_path, state_root, seat_cfgs = load_config(config_path)
    if not apollo_path.exists():
        raise SystemExit(f"Apollo binary not found: {apollo_path}")

    if not skip_sink_check:
        for warning in validate_audio_sinks(seat_cfgs):
            log(f"[fleet] warning: {warning}")

    state_root.mkdir(parents=True, exist_ok=True)
    seats = [Seat(c, apollo_path, state_root) for c in seat_cfgs]
    return Fleet(seats, log=log)


def main() -> int:
    parser = argparse.ArgumentParser(description="Apollo fleet supervisor")
    parser.add_argument("--config", "-c", default="seats.toml", help="Path to TOML config")
    parser.add_argument("--dry-run", action="store_true", help="Generate per-seat configs without spawning Apollo")
    parser.add_argument("--list-sinks", action="store_true", help="List active Windows audio endpoints and exit")
    parser.add_argument("--skip-sink-check", action="store_true", help="Don't verify audio_sink names exist before launch")
    args = parser.parse_args()

    if args.list_sinks:
        endpoints = list_audio_endpoints()
        if not endpoints:
            print("(no endpoints found — is PowerShell available?)")
            return 1
        for name in endpoints:
            print(name)
        return 0

    config_path = Path(args.config).resolve()
    if not config_path.exists():
        print(f"config not found: {config_path}", file=sys.stderr)
        return 2

    try:
        fleet = build_fleet(config_path, skip_sink_check=args.skip_sink_check)
    except SystemExit as e:
        print(str(e), file=sys.stderr)
        return 2

    if args.dry_run:
        for s in fleet.seats:
            print(f"[dry-run] {s.cfg.name} -> {s.config_file}")
        return 0

    stop_event = threading.Event()

    def request_stop(*_):
        if not stop_event.is_set():
            print("\n[fleet] shutdown requested")
            stop_event.set()

    signal.signal(signal.SIGINT, request_stop)
    if hasattr(signal, "SIGBREAK"):
        signal.signal(signal.SIGBREAK, request_stop)

    fleet.start()
    try:
        stop_event.wait()
    finally:
        fleet.stop()

    return 0


if __name__ == "__main__":
    sys.exit(main())
