use std::collections::HashMap;

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};

/// Return `root_pid` plus every PID whose parent chain leads back to it.
pub fn descendant_pids(root_pid: u32) -> Vec<u32> {
    let mut parents: HashMap<u32, u32> = HashMap::new();
    unsafe {
        let snap = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
            Ok(h) => h,
            Err(_) => return vec![root_pid],
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                parents.insert(entry.th32ProcessID, entry.th32ParentProcessID);
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }

    let mut result = vec![root_pid];
    let mut changed = true;
    while changed {
        changed = false;
        for (&child, &parent) in &parents {
            if result.contains(&parent) && !result.contains(&child) {
                result.push(child);
                changed = true;
            }
        }
    }
    result
}
