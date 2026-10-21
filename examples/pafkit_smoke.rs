//! Windows-only integration: compile a real official-source launcher and the
//! InstallScope installer, then exercise launch, relocation and Data-preserving upgrade.
use anyhow::{anyhow, Context, Result};
use installscope_core::{
    pafbuild::{self, AppDetails, PaTools},
    util,
};
use portable_builder_kit::process::{run_tool, ToolRequest};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

struct Work(PathBuf);
impl Drop for Work {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn run(exe: &Path, arguments: Vec<std::ffi::OsString>) -> Result<()> {
    println!("Running {}", exe.display());
    run_tool(
        &ToolRequest {
            executable: exe.to_path_buf(),
            arguments,
            working_directory: None,
            timeout: Duration::from_secs(60),
        },
        &|| false,
    )
    .with_context(|| format!("running {}", exe.display()))?;
    Ok(())
}
fn main() -> Result<()> {
    if !cfg!(windows) {
        return Err(anyhow!("This integration test must run on Windows"));
    }
    let payload = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .context("provide the test-tool executable path")?,
    );
    let work = Work(std::env::temp_dir().join(format!("pafkit smoke 工具 {}", util::new_id())));
    std::fs::create_dir(&work.0)?;
    let package = work.0.join("build/ProbePortable");
    for dir in [
        "App/Probe",
        "App/AppInfo/Launcher",
        "App/DefaultData",
        "Data",
        "Other/Source",
    ] {
        std::fs::create_dir_all(package.join(dir))?;
    }
    std::fs::copy(payload, package.join("App/Probe/probe.exe"))?;
    let mut details = AppDetails {
        name: "Probe".into(),
        publisher: "SDK integration test".into(),
        version: "1.0".into(),
        ..Default::default()
    };
    std::fs::write(
        package.join("App/AppInfo/appinfo.ini"),
        pafbuild::appinfo_ini(&details),
    )?;
    std::fs::write(
        package.join("help.html"),
        "<!doctype html><title>Build test</title>",
    )?;
    std::fs::write(package.join("App/AppInfo/Launcher/ProbePortable.ini"),"[Launch]\r\nProgramExecutable=Probe\\probe.exe\r\nCommandLineArguments=touch \"%PAL:DataDir%\\probe.txt\"\r\nDirectoryMoveOK=yes\r\nSupportsUNC=yes\r\n")?;
    let compiled = pafbuild::compile(
        &package,
        "ProbePortable",
        &PaTools::default(),
        &work.0.join("data"),
        &|s| println!("{s}"),
    )?;
    let installer = compiled.paf.context("installer missing")?;
    portable_builder_kit::validate_executable(&installer)?;
    let destination = work.0.join("installed");
    std::fs::create_dir(&destination)?;
    run(
        &installer,
        vec![
            "/S".into(),
            format!("/DESTINATION={}", destination.display()).into(),
        ],
    )?;
    let installed = destination.join("ProbePortable");
    if installscope_core::patools::ini_value(
        &installed.join("App/AppInfo/appinfo.ini"),
        "Version",
        "PackageVersion",
    )
    .as_deref()
        != Some("1.0.0.0")
    {
        return Err(anyhow!("initial install has wrong package version"));
    }
    let launcher = installed.join("ProbePortable.exe");
    run(&launcher, vec![])?;
    if std::fs::read(installed.join("Data/probe.txt"))? != b"started" {
        return Err(anyhow!("launcher failed to create portable data"));
    }
    std::fs::write(
        installed.join("Data/keep.txt"),
        b"user settings survive upgrade",
    )?;
    let relocated = work.0.join("relocated");
    std::fs::create_dir(&relocated)?;
    std::fs::rename(&installed, relocated.join("ProbePortable"))?;
    let installed = relocated.join("ProbePortable");
    std::fs::remove_file(installed.join("Data/probe.txt"))?;
    run(&installed.join("ProbePortable.exe"), vec![])?;
    if !installed.join("Data/probe.txt").is_file() {
        return Err(anyhow!("relocated launcher failed"));
    }
    std::fs::write(
        installed.join("App/Probe/obsolete.txt"),
        b"old application file",
    )?;
    std::fs::create_dir_all(installed.join("Other/Source"))?;
    std::fs::write(
        installed.join("Other/Source/obsolete.txt"),
        b"old source file",
    )?;
    std::fs::write(
        package.join("Other/Source/new-source.txt"),
        b"new source file",
    )?;
    details.version = "1.0.1".into();
    std::fs::write(
        package.join("App/AppInfo/appinfo.ini"),
        pafbuild::appinfo_ini(&details),
    )?;
    let compiled = pafbuild::compile(
        &package,
        "ProbePortable",
        &PaTools::default(),
        &work.0.join("data"),
        &|s| println!("upgrade: {s}"),
    )?;
    run(
        &compiled.paf.context("upgrade installer missing")?,
        vec![
            "/S".into(),
            format!("/DESTINATION={}", relocated.display()).into(),
        ],
    )?;
    if std::fs::read(installed.join("Data/keep.txt"))? != b"user settings survive upgrade" {
        return Err(anyhow!("upgrade lost user data"));
    }
    if installscope_core::patools::ini_value(
        &installed.join("App/AppInfo/appinfo.ini"),
        "Version",
        "PackageVersion",
    )
    .as_deref()
        != Some("1.0.1.0")
    {
        return Err(anyhow!("upgrade did not replace application metadata"));
    }
    if installed.join("App/Probe/obsolete.txt").exists()
        || installed.join("Other/Source/obsolete.txt").exists()
    {
        return Err(anyhow!("upgrade retained obsolete App/Other files"));
    }
    if std::fs::read(installed.join("Other/Source/new-source.txt"))? != b"new source file" {
        return Err(anyhow!("upgrade did not install new source file"));
    }
    std::fs::remove_file(installed.join("Data/probe.txt"))?;
    run(&installed.join("ProbePortable.exe"), vec![])?;
    if !installed.join("Data/probe.txt").is_file() {
        return Err(anyhow!("upgraded launcher failed"));
    }
    println!("Verified: real launcher compilation, installer build, silent install, portable launch, folder relocation and upgrade preserving Data (InstallScope installer; not the official Installer frontend)");
    Ok(())
}
