// Thin wrapper around MessageBoxW. Replaces `rfd::MessageDialog` so we don't pull rfd
// (and its TaskDialogIndirect plumbing) just for two one-off dialogs.

use windows::core::PCWSTR;
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MESSAGEBOX_STYLE,
};

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn info(title: &str, body: &str) {
    show(title, body, MB_OK | MB_ICONINFORMATION);
}

pub fn error(title: &str, body: &str) {
    show(title, body, MB_OK | MB_ICONERROR);
}

fn show(title: &str, body: &str, style: MESSAGEBOX_STYLE) {
    let title_w = to_wide(title);
    let body_w = to_wide(body);
    unsafe {
        MessageBoxW(
            HWND::default(),
            PCWSTR(body_w.as_ptr()),
            PCWSTR(title_w.as_ptr()),
            style,
        );
    }
}
