// Embed a Windows manifest declaring Common Controls v6 + DPI awareness so the
// TaskDialogIndirect API used by `rfd` resolves at load time.

#[cfg(target_os = "windows")]
fn main() {
    use embed_manifest::{embed_manifest, new_manifest};
    embed_manifest(new_manifest("ApolloFleet"))
        .expect("embed manifest");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
