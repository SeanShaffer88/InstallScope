//! Embeds the icon, version information and application manifest into
//! SetupSweep.exe when building for Windows.

fn main() {
    println!("cargo:rerun-if-changed=assets/setupsweep.ico");
    println!("cargo:rerun-if-changed=assets/setupsweep.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = env!("CARGO_PKG_VERSION");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/setupsweep.ico")
        .set_manifest_file("assets/setupsweep.manifest")
        .set("ProductName", "SetupSweep")
        .set(
            "FileDescription",
            "SetupSweep – Open Source Install Monitor & Uninstaller",
        )
        .set("CompanyName", "Sean Shaffer")
        .set(
            "LegalCopyright",
            "Copyright © 2026 Sean Shaffer. Licensed under the MIT License.",
        )
        .set("OriginalFilename", "SetupSweep.exe")
        .set("InternalName", "SetupSweep")
        .set("ProductVersion", version)
        .set("FileVersion", version);
    if let Err(e) = res.compile() {
        panic!("failed to embed Windows resources: {e}");
    }
}
