use anyhow::Result;
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, NO_ERROR, WIN32_ERROR};
use windows::Win32::NetworkManagement::IpHelper::{
    GetTcpTable2, MIB_TCPROW2, MIB_TCPTABLE2, MIB_TCP_STATE_ESTAB,
};

/// Returns true if `port` has at least one ESTABLISHED TCP connection where neither end
/// is a loopback address.
pub fn has_external_client(port: u16) -> Result<bool> {
    let rows = list_established_rows()?;
    let port_be = u16::from_be(port).to_be(); // keep semantics explicit
    let _ = port_be;
    for row in rows {
        // dwLocalPort and dwRemotePort are big-endian DWORDs.
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
    // 127.0.0.0/8 + the wildcard 0.0.0.0 (which Apollo's own listener uses).
    octets[0] == 127 || octets == [0, 0, 0, 0]
}

fn list_established_rows() -> Result<Vec<MIB_TCPROW2>> {
    let mut size: u32 = 0;
    unsafe {
        let rc = GetTcpTable2(None, &mut size as *mut u32, true.into());
        let err = WIN32_ERROR(rc);
        if err != ERROR_INSUFFICIENT_BUFFER && err != NO_ERROR {
            return Err(anyhow::anyhow!("GetTcpTable2 sizing failed: {rc}"));
        }
    }
    let mut buf = vec![0u8; size as usize];
    let table = buf.as_mut_ptr() as *mut MIB_TCPTABLE2;
    let rc = unsafe { GetTcpTable2(Some(table), &mut size as *mut u32, true.into()) };
    if WIN32_ERROR(rc) != NO_ERROR {
        return Err(anyhow::anyhow!("GetTcpTable2 failed: {rc}"));
    }
    let header = unsafe { &*table };
    let entries = header.dwNumEntries as usize;
    let mut out = Vec::with_capacity(entries);
    let first = unsafe { (table as *const u8).add(std::mem::size_of::<u32>()) as *const MIB_TCPROW2 };
    for i in 0..entries {
        let row = unsafe { &*first.add(i) };
        if row.State == MIB_TCP_STATE_ESTAB {
            out.push(*row);
        }
    }
    Ok(out)
}
