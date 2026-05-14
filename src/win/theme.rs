// Read the user's current system theme (light vs dark) from the registry. The taskbar
// and notification area follow `SystemUsesLightTheme`; `AppsUseLightTheme` is a separate
// (per-app-surface) value we don't care about for the tray.

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Theme {
    Light,
    Dark,
}

pub fn system_theme() -> Theme {
    match read_dword(
        HKEY_CURRENT_USER,
        r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize",
        "SystemUsesLightTheme",
    ) {
        Some(0) => Theme::Dark,
        Some(_) => Theme::Light,
        None => Theme::Light, // safe default on misread
    }
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn read_dword(hive: HKEY, subkey: &str, value: &str) -> Option<u32> {
    let mut key: HKEY = HKEY::default();
    let subkey_w = to_wide(subkey);
    let rc =
        unsafe { RegOpenKeyExW(hive, PCWSTR(subkey_w.as_ptr()), 0, KEY_READ, &mut key) };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let value_w = to_wide(value);
    let mut ty = REG_VALUE_TYPE(0);
    let mut buf = [0u8; 4];
    let mut size: u32 = buf.len() as u32;
    let rc = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(value_w.as_ptr()),
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr()),
            Some(&mut size),
        )
    };
    unsafe {
        let _ = RegCloseKey(key);
    }
    if rc != ERROR_SUCCESS || size < 4 {
        return None;
    }
    Some(u32::from_le_bytes(buf))
}
