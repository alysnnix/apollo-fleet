use std::path::{Path, PathBuf};

use windows::core::PCWSTR;
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
    KEY_READ, REG_VALUE_TYPE,
};

pub fn find_steam_install() -> Option<PathBuf> {
    let candidates = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Valve\Steam"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Valve\Steam"),
        (HKEY_CURRENT_USER, r"SOFTWARE\Valve\Steam"),
    ];
    for (hive, sub) in candidates {
        if let Some(install_dir) = read_string(hive, sub, "InstallPath") {
            let exe = Path::new(&install_dir).join("steam.exe");
            if exe.exists() {
                return Some(exe);
            }
        }
    }
    for p in [
        Path::new(r"C:\Program Files (x86)\Steam\steam.exe"),
        Path::new(r"C:\Program Files\Steam\steam.exe"),
    ] {
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }
    None
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn read_string(hive: HKEY, subkey: &str, value: &str) -> Option<String> {
    let mut key: HKEY = HKEY::default();
    let subkey_w = to_wide(subkey);
    let rc = unsafe {
        RegOpenKeyExW(hive, PCWSTR(subkey_w.as_ptr()), 0, KEY_READ, &mut key)
    };
    if rc != ERROR_SUCCESS {
        return None;
    }
    let value_w = to_wide(value);
    let mut ty = REG_VALUE_TYPE(0);
    let mut size: u32 = 0;
    // Determine size.
    let rc = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(value_w.as_ptr()),
            None,
            Some(&mut ty),
            None,
            Some(&mut size),
        )
    };
    if rc != ERROR_SUCCESS || size == 0 {
        unsafe { let _ = RegCloseKey(key); }
        return None;
    }
    let mut buf = vec![0u8; size as usize];
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
    unsafe { let _ = RegCloseKey(key); }
    if rc != ERROR_SUCCESS {
        return None;
    }
    let wide: Vec<u16> = buf
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .take_while(|&c| c != 0)
        .collect();
    Some(String::from_utf16_lossy(&wide))
}
