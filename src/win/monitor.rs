use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT, TRUE};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
};

#[derive(Debug, Clone)]
pub struct MonitorInfo {
    pub device: String,
    pub rect: [i32; 4],
}

pub fn list_active() -> Vec<MonitorInfo> {
    let mut monitors: Vec<MonitorInfo> = Vec::new();
    let ptr = &mut monitors as *mut Vec<MonitorInfo> as isize;
    unsafe {
        let _ = EnumDisplayMonitors(HDC::default(), None, Some(monitor_proc), LPARAM(ptr));
    }
    monitors
}

unsafe extern "system" fn monitor_proc(
    hmon: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam.0 as *mut Vec<MonitorInfo>);
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let ok = GetMonitorInfoW(hmon, &mut info as *mut _ as *mut MONITORINFO).as_bool();
    if ok {
        let device = String::from_utf16_lossy(
            &info.szDevice[..info
                .szDevice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(info.szDevice.len())],
        );
        let r = info.monitorInfo.rcMonitor;
        monitors.push(MonitorInfo {
            device,
            rect: [r.left, r.top, r.right, r.bottom],
        });
    }
    TRUE
}

/// Build a HWND newtype to ferry an opaque handle across modules. Not actually a window,
/// just a wrapper so other modules don't need the windows crate in scope.
pub fn hwnd_from_isize(v: isize) -> HWND {
    HWND(v as *mut _)
}
