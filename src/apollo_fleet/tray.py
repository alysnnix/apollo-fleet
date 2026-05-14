"""
Apollo Fleet — tray app.

Single tray icon that supervises N hidden Apollo instances. Apollo's own tray
icon is suppressed (system_tray = disabled in generated configs). Right-click
the tray icon for: start/stop, per-seat web UI, edit config, quit.

Run with admin to allow Apollo to create virtual displays.
"""

import argparse
import os
import shutil
import socket
import subprocess
import sys
import threading
import webbrowser
from pathlib import Path

from PIL import Image, ImageDraw
import pystray

from . import supervisor as apollo_fleet


SINGLETON_PORT = 47999  # arbitrary loopback port; bind succeeds only for the first instance


def acquire_singleton(port: int = SINGLETON_PORT) -> socket.socket | None:
    """Try to bind a loopback port. Returns the socket if we're the only instance,
    None if another instance is already running."""
    s = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 0)
    try:
        s.bind(("127.0.0.1", port))
        s.listen(1)
        return s
    except OSError:
        s.close()
        return None


SCRIPT_DIR = Path(__file__).resolve().parent
PROJECT_ROOT = SCRIPT_DIR.parent.parent


def _is_frozen() -> bool:
    return getattr(sys, "frozen", False)


def _bundled_resource(name: str) -> Path:
    """Path to a resource bundled into the PyInstaller exe, or in the repo's config/ dir."""
    if _is_frozen() and hasattr(sys, "_MEIPASS"):
        return Path(sys._MEIPASS) / name
    return PROJECT_ROOT / "config" / name


def _user_config_dir() -> Path:
    if _is_frozen():
        return Path(os.environ.get("APPDATA", str(Path.home()))) / "apollo-fleet"
    return PROJECT_ROOT / "config"


DEFAULT_CONFIG = _user_config_dir() / "seats.toml"


def ensure_user_config(config_path: Path) -> None:
    """On first run from the .exe, copy seats.toml.example to the user config dir."""
    if config_path.exists():
        return
    config_path.parent.mkdir(parents=True, exist_ok=True)
    bundled = _bundled_resource("seats.toml.example")
    if bundled.exists():
        shutil.copyfile(bundled, config_path)


