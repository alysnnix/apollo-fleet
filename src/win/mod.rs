pub mod audio;
pub mod monitor;
pub mod process;
pub mod registry;
pub mod tcp;
pub mod tray_icon;
pub mod window;

use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

/// COM init for the current thread. Audio enumeration needs it; calling more than once is safe.
pub fn ensure_com_init() {
    unsafe {
        // S_OK or S_FALSE (already initialized) are both fine.
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    }
}
