// Hide Apollo's own zserge/tray icon by locating its hidden tray window (class "TRAY")
// and calling Shell_NotifyIcon(NIM_DELETE) ourselves. Apollo v0.4.x ignores
// `system_tray = disabled` in the config until master ships, so we strip the icon manually.

use std::thread::sleep;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
use windows::Win32::UI::Shell::{Shell_NotifyIconW, NIM_DELETE, NOTIFYICONDATAW};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetClassNameW, GetWindowThreadProcessId};

struct FindCtx {
    pid: u32,
    hwnds: Vec<isize>,
}

pub fn hide_for_pid(pid: u32) -> bool {
    let mut ctx = FindCtx { pid, hwnds: Vec::new() };
    let lparam = LPARAM(&mut ctx as *mut FindCtx as isize);
    unsafe {
        let _ = EnumWindows(Some(enum_proc), lparam);
    }
    let mut removed = false;
    for raw in ctx.hwnds {
        let hwnd = HWND(raw as *mut _);
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 0; // zserge/tray hardcodes uID=0
        let ok = unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
        if ok.as_bool() {
            removed = true;
        }
    }
    removed
}

pub fn hide_for_pid_with_retry(pid: u32, attempts: u32, interval: Duration) -> bool {
    for _ in 0..attempts {
        if hide_for_pid(pid) {
            return true;
        }
        sleep(interval);
    }
    false
}

unsafe extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut FindCtx);
    let mut buf = [0u16; 64];
    let len = GetClassNameW(hwnd, &mut buf);
    if len <= 0 {
        return TRUE;
    }
    let name = String::from_utf16_lossy(&buf[..len as usize]);
    if name != "TRAY" {
        return TRUE;
    }
    let mut wpid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut wpid));
    if wpid == ctx.pid {
        ctx.hwnds.push(hwnd.0 as isize);
    }
    TRUE
}

// Silence unused-import warning for PCWSTR — kept for future expansion if needed.
#[allow(dead_code)]
fn _retain_pcwstr_import(_: PCWSTR) {}
