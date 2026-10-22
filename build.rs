//! Embeds the icon, version information and application manifest into
//! SetupScope.exe when building for Windows.

fn main() {
    println!("cargo:rerun-if-changed=assets/setupscope.ico");
    println!("cargo:rerun-if-changed=assets/setupscope.manifest");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = format!("{}.0", env!("CARGO_PKG_VERSION"));
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/setupscope.ico")
        .set_manifest_file("assets/setupscope.manifest")
        .set("ProductName", "SetupScope")
        .set(
            "FileDescription",
            "SetupScope – Windows Desktop Development Toolkit",
        )
        .set("CompanyName", "Sean Shaffer")
        .set(
            "LegalCopyright",
            "Copyright © 2026 Sean Shaffer. Licensed under the MIT License.",
        )
        .set("OriginalFilename", "SetupScope.exe")
        .set("InternalName", "SetupScope")
        .set("ProductVersion", &version)
        .set("FileVersion", &version);
    if let Err(e) = res.compile() {
        panic!("failed to embed Windows resources: {e}");
    }
}