def make_icon(running: bool, color_running=(0, 180, 90), color_stopped=(180, 60, 60)) -> Image.Image:
    """Generate a 64x64 tray icon. Filled circle, color depends on state."""
    img = Image.new("RGBA", (64, 64), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    color = color_running if running else color_stopped
    d.ellipse((8, 8, 56, 56), fill=color, outline=(20, 20, 20, 255), width=2)
    d.text((22, 18), "AF", fill=(255, 255, 255, 255))
    return img


def open_path_in_explorer(path: Path) -> None:
    if path.exists():
        subprocess.Popen(["explorer", str(path)])


def open_in_editor(path: Path) -> None:
    # Falls back to OS default — usually Notepad for .toml.
    os.startfile(str(path))


class TrayApp:
    def __init__(self, config_path: Path, skip_sink_check: bool):
        self.config_path = config_path
        self.skip_sink_check = skip_sink_check
        self.fleet: apollo_fleet.Fleet | None = None
        self.last_error: str | None = None
        self.icon: pystray.Icon | None = None
        self._build_icon()

    # --- fleet lifecycle -------------------------------------------------

    def _ensure_fleet(self) -> bool:
        if self.fleet is not None:
            return True
        try:
            self.fleet = apollo_fleet.build_fleet(
                self.config_path,
                skip_sink_check=self.skip_sink_check,
                log=self._fleet_log,
            )
            self.last_error = None
            return True
        except SystemExit as e:
            self.last_error = str(e)
            self._notify("Fleet config error", str(e).split("\n")[0])
            return False

    def _fleet_log(self, line: str) -> None:
        # Keep stdout for diagnostics; tray itself is silent.
        print(line, flush=True)

    def start_fleet(self, _icon=None, _item=None) -> None:
        if not self._ensure_fleet():
            self._refresh_icon()
            return
        if not self.fleet.is_running():
            self.fleet.start()
            self._notify("Apollo Fleet", f"Started {len(self.fleet.seats)} seat(s)")
        self._refresh_icon()

    def stop_fleet(self, _icon=None, _item=None) -> None:
        if self.fleet and self.fleet.is_running():
            self.fleet.stop()
            self._notify("Apollo Fleet", "Stopped")
        self._refresh_icon()

    def restart_fleet(self, _icon=None, _item=None) -> None:
        self.stop_fleet()
        # Force config reload by tearing down the Fleet object.
        self.fleet = None
        self.start_fleet()

    def quit_app(self, _icon=None, _item=None) -> None:
        self.stop_fleet()
        if self.icon:
            self.icon.stop()

    # --- menu actions ----------------------------------------------------

    def open_seat_web_ui(self, seat: apollo_fleet.Seat):
        # Apollo web admin runs on (port + 1) on localhost.
        def handler(_icon=None, _item=None):
            url = f"https://localhost:{seat.cfg.port + 1}/"
            webbrowser.open(url)
        return handler

    def open_seat_logs(self, seat: apollo_fleet.Seat):
        def handler(_icon=None, _item=None):
            open_path_in_explorer(seat.state_dir)
        return handler

    def move_foreground_to_seat(self, seat: apollo_fleet.Seat):
        """Move the host's currently-focused window onto this seat's virtual display."""
        def handler(_icon=None, _item=None):
            if not self.fleet:
                return
            mon = self.fleet.virtual_monitor_for(seat)
            if not mon:
                self._notify(
                    "Move failed",
                    f"No virtual display tracked for '{seat.cfg.name}'. Connect a client first.",
                )
                return
            hwnd = apollo_fleet.get_foreground_window()
            if not hwnd:
                self._notify("Move failed", "No foreground window.")
                return
            try:
                apollo_fleet.move_window_to_rect(hwnd, mon[1])
                self._notify("Window moved", f"Moved to '{seat.cfg.name}' display.")
            except Exception as e:
                self._notify("Move failed", str(e)[:140])
        return handler

    def edit_config(self, _icon=None, _item=None):
        open_in_editor(self.config_path)

    def open_state_dir(self, _icon=None, _item=None):
        # The fleet state root is under ~/.apollo-fleet by default.
        if self.fleet and self.fleet.seats:
            open_path_in_explorer(self.fleet.seats[0].state_dir.parent)
        else:
            open_path_in_explorer(Path.home() / ".apollo-fleet")

    def set_master_credentials(self, _icon=None, _item=None):
        """Prompt the user once and apply the same Apollo creds to all seats."""
        # Run dialog + sunshine --creds calls off the pystray thread.
        threading.Thread(target=self._set_master_credentials_impl, daemon=True).start()

    def _set_master_credentials_impl(self) -> None:
        import tkinter as tk
        from tkinter import simpledialog, messagebox

        if not self.fleet or not self.fleet.seats:
            return
        root = tk.Tk()
        root.withdraw()
        root.attributes("-topmost", True)
        try:
            user = simpledialog.askstring(
                "Apollo Fleet — Set credentials",
                "Username (will apply to all seats):",
                parent=root,
            )
            if not user:
                return
            pw = simpledialog.askstring(
                "Apollo Fleet — Set credentials",
                "Password:",
                parent=root, show="*",
            )
            if not pw:
                return

            was_running = self._is_running()
            if was_running:
                self.fleet.stop()

            failed: list[tuple[str, str]] = []
            for seat in self.fleet.seats:
                try:
                    res = subprocess.run(
                        [str(seat.apollo_path), str(seat.config_file),
                         "--creds", user, pw],
                        cwd=str(seat.apollo_path.parent),
                        capture_output=True, text=True, timeout=15,
                        creationflags=apollo_fleet._NO_WINDOW,
                    )
                    if res.returncode != 0:
                        failed.append((seat.cfg.name, res.stderr.strip() or f"exit {res.returncode}"))
                except Exception as e:
                    failed.append((seat.cfg.name, str(e)))

            if was_running:
                self.fleet.start()
            self._refresh_icon()

            if failed:
                err = "\n".join(f"  - {n}: {msg}" for n, msg in failed)
                messagebox.showerror(
                    "Apollo Fleet",
                    f"Failed to set credentials on {len(failed)} seat(s):\n{err}",
                    parent=root,
                )
            else:
                messagebox.showinfo(
                    "Apollo Fleet",
                    f"Credentials applied to {len(self.fleet.seats)} seat(s).\n\n"
                    "Open any seat's web UI and log in with these credentials. "
                    "Your browser's password manager will autofill subsequent visits.",
                    parent=root,
                )
        finally:
            root.destroy()

    def install_vbcable(self, _icon=None, _item=None):
        webbrowser.open("https://vb-audio.com/Cable/")
        self._notify(
            "VB-Cable installer",
            "Download the installer, run it as admin, reboot, then restart "
            "Apollo Fleet. Adds 1 virtual sink ('CABLE Input').",
        )

    def install_voicemeeter(self, _icon=None, _item=None):
        webbrowser.open("https://vb-audio.com/Voicemeeter/potato.htm")
        self._notify(
            "Voicemeeter Potato installer",
            "Download the installer, run it as admin, reboot, then restart "
            "Apollo Fleet. Adds 3 virtual sinks (Voicemeeter Input/AUX/VAIO3).",
        )

    # Steam app id for Borderless Gaming (https://store.steampowered.com/app/388080/).
    BORDERLESS_GAMING_STEAM_ID = 388080

    def launch_borderless_gaming(self, _icon=None, _item=None):
        """Ask Steam to run Borderless Gaming. Requires the game to be in the
        user's Steam library; Steam handles install/launch on its end."""
        url = f"steam://rungameid/{self.BORDERLESS_GAMING_STEAM_ID}"
        try:
            os.startfile(url)
        except OSError as e:
            self._notify("Borderless Gaming", f"Couldn't ask Steam to run it: {e}")
            return
        self._notify(
            "Borderless Gaming",
            "Launching via Steam. Once it opens, add the game window to "
            "Favorites — keeps gamepad input alive while the host is in use.",
        )

    def show_focus_tip(self, _icon=None, _item=None):
        """Open a tip file explaining how to keep gamepad input flowing while
        the host user uses other apps."""
        out = Path(os.environ.get("TEMP", str(Path.home()))) / "apollo-fleet-focus-tip.txt"
        out.write_text(
            "Apollo Fleet — keeping gamepad input alive while host is in use\n"
            + "=" * 60 + "\n\n"
            "WHY THIS HAPPENS\n"
            "----------------\n"
            "Apollo always forwards gamepad events to a virtual XInput device.\n"
            "That part works regardless of focus. The problem is that most\n"
            "Windows games STOP processing input when their own window loses\n"
            "focus. So when you click somewhere on the host, the game on the\n"
            "virtual display sees 'I lost focus' and ignores incoming events.\n\n"
            "There is no purely-software fix that works for EVERY game without\n"
            "injecting code into the game's process (which anti-cheat systems\n"
            "treat as cheating). The options below cover ~95% of real cases.\n\n"
            "FIX 1 — In-game setting (works for most modern singleplayer games)\n"
            "------------------------------------------------------------------\n"
            "Look in the game's options for one of these:\n"
            "  - 'Pause when window not focused' / 'Pause on focus loss'\n"
            "  - 'Background mode' / 'Allow input in background'\n"
            "  - 'Run in background'\n"
            "Disable it.\n\n"
            "Confirmed examples:\n"
            "  - Baldur's Gate 3 -> Settings > Gameplay\n"
            "  - Cyberpunk 2077  -> Settings > Gameplay\n"
            "  - The Witcher 3   -> Options > Gameplay > Advanced\n"
            "  - Skyrim/Fallout  -> ENB or .ini tweak (bAlwaysActive=1)\n"
            "  - Diablo 4        -> Options > Gameplay\n\n"
            "FIX 2 — Borderless Gaming (works for games without that setting)\n"
            "---------------------------------------------------------------\n"
            "Tool that intercepts WM_KILLFOCUS so the game thinks it always\n"
            "has focus. Sold on Steam for a small fee:\n"
            "  https://store.steampowered.com/app/388080/\n"
            "Once installed in your Steam library, use the tray menu:\n"
            "  Apollo Fleet > 'Launch Borderless Gaming'\n"
            "Add your game to its Favorites list once; from then on it keeps\n"
            "your inputs alive automatically.\n\n"
            "FIX 3 — Exclusive fullscreen on the virtual display\n"
            "---------------------------------------------------\n"
            "Some games behave better in exclusive (not borderless) fullscreen\n"
            "because they capture input at a lower level. Worth trying if FIX 1\n"
            "and FIX 2 don't help for a specific game.\n\n"
            "FIX 4 — True multiseat (Aster, paid)\n"
            "------------------------------------\n"
            "Aster Multiseat creates real separate Windows sessions, each with\n"
            "its own input queue. Cleanest possible solution but ~$70 license.\n\n"
            "WHAT WON'T WORK\n"
            "---------------\n"
            "Multiplayer competitive games (CS2, Valorant, Fortnite) actively\n"
            "block focus-stealing tricks via anti-cheat — there is no safe\n"
            "way to make those work in background-streamed mode.\n",
            encoding="utf-8",
        )
        os.startfile(str(out))

    def show_diagnostics(self, _icon=None, _item=None):
        """Write a diagnostics file with active audio endpoints + last error."""
        endpoints = apollo_fleet.list_audio_endpoints()
        out = Path(os.environ.get("TEMP", str(Path.home()))) / "apollo-fleet-diag.txt"
        lines = [
            "Apollo Fleet — diagnostics",
            "=" * 40,
            f"Config:       {self.config_path}",
            f"Last error:   {self.last_error or '(none)'}",
            "",
            "Active Windows audio endpoints (use any of these as audio_sink):",
        ]
        lines.extend(f"  - {n}" for n in endpoints) if endpoints else lines.append("  (none detected)")
        lines += [
            "",
            "If you don't see a 'virtual' sink (Voicemeeter / VB-Cable / Steam",
            "Streaming Speakers), install one of these to enable per-seat audio:",
            "  - Voicemeeter Potato: https://vb-audio.com/Voicemeeter/potato.htm",
            "  - VB-Cable:           https://vb-audio.com/Cable/",
        ]
        out.write_text("\n".join(lines), encoding="utf-8")
        os.startfile(str(out))

    # --- icon / menu wiring ---------------------------------------------

    def _build_icon(self) -> None:
        self.icon = pystray.Icon(
            "apollo-fleet",
            icon=make_icon(running=False),
            title="Apollo Fleet — stopped",
            menu=self._build_menu(),
        )

    def _build_menu(self) -> pystray.Menu:
        seat_items: list = []
        if self.fleet:
            spawned_set = set(id(s) for s in self.fleet.spawned_seats)
            for s in self.fleet.seats:
                spawned = id(s) in spawned_set
                connected = spawned and self.fleet.has_client(s)
                if connected:
                    label = f"{s.cfg.name} (port {s.cfg.port}) — connected"
                elif spawned:
                    label = f"{s.cfg.name} (port {s.cfg.port}) — idle"
                else:
                    label = f"{s.cfg.name} (port {s.cfg.port}) — pending"
                seat_items.append(
                    pystray.MenuItem(
                        label,
                        pystray.Menu(
                            pystray.MenuItem("Open web UI", self.open_seat_web_ui(s),
                                             enabled=spawned),
                            pystray.MenuItem("Move foreground game here",
                                             self.move_foreground_to_seat(s),
                                             enabled=connected),
                            pystray.MenuItem("Open logs folder", self.open_seat_logs(s)),
                        ),
                    )
                )
        if not seat_items:
            seat_items.append(pystray.MenuItem("(no seats loaded yet)", None, enabled=False))

        return pystray.Menu(
            pystray.MenuItem(
                lambda _: self._status_label(),
                None,
                enabled=False,
            ),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem("Start", self.start_fleet, enabled=lambda _: not self._is_running()),
            pystray.MenuItem("Stop", self.stop_fleet, enabled=lambda _: self._is_running()),
            pystray.MenuItem("Restart", self.restart_fleet, enabled=lambda _: self._is_running()),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem("Seats", pystray.Menu(*seat_items)),
            pystray.MenuItem("Set master credentials...", self.set_master_credentials),
            pystray.MenuItem("Edit seats.toml", self.edit_config),
            pystray.MenuItem(
                "Install audio drivers",
                pystray.Menu(
                    pystray.MenuItem("VB-Cable (free, +1 sink)", self.install_vbcable),
                    pystray.MenuItem("Voicemeeter Potato (free, +3 sinks)", self.install_voicemeeter),
                ),
            ),
            pystray.MenuItem("Show audio diagnostics", self.show_diagnostics),
            pystray.MenuItem("Launch Borderless Gaming", self.launch_borderless_gaming),
            pystray.MenuItem("Gamepad focus tip", self.show_focus_tip),
            pystray.MenuItem("Open state folder", self.open_state_dir),
            pystray.Menu.SEPARATOR,
            pystray.MenuItem("Quit", self.quit_app),
        )

    def _is_running(self) -> bool:
        return self.fleet is not None and self.fleet.is_running()

    def _status_label(self) -> str:
        if self.last_error:
            return "Status: config error"
        if self._is_running():
            total = len(self.fleet.seats)
            spawned = len(self.fleet.spawned_seats)
            connected = self.fleet.client_count()
            dead = len(self.fleet.dead_seats())
            base = f"running: {connected} connected / {spawned}/{total} active"
            if dead:
                base += f" ({dead} dead)"
            return f"Status: {base}"
        return "Status: stopped"

    def _refresh_icon(self) -> None:
        if not self.icon:
            return
        self.icon.icon = make_icon(running=self._is_running())
        self.icon.title = f"Apollo Fleet — {self._status_label().split(': ', 1)[1]}"
        # Rebuild menu because seat list may have changed after loading.
        self.icon.menu = self._build_menu()
        self.icon.update_menu()

    def _notify(self, title: str, msg: str) -> None:
        if self.icon:
            try:
                self.icon.notify(msg, title)
            except Exception:
                pass

    # --- entry -----------------------------------------------------------

    def _refresh_loop(self) -> None:
        """Periodically refresh the tray icon/menu so connection state is visible."""
        import time
        while True:
            time.sleep(3)
            try:
                self._refresh_icon()
            except Exception:
                pass

    def run(self) -> int:
        threading.Thread(target=self.start_fleet, daemon=True).start()
        threading.Thread(target=self._refresh_loop, daemon=True).start()
        self.icon.run()
        return 0


def main() -> int:
    # The --launch subcommand needs to capture an arbitrary exec command after
    # the seat name, so we split on the special "--" separator before handing
    # the rest to argparse.
    argv = sys.argv[1:]
    if "--launch" in argv:
        i = argv.index("--launch")
        try:
            seat = argv[i + 1]
        except IndexError:
            print("--launch requires a seat name", file=sys.stderr)
            return 2
        # Everything after seat name is the command to launch (with optional --
        # separator stripped).
        rest = argv[i + 2:]
        if rest and rest[0] == "--":
            rest = rest[1:]
        return apollo_fleet.run_launcher(seat, rest)

    p = argparse.ArgumentParser(description="Apollo Fleet tray app")
    p.add_argument("--config", "-c", default=str(DEFAULT_CONFIG))
    p.add_argument("--skip-sink-check", action="store_true")
    args = p.parse_args()

    config_path = Path(args.config).resolve()
    ensure_user_config(config_path)
    if not config_path.exists():
        print(f"config not found and example missing: {config_path}", file=sys.stderr)
        return 2

    lock = acquire_singleton()
    if lock is None:
        # Another ApolloFleet instance is already running. Exit silently —
        # the running instance's tray icon is the user's interface.
        return 0
    try:
        return TrayApp(config_path, skip_sink_check=args.skip_sink_check).run()
    finally:
        lock.close()


if __name__ == "__main__":
    sys.exit(main())
