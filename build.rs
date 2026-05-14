// Embed a Windows manifest (Common Controls v6, required by rfd's TaskDialogIndirect)
// and the executable icon shown by Explorer / Alt-Tab / taskbar.

#[cfg(target_os = "windows")]
fn main() {
    use embed_manifest::{embed_manifest, new_manifest};
    embed_manifest(new_manifest("ApolloFleet")).expect("embed manifest");

    let mut res = winresource::WindowsResource::new();
    res.set_icon("resources/apollo.ico");
    res.compile().expect("compile windows resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
