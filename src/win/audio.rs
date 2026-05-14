use anyhow::{Context, Result};
use windows::core::PCWSTR;
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::{
    eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};

use super::ensure_com_init;

pub fn list_audio_endpoints() -> Result<Vec<String>> {
    ensure_com_init();
    unsafe {
        let enumerator: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
            .context("CoCreateInstance(MMDeviceEnumerator)")?;
        let collection = enumerator
            .EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)
            .context("EnumAudioEndpoints")?;
        let count = collection.GetCount().context("GetCount")?;
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let device = collection.Item(i).context("Item")?;
            let props = device.OpenPropertyStore(STGM_READ).context("OpenPropertyStore")?;
            let value = props.GetValue(&PKEY_Device_FriendlyName).context("GetValue")?;
            let name = prop_to_string(&value);
            if !name.is_empty() {
                out.push(name);
            }
        }
        Ok(out)
    }
}

unsafe fn prop_to_string(value: &windows::Win32::System::Com::StructuredStorage::PROPVARIANT) -> String {
    use windows::Win32::System::Variant::{VT_BSTR, VT_LPWSTR};
    let vt = value.Anonymous.Anonymous.vt;
    if vt == VT_LPWSTR {
        let ptr = value.Anonymous.Anonymous.Anonymous.pwszVal;
        return pcwstr_to_string(ptr);
    }
    if vt == VT_BSTR {
        let ptr = value.Anonymous.Anonymous.Anonymous.bstrVal.0;
        return pcwstr_to_string(PCWSTR(ptr));
    }
    String::new()
}

unsafe fn pcwstr_to_string(s: PCWSTR) -> String {
    if s.is_null() {
        return String::new();
    }
    s.to_string().unwrap_or_default()
}

pub fn validate_sinks(seats: &[crate::config::SeatCfg]) -> Vec<String> {
    let needed: Vec<(&str, &str)> = seats
        .iter()
        .filter(|s| !s.audio_sink.is_empty())
        .map(|s| (s.name.as_str(), s.audio_sink.as_str()))
        .collect();
    if needed.is_empty() {
        return Vec::new();
    }
    let endpoints = match list_audio_endpoints() {
        Ok(v) if !v.is_empty() => v,
        Ok(_) => return vec!["could not enumerate audio endpoints (empty list)".to_string()],
        Err(e) => return vec![format!("could not enumerate audio endpoints: {e:#}")],
    };
    let lower: Vec<String> = endpoints.iter().map(|e| e.to_lowercase()).collect();
    let mut errors = Vec::new();
    for (name, sink) in needed {
        let needle = sink.to_lowercase();
        if !lower.iter().any(|e| e.contains(&needle)) {
            errors.push(format!(
                "seat '{name}' wants audio_sink '{sink}' -- not found among active endpoints"
            ));
        }
    }
    errors
}
