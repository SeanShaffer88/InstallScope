//! Windows-only integration with explicitly supplied official frontends.
//! Exercises build, install, launch, relocation and Data-preserving upgrade.
use anyhow::{anyhow, Context, Result};
use installscope_core::{
    pafbuild::{self, PaTools},
    pafbuildsession::Review,
    pafproject::Project,
    pafstage::{Prepared, StagePlan},
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
        executable: std::fs::canonicalize(
            if arguments.first().is_some_and(|s| s == "nsis-install") {
                Path::new(
                    arguments
                        .get(1)
                        .context("NSIS test invocation lacks an installer")?,
                )
            } else {
                exe
            },
        )?,
        reported: String::new(),
        reports: 0,
        install: if arguments.first().is_some_and(|s| s == "nsis-install") {
            Some((
                portable_builder_kit::process::resolve_tool_path(
                    arguments
                        .get(2)
                        .context("NSIS test invocation lacks a destination")?,
                )?
                .join("ProbePortable"),
                0,
            ))
        } else {
            None
        },
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
            {
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
    reported: String,
    reports: usize,
    install: Option<(PathBuf, u8)>,
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
        if text.len() >= 64 * 1024 {
            return 0;
        }
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
    if let Some((destination, phase)) = &mut state.install {
        let button = GetDlgItem(window, 1);
        let mut caption = [0u16; 64];
        let mut count = 0usize;
        if !button.is_null()
            && IsWindowEnabled(button) != 0
            && SendMessageTimeoutW(
                button,
                WM_GETTEXT,
                caption.len(),
                caption.as_mut_ptr() as isize,
                SMTO_ABORTIFHUNG | SMTO_BLOCK,
                100,
                &mut count,
            ) != 0
            && count < caption.len()
        {
            let caption = String::from_utf16_lossy(&caption[..count]).replace('&', "");
            let advance =
                match *phase {
                    0 => caption.trim() == "Next >"
                        && text.contains(
                            "This wizard will guide you through the installation of Probe Portable",
                        ),
                    1 => {
                        caption.trim() == "Install"
                            && text.contains("Choose Install Location")
                            && text
                                .to_lowercase()
                                .contains(&destination.to_string_lossy().to_lowercase())
                    }
                    2 => {
                        caption.trim() == "Finish"
                            && text.contains("Probe Portable")
                            && destination.join("ProbePortable.exe").is_file()
                            && destination.join("App/AppInfo/appinfo.ini").is_file()
                    }
                    _ => false,
                };
            if advance && PostMessageW(button, BM_CLICK, 0, 0) != 0 {
                println!(
                    "Fixture installer wizard: confirmed phase {} at {}",
                    phase,
                    destination.display()
                );
                *phase += 1;
            }
        }
    }
    if state.reported != text && state.reports < 10 {
        println!("Fixture dialog from {}: {text}", state.executable.display());
        state.reported = text;
        state.reports += 1;
    }
    1
}

fn compile(prepared: &Prepared, project: &Project, _tools: &PaTools) -> Result<pafbuild::Compiled> {
    #[cfg(windows)]
    let tools = _tools;
    #[cfg(windows)]
    let dialogs = {
        let mut states = Vec::new();
        for path in [
            tools
                .launcher_generator
                .as_ref()
                .context("Generator missing")?,
            tools.installer.as_ref().context("Installer missing")?,
        ] {
            states.push(FixtureDialog {
                executable: std::fs::canonicalize(path)?,
                reported: String::new(),
                reports: 0,
                install: None,
            });
        }
        std::cell::RefCell::new(states)
    };
    let started = std::time::Instant::now();
    let review = Review::create(prepared, project, &|| false)?;
    let attempt = review.run(
        false,
        &|| {
            #[cfg(windows)]
            for state in dialogs.borrow_mut().iter_mut() {
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows(
                        Some(fixture_dialog),
                        state as *mut FixtureDialog as isize,
                    );
                }
            }
            started.elapsed() > Duration::from_secs(120)
        },
        &|s| println!("{s}"),
    )?;
    println!("Build journal: {}", attempt.journal.display());
    for warning in &attempt.warnings {
        println!("Build-log warning: {warning}");
    }
    let compiled = attempt.result.map_err(|e| anyhow!(e))?;
    if attempt.logs.len() != 2
        || attempt.validation_report.is_none()
        || attempt.validation_errors != Some(0)
    {
        return Err(anyhow!(
            "Expected two fresh compiler logs and a passing partial static report; inspect {}",
            attempt.directory.display()
        ));
    }
    println!("Retained two official compiler logs and partial validation report");
    Ok(compiled)
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
    let source = work.0.join("source");
    let workspace = work.0.join("workspace");
    std::fs::create_dir(&source)?;
    std::fs::create_dir(&workspace)?;
    std::fs::copy(&payload, source.join("probe.exe"))?;
    let icon = work.0.join("icon.png");
    std::fs::write(
        &icon,
        installscope_core::png::encode_rgba(256, 256, &vec![255; 256 * 256 * 4])
            .context("encode fixture icon")?,
    )?;
    let mut project=Project {
        source: source.to_string_lossy().into(), workspace: workspace.to_string_lossy().into(),
        app_id:"ProbePortable".into(), name:"Probe Portable".into(),publisher:"InstallScope test fixture".into(),
        homepage:"https://github.com/SeanShaffer88/InstallScope".into(),description:"Harmless packaging lifecycle fixture".into(),
        category:"Utilities".into(),language:"English".into(),package_version:"1.0.0.0".into(),display_version:"1.0".into(),
        program:"Probe/probe.exe".into(),arguments:r#"touch "%PAL:DataDir%\probe.txt""#.into(),directory_move:"yes".into(),
        portability_notes:"Fixture writes only its explicitly supplied Data path. Verify folder relocation and Data-preserving upgrade below.".into(),
        icon:icon.to_string_lossy().into(),generator:tools.launcher_generator.as_ref().context("Generator missing")?.to_string_lossy().into(),installer:tools.installer.as_ref().context("Installer missing")?.to_string_lossy().into(),
        rights:Some(true),open_source:Some(true),freeware:Some(true),commercial_use:Some(true),shareable:Some(true),..Default::default()
    };
    let prepared = StagePlan::create(&project, &|| false, &|s| println!("Prepare: {s}"))?
        .prepare(&|| false, &|s| println!("Prepare: {s}"))?;
    let compiled = compile(&prepared, &project, &tools)?;
    let installer = compiled.paf.context("installer missing")?;
    portable_builder_kit::validate_executable(&installer)?;
    let destination = work.0.join("installed");
    std::fs::create_dir(&destination)?;
    run(
        &payload,
        vec![
            "nsis-install".into(),
            installer.as_os_str().to_os_string(),
            destination.as_os_str().to_os_string(),
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
    project.package_version = "1.0.1.0".into();
    project.display_version = "1.0.1".into();
    let prepared = StagePlan::create(&project, &|| false, &|s| println!("Upgrade prepare: {s}"))?
        .prepare(&|| false, &|s| println!("Upgrade prepare: {s}"))?;
    std::fs::write(
        prepared.package.join("Other/Source/new-source.txt"),
        b"new source file",
    )?;
    let compiled = compile(&prepared, &project, &tools)?;
    run(
        &payload,
        vec![
            "nsis-install".into(),
            compiled
                .paf
                .context("upgrade installer missing")?
                .into_os_string(),
            relocated.as_os_str().to_os_string(),
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
    println!("Verified: wizard project staging/reconnect/build review, official launcher/installer build, log/report retention, installer wizard, portable launch, folder relocation and upgrade preserving Data (official frontends supplied by the test environment; not certification)");
    work.1 = false;
    Ok(())
}
