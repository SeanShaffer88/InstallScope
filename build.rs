//! Embeds the icon, version information and application manifest into
//! InstallScope.exe when building for Windows.

fn main() {
    println!("cargo:rerun-if-changed=assets/installscope.ico");
    println!("cargo:rerun-if-changed=assets/installscope.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = env!("CARGO_PKG_VERSION");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/installscope.ico")
        .set_manifest_file("assets/installscope.manifest")
        .set("ProductName", "InstallScope")
        .set(
            "FileDescription",
            "InstallScope – Open Source Install Monitor & Uninstaller",
        )
        .set("CompanyName", "Sean Shaffer")
        .set(
            "LegalCopyright",
            "Copyright © 2026 Sean Shaffer. Licensed under GPLv3.",
        )
        .set("OriginalFilename", "InstallScope.exe")
        .set("InternalName", "InstallScope")
        .set("ProductVersion", version)
        .set("FileVersion", version);
    if let Err(e) = res.compile() {
        panic!("failed to embed Windows resources: {e}");
    }
}
