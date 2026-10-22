//! Read-only executable-path evidence. No title/name heuristics or process control.
use crate::programs::InstalledProgram;
use anyhow::{bail, Context, Result};

fn normalized(value: &str) -> Option<String> {
    let path = value.trim().trim_matches('"').replace('/', "\\");
    let bytes = path.as_bytes();
    if bytes.len() < 4 || !bytes[0].is_ascii_alphabetic() || &bytes[1..3] != b":\\" {
        return None;
    }
    let path = path.trim_end_matches('\\');
    if path[3..].split('\\').any(|s| {
        s.is_empty()
            || s == "."
            || s == ".."
            || s.ends_with(['.', ' '])
            || s.chars().any(|c| c < ' ' || "<>:\"|?*".contains(c))
    }) {
        return None;
    }
    Some(path.to_lowercase())
}

/// Require exactly one specific, absolute install-folder relationship.
/// Nested registrations remain ambiguous rather than picking an arbitrary owner.
pub fn unique_program<'a>(
    programs: &'a [InstalledProgram],
    image: &str,
) -> Option<&'a InstalledProgram> {
    let image = normalized(image)?;
    let basename = image.rsplit('\\').next()?;
    if matches!(
        basename,
        "node.exe"
            | "python.exe"
            | "pythonw.exe"
            | "java.exe"
            | "javaw.exe"
            | "dotnet.exe"
            | "rundll32.exe"
            | "dllhost.exe"
            | "applicationframehost.exe"
            | "msedgewebview2.exe"
    ) {
        return None;
    }
    let mut matches = programs.iter().filter(|program| {
        let Some(location) = normalized(&program.install_location) else {
            return false;
        };
        if location[3..].split('\\').count() < 2 {
            return false;
        }
        // Shared Windows and common application roots are never program ownership.
        let suffix = &location[3..];
        if suffix.starts_with("windows\\")
            || matches!(
                suffix,
                "program files\\common files"
                    | "program files (x86)\\common files"
                    | "program files\\windowsapps"
            )
        {
            return false;
        }
        image
            .strip_prefix(&location)
            .is_some_and(|rest| rest.starts_with('\\'))
    });
    let first = matches.next()?;
    matches.next().is_none().then_some(first)
}

#[cfg(windows)]
pub fn foreground_image() -> Result<String> {
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        System::Threading::{
            OpenProcess, QueryFullProcessImageNameW, PROCESS_QUERY_LIMITED_INFORMATION,
        },
        UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId},
    };
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            bail!("No active window was available. Activate the intended window and try again.");
        }
        let mut pid = 0;
        GetWindowThreadProcessId(window, &mut pid);
        if pid == 0 || pid == std::process::id() {
            bail!("No other application window selected. Switch to its window during the countdown, then return here.");
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return Err(std::io::Error::last_os_error()).context("Cannot inspect this process. Protected or closed windows may be unavailable; choose another window.");
        }
        let mut path = vec![0u16; 32768];
        let mut length = path.len() as u32;
        let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut length);
        let error = std::io::Error::last_os_error();
        CloseHandle(process);
        if ok == 0 {
            return Err(error)
                .context("Cannot read the selected process path; it may have closed.");
        }
        let mut current_pid = 0;
        if GetForegroundWindow() != window
            || GetWindowThreadProcessId(window, &mut current_pid) == 0
            || current_pid != pid
        {
            bail!("The active window changed during inspection. Try again.");
        }
        Ok(String::from_utf16(&path[..length as usize])
            .context("Process path contains invalid Unicode")?)
    }
}

#[cfg(not(windows))]
pub fn foreground_image() -> Result<String> {
    bail!("Window identification is available on Windows only.")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app(location: &str) -> InstalledProgram {
        InstalledProgram {
            install_location: location.into(),
            ..Default::default()
        }
    }
    #[test]
    fn unique_specific_paths_only() {
        let list = [app("\"C:/Apps/Acme/\"")];
        assert!(unique_program(&list, "c:\\apps\\ACME\\bin\\tool.exe").is_some());
        for image in [
            "C:\\Apps\\AcmeOther\\a.exe",
            "C:\\Apps\\Acme",
            "C:\\Apps\\Acme\\..\\other.exe",
            "relative.exe",
            "\\\\server\\tool.exe",
        ] {
            assert!(unique_program(&list, image).is_none(), "{image}");
        }
    }
    #[test]
    fn shared_nested_and_broad_roots_do_not_match() {
        for root in [
            "",
            "C:\\",
            "C:\\Apps",
            "C:\\Program Files",
            "C:\\Windows\\System32",
            "C:\\Program Files\\Common Files",
            "C:\\Apps\\..\\Acme",
            "C:\\Apps\\Acme.",
        ] {
            assert!(
                unique_program(&[app(root)], &format!("{root}\\a.exe")).is_none(),
                "{root}"
            );
        }
        assert!(unique_program(
            &[app("C:\\Apps\\Acme"), app("C:\\Apps\\Acme")],
            "C:\\Apps\\Acme\\a.exe"
        )
        .is_none());
        assert!(unique_program(
            &[app("C:\\Apps\\Acme"), app("C:\\Apps\\Acme\\Child")],
            "C:\\Apps\\Acme\\Child\\a.exe"
        )
        .is_none());
    }
    #[test]
    fn known_hosts_and_name_only_evidence_remain_unknown() {
        let list = [app("C:\\Apps\\Acme")];
        for host in [
            "node.exe",
            "python.exe",
            "javaw.exe",
            "dotnet.exe",
            "msedgewebview2.exe",
        ] {
            assert!(unique_program(&list, &format!("C:\\Apps\\Acme\\{host}")).is_none());
        }
        let list = [InstalledProgram {
            name: "Acme".into(),
            display_icon: "C:\\Apps\\Acme\\tool.exe".into(),
            ..Default::default()
        }];
        assert!(unique_program(&list, "C:\\Apps\\Acme\\tool.exe").is_none());
    }
}
