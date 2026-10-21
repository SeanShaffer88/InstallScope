//! Windows-only integration with explicitly supplied official frontends.
//! Exercises build, install, launch, relocation and Data-preserving upgrade.
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

struct Work(PathBuf, bool);
impl Drop for Work {
    fn drop(&mut self) {
        if self.1 {
            eprintln!(
                "Failed fixture retained for diagnosis: {}",
                self.0.display()
            );
        } else {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
fn run(exe: &Path, arguments: Vec<std::ffi::OsString>) -> Result<()> {
    println!("Running {}", exe.display());
    #[cfg(windows)]
    let dialog = std::cell::RefCell::new(FixtureDialog {
        executable: std::fs::canonicalize(exe)?,
        reported: false,
    });
    run_tool(
        &ToolRequest {
            executable: exe.to_path_buf(),
            arguments,
            working_directory: None,
            timeout: Duration::from_secs(60),
        },
        &|| {
            #[cfg(windows)]
            if exe.file_name().is_some_and(|n| n == "ProbePortable.exe") {
                // Report this fixture's own dialog only; never dismiss warnings.
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows(
                        Some(fixture_dialog),
                        &mut *dialog.borrow_mut() as *mut FixtureDialog as isize,
                    );
                }
            }
            false
        },
    )
    .with_context(|| format!("running {}", exe.display()))?;
    Ok(())
}
#[cfg(windows)]
struct FixtureDialog {
    executable: PathBuf,
    reported: bool,
}

#[cfg(windows)]
unsafe extern "system" fn fixture_dialog(
    window: windows_sys::Win32::Foundation::HWND,
    data: isize,
) -> i32 {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::*,
    };
    let state = &mut *(data as *mut FixtureDialog);
    let mut class = [0u16; 64];
    let count = GetClassNameW(window, class.as_mut_ptr(), class.len() as i32);
    if count <= 0 || String::from_utf16_lossy(&class[..count as usize]) != "#32770" {
        return 1;
    }
    let mut pid = 0;
    GetWindowThreadProcessId(window, &mut pid);
    let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
    if process.is_null() {
        return 1;
    }
    let mut image = vec![0u16; 32768];
    let mut size = image.len() as u32;
    let ok = QueryFullProcessImageNameW(process, 0, image.as_mut_ptr(), &mut size);
    CloseHandle(process);
    if ok == 0 {
        return 1;
    }
    let image = PathBuf::from(std::ffi::OsString::from_wide(&image[..size as usize]));
    if std::fs::canonicalize(image).ok().as_ref() != Some(&state.executable) {
        return 1;
    }
    unsafe extern "system" fn child_text(
        window: windows_sys::Win32::Foundation::HWND,
        data: isize,
    ) -> i32 {
        let text = &mut *(data as *mut String);
        let mut buffer = [0u16; 4096];
        let mut count = 0usize;
        let delivered = windows_sys::Win32::UI::WindowsAndMessaging::SendMessageTimeoutW(
            window,
            WM_GETTEXT,
            buffer.len(),
            buffer.as_mut_ptr() as isize,
            SMTO_ABORTIFHUNG | SMTO_BLOCK,
            100,
            &mut count,
        );
        if delivered != 0 && count > 0 && count < buffer.len() {
            text.push_str(&String::from_utf16_lossy(&buffer[..count as usize]));
            text.push(' ');
        }
        1
    }
    let mut text = String::new();
    EnumChildWindows(window, Some(child_text), &mut text as *mut String as isize);
    if !state.reported {
        println!("Fixture launcher dialog: {text}");
        state.reported = true;
    }
    1
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
    let tools = PaTools {
        launcher_generator: Some(
            std::env::var_os("PAF_TEST_GENERATOR")
                .context("Set PAF_TEST_GENERATOR to the verified official frontend")?
                .into(),
        ),
        installer: Some(
            std::env::var_os("PAF_TEST_INSTALLER")
                .context("Set PAF_TEST_INSTALLER to the verified official frontend")?
                .into(),
        ),
        ..Default::default()
    };
    let mut work = Work(
        std::env::temp_dir().join(format!("pafkit smoke 工具 {}", util::new_id())),
        true,
    );
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
        open_source: true,
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
    let png = installscope_core::png::encode_rgba(256, 256, &vec![255; 256 * 256 * 4])
        .context("encode fixture icon")?;
    for (name, bytes) in installscope_core::pafassets::from_png(&png, &|| false)?.files {
        std::fs::write(package.join("App/AppInfo").join(name), bytes)?;
    }
    let compiled = pafbuild::compile_official_with_cancel(
        &package,
        "ProbePortable",
        &tools,
        false,
        &|s| println!("{s}"),
        &|| false,
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
    let installed_version = installscope_core::patools::ini_value(
        &installed.join("App/AppInfo/appinfo.ini"),
        "Version",
        "PackageVersion",
    );
    if installed_version.as_deref() != Some("1.0.0.0") {
        return Err(anyhow!(
            "initial install at {} has version {:?}; expected 1.0.0.0",
            installed.display(),
            installed_version
        ));
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
    let compiled = pafbuild::compile_official_with_cancel(
        &package,
        "ProbePortable",
        &tools,
        false,
        &|s| println!("upgrade: {s}"),
        &|| false,
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
    println!("Verified: real launcher compilation, installer build, silent install, portable launch, folder relocation and upgrade preserving Data (official frontends supplied by the test environment; not certification)");
    work.1 = false;
    Ok(())
}
