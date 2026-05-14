// Native Windows credential prompt. Replaces a previous PowerShell + VBA
// InputBox shell-out which was slow, ugly, and showed the password in cleartext.

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, WIN32_ERROR};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::Security::Credentials::{
    CredUIPromptForCredentialsW, CREDUI_FLAGS_DO_NOT_PERSIST, CREDUI_FLAGS_GENERIC_CREDENTIALS,
    CREDUI_INFOW,
};

const BUFLEN: usize = 256;

pub fn prompt(title: &str, message: &str) -> Option<(String, String)> {
    let title_w: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let msg_w: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();
    let target_w: Vec<u16> = "apollo-fleet"
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let ui_info = CREDUI_INFOW {
        cbSize: std::mem::size_of::<CREDUI_INFOW>() as u32,
        hwndParent: HWND::default(),
        pszMessageText: PCWSTR(msg_w.as_ptr()),
        pszCaptionText: PCWSTR(title_w.as_ptr()),
        hbmBanner: HBITMAP::default(),
    };

    let mut user_buf = [0u16; BUFLEN];
    let mut pass_buf = [0u16; BUFLEN];

    let rc = unsafe {
        CredUIPromptForCredentialsW(
            Some(&ui_info),
            PCWSTR(target_w.as_ptr()),
            None,
            0,
            PWSTR(user_buf.as_mut_ptr()),
            BUFLEN as u32,
            PWSTR(pass_buf.as_mut_ptr()),
            BUFLEN as u32,
            None,
            CREDUI_FLAGS_GENERIC_CREDENTIALS | CREDUI_FLAGS_DO_NOT_PERSIST,
        )
    };

    if WIN32_ERROR(rc) != ERROR_SUCCESS {
        return None;
    }

    let user_len = user_buf.iter().position(|&c| c == 0).unwrap_or(BUFLEN);
    let pass_len = pass_buf.iter().position(|&c| c == 0).unwrap_or(BUFLEN);
    let user = String::from_utf16_lossy(&user_buf[..user_len]);
    let pass = String::from_utf16_lossy(&pass_buf[..pass_len]);

    // Best-effort wipe of the password from local memory after we've copied it.
    pass_buf.fill(0);

    if user.is_empty() || pass.is_empty() {
        return None;
    }
    Some((user, pass))
}
