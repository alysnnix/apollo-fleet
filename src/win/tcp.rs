use anyhow::Result;
use windows::Win32::NetworkManagement::IpHelper::{GetTcpTable2, MIB_TCPROW2, MIB_TCPTABLE2};

/// `MIB_TCP_STATE_ESTAB` numerical value. Defined here to avoid pulling the
/// `MIB_TCP_STATE` enum through API renames; the value is stable in Windows.
const TCP_STATE_ESTAB: u32 = 5;
const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const NO_ERROR: u32 = 0;

/// Returns true if `port` has at least one ESTABLISHED TCP connection where neither end
/// is a loopback address.
pub fn has_external_client(port: u16) -> Result<bool> {
    let rows = list_established_rows()?;
    for row in rows {
        // dwLocalPort and dwRemotePort are network-byte-order DWORDs whose low 16 bits
        // hold the port. ntohs equivalent on the low half.
        let local_port = u16::from_be((row.dwLocalPort & 0xFFFF) as u16);
        if local_port != port {
            continue;
        }
        let local_addr = row.dwLocalAddr.to_le_bytes();
        let remote_addr = row.dwRemoteAddr.to_le_bytes();
        if is_loopback(local_addr) || is_loopback(remote_addr) {
            continue;
        }
        return Ok(true);
    }
    Ok(false)
}

fn is_loopback(octets: [u8; 4]) -> bool {
    // 127.0.0.0/8 + the wildcard 0.0.0.0 (Apollo's own listener binds to it).
    octets[0] == 127 || octets == [0, 0, 0, 0]
}

fn list_established_rows() -> Result<Vec<MIB_TCPROW2>> {
    let mut size: u32 = 0;
    let rc = unsafe { GetTcpTable2(None, &mut size as *mut u32, true) };
    if rc != ERROR_INSUFFICIENT_BUFFER && rc != NO_ERROR {
        return Err(anyhow::anyhow!("GetTcpTable2 sizing failed: {rc}"));
    }
    let mut buf = vec![0u8; size as usize];
    let table = buf.as_mut_ptr() as *mut MIB_TCPTABLE2;
    let rc = unsafe { GetTcpTable2(Some(table), &mut size as *mut u32, true) };
    if rc != NO_ERROR {
        return Err(anyhow::anyhow!("GetTcpTable2 failed: {rc}"));
    }
    let header = unsafe { &*table };
    let entries = header.dwNumEntries as usize;
    let mut out = Vec::with_capacity(entries);
    let first = unsafe {
        (table as *const u8).add(std::mem::size_of::<u32>()) as *const MIB_TCPROW2
    };
    for i in 0..entries {
        let row = unsafe { &*first.add(i) };
        if row.dwState == TCP_STATE_ESTAB {
            out.push(*row);
        }
    }
    Ok(out)
}
