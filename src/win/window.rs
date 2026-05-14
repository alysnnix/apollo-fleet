use std::time::{Duration, Instant};

use windows::Win32::Foundation::{BOOL, HWND, LPARAM, TRUE};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowTextLengthW, GetWindowThreadProcessId, IsWindowVisible,
    SetWindowPos, ShowWindow, HWND_TOP, SET_WINDOW_POS_FLAGS, SHOW_WINDOW_CMD,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_MAXIMIZE, SW_RESTORE,
};

pub fn foreground_window() -> isize {
    unsafe { GetForegroundWindow().0 as isize }
}

pub fn move_to_rect(hwnd: isize, rect: [i32; 4]) {
    let hwnd = HWND(hwnd as *mut _);
    let [left, top, right, bottom] = rect;
    unsafe {
        // Restore first — maximized windows can't be moved via SetWindowPos.
        let _ = ShowWindow(hwnd, SW_RESTORE);
        let flags: SET_WINDOW_POS_FLAGS = SWP_NOZORDER | SWP_NOACTIVATE;
        let _ = SetWindowPos(hwnd, HWND_TOP, left, top, right - left, bottom - top, flags);
        let _ = ShowWindow(hwnd, SW_MAXIMIZE);
    }
}

#[allow(dead_code)]
fn show_window(hwnd: HWND, cmd: SHOW_WINDOW_CMD) {
    unsafe {
        let _ = ShowWindow(hwnd, cmd);
    }
}

/// Wait for the first visible top-level window owned by `pid` (or any of its descendants).
/// Returns the HWND as isize.
pub fn wait_for_process_window(pid: u32, timeout: Duration) -> Option<isize> {
    let deadline = Instant::now() + timeout;
    loop {
        let related = super::process::descendant_pids(pid);
        let mut candidate = WaitCtx { pids: related, hwnd: 0 };
        let lparam = LPARAM(&mut candidate as *mut WaitCtx as isize);
        unsafe {
            let _ = EnumWindows(Some(wait_proc), lparam);
        }
        if candidate.hwnd != 0 {
            return Some(candidate.hwnd);
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
}

struct WaitCtx {
    pids: Vec<u32>,
    hwnd: isize,
}

unsafe extern "system" fn wait_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut WaitCtx);
    if !IsWindowVisible(hwnd).as_bool() {
        return TRUE;
    }
    let mut wpid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut wpid));
    if !ctx.pids.contains(&wpid) {
        return TRUE;
    }
    if GetWindowTextLengthW(hwnd) <= 0 {
        return TRUE;
    }
    ctx.hwnd = hwnd.0 as isize;
    BOOL(0)
}
