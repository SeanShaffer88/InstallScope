//! Unattended uninstallation: work out how to run each program's uninstaller
//! without any windows or prompts, and without restarting Windows.
//!
//! Installer technologies each have their own switches:
//!
//! | Technology        | Recognised by                               | Silent switches                              |
//! |-------------------|---------------------------------------------|----------------------------------------------|
//! | Windows Installer | `WindowsInstaller`=1 / `MsiExec` + GUID     | `MsiExec /X{GUID} /qn /norestart`            |
//! | Vendor-declared   | `QuietUninstallString`                      | used as given                                |
//! | Inno Setup        | `unins000.exe`, key ending in `_is1`        | `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART`   |
//! | NSIS              | "Nullsoft" inside the uninstaller           | `/S _?=<folder>` (also makes it wait)        |
//! | WiX Burn bundle   | `/uninstall` from the Package Cache         | `/quiet /norestart`                          |
//! | Squirrel          | `Update.exe --uninstall`                    | `-s`                                         |

use crate::model::fold;
use crate::platform::{expand_env, Registry};
use crate::programs::{InstalledProgram, Source};
use crate::util::split_command_line;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Engine {
    Msi,
    Vendor,
    Inno,
    Nsis,
    Burn,
    Squirrel,
    /// Microsoft Store app: removed through Windows, always silent.
    Store,
    /// Unknown technology: its uninstaller has to be shown.
    Manual,
}
crate::json_enum!(Engine {
    Msi,
    Vendor,
    Inno,
    Nsis,
    Burn,
    Squirrel,
    Store,
    Manual
});

impl Engine {
    pub fn label(self) -> &'static str {
        match self {
            Engine::Msi => "Windows Installer",
            Engine::Vendor => "Silent mode from the publisher",
            Engine::Inno => "Inno Setup",
            Engine::Nsis => "NSIS",
            Engine::Burn => "WiX bundle",
            Engine::Squirrel => "Squirrel",
            Engine::Store => "Microsoft Store",
            Engine::Manual => "Needs its own window",
        }
    }

    pub fn is_silent(self) -> bool {
        self != Engine::Manual
    }
}

fn exe_name(exe: &str) -> String {
    fold(exe.rsplit(['\\', '/']).next().unwrap_or(exe))
}

fn is_inno_uninstaller(name: &str) -> bool {
    // unins000.exe, unins001.exe, …
    name.len() == 12
        && name.starts_with("unins")
        && name.ends_with(".exe")
        && name[5..8].chars().all(|c| c.is_ascii_digit())
}

/// Recognise the technology from the registry entry alone (no file access).
pub fn quick_engine(p: &InstalledProgram) -> Engine {
    if p.source == Source::Store {
        return Engine::Store;
    }
    if p.is_msi && !p.product_code.is_empty() {
        return Engine::Msi;
    }
    if !p.quiet_uninstall_string.trim().is_empty() {
        return Engine::Vendor;
    }
    let (exe, args) = split_command_line(&expand_env(&p.uninstall_string));
    let name = exe_name(&exe);
    let args_l = fold(&args);
    if name == "update.exe" && args_l.contains("--uninstall") {
        return Engine::Squirrel;
    }
    if args_l.contains("/uninstall") && fold(&exe).contains("package cache") {
        return Engine::Burn;
    }
    if is_inno_uninstaller(&name) || fold(&p.key).ends_with("_is1") {
        return Engine::Inno;
    }
    Engine::Manual
}

/// True if the file looks like an NSIS-built executable.
pub fn is_nsis_file(path: &str) -> bool {
    use std::io::Read;
    let Ok(f) = std::fs::File::open(path) else {
        return false;
    };
    // The marker sits in the stub's manifest and data header, well inside
    // the first couple of megabytes.
    let mut buf = Vec::with_capacity(1 << 20);
    if f.take(2 << 20).read_to_end(&mut buf).is_err() {
        return false;
    }
    buf.windows(8).any(|w| w == b"Nullsoft")
}

/// Full detection; may read the uninstaller file to recognise NSIS.
pub fn detect(p: &InstalledProgram) -> Engine {
    let e = quick_engine(p);
    if e != Engine::Manual {
        return e;
    }
    let (exe, _) = split_command_line(&expand_env(&p.uninstall_string));
    if !exe.is_empty() && is_nsis_file(&exe) {
        Engine::Nsis
    } else {
        Engine::Manual
    }
}

fn quote(s: &str) -> String {
    format!("\"{}\"", s.trim_matches('"'))
}

fn with_args(exe: &str, args: &str, extra: &str) -> String {
    let args = args.trim();
    if args.is_empty() {
        format!("{} {extra}", quote(exe))
    } else {
        format!("{} {args} {extra}", quote(exe))
    }
}

/// The command that removes the program without any user interaction, or
/// None for Store apps (removed through Windows) and manual-only programs.
pub fn silent_command(p: &InstalledProgram, engine: Engine) -> Option<String> {
    let (exe, args) = split_command_line(&expand_env(&p.uninstall_string));
    let args_l = fold(&args);
    match engine {
        Engine::Msi => Some(format!("MsiExec.exe /X{} /qn /norestart", p.product_code)),
        Engine::Vendor => Some(expand_env(&p.quiet_uninstall_string)),
        Engine::Inno => {
            let mut extra = String::new();
            for sw in ["/VERYSILENT", "/SUPPRESSMSGBOXES", "/NORESTART"] {
                if !args_l.contains(&fold(sw)) {
                    extra.push_str(sw);
                    extra.push(' ');
                }
            }
            Some(with_args(&exe, &args, extra.trim()))
        }
        Engine::Nsis => {
            // `_?=` runs the uninstaller in place (instead of a copy in %TEMP%)
            // so SetupScope can wait for it; it must come last. The uninstaller
            // file itself is then left behind and removed by the leftover scan.
            let dir = exe
                .trim_matches('"')
                .rsplit_once(['\\', '/'])
                .map(|(d, _)| d.to_string())
                .unwrap_or_default();
            let mut extra = String::new();
            if !args_l.split_whitespace().any(|a| a == "/s") {
                extra.push_str("/S ");
            }
            if !dir.is_empty() && !args_l.contains("_?=") {
                extra.push_str(&format!("_?={dir}"));
            }
            Some(with_args(&exe, &args, extra.trim()))
        }
        Engine::Burn => {
            let mut extra = String::new();
            if !args_l.contains("/quiet") && !args_l.contains("/silent") {
                extra.push_str("/quiet ");
            }
            if !args_l.contains("/norestart") {
                extra.push_str("/norestart");
            }
            Some(with_args(&exe, &args, extra.trim()))
        }
        Engine::Squirrel => {
            if args_l
                .split_whitespace()
                .any(|a| a == "-s" || a == "--silent")
            {
                Some(with_args(&exe, &args, ""))
            } else {
                Some(with_args(&exe, &args, "-s"))
            }
        }
        Engine::Store | Engine::Manual => None,
    }
}

/// What an uninstaller's exit code means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitMeaning {
    Success,
    /// Finished; Windows must restart to complete it.
    RebootRequired,
    /// Another installation is running; try again shortly.
    Busy,
    /// The user (or a policy) cancelled it.
    Cancelled,
    Failed,
}

pub fn exit_meaning(engine: Engine, code: u32) -> ExitMeaning {
    match (engine, code) {
        (_, 0) => ExitMeaning::Success,
        // Windows Installer codes, also used by Burn bundles.
        (_, 3010) | (_, 1641) => ExitMeaning::RebootRequired,
        (Engine::Msi | Engine::Burn, 1605) => ExitMeaning::Success, // already gone
        (Engine::Msi | Engine::Burn, 1618) => ExitMeaning::Busy,
        (Engine::Msi | Engine::Burn, 1602) => ExitMeaning::Cancelled,
        _ => ExitMeaning::Failed,
    }
}

/// Whether Windows is waiting for a restart to finish installing or
/// removing something.
pub fn reboot_pending(reg: &dyn Registry) -> bool {
    reg.exists("HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Component Based Servicing\\RebootPending")
        || reg.exists(
            "HKLM\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\WindowsUpdate\\Auto Update\\RebootRequired",
        )
        || reg
            .read_key("HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager")
            .is_some_and(|k| {
                k.values
                    .values()
                    .any(|v| v.name.eq_ignore_ascii_case("PendingFileRenameOperations"))
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{KeyData, RegValue};
    use crate::platform::MemRegistry;

    fn prog(key: &str, uninstall: &str) -> InstalledProgram {
        InstalledProgram {
            key: key.into(),
            name: "Contoso".into(),
            uninstall_string: uninstall.into(),
            ..Default::default()
        }
    }

    #[test]
    fn detects_and_builds_silent_commands() {
        let mut msi = prog("HKLM\\X\\{1111}", "MsiExec.exe /I{1111}");
        msi.is_msi = true;
        msi.product_code = "{1111}".into();
        assert_eq!(quick_engine(&msi), Engine::Msi);
        assert_eq!(
            silent_command(&msi, Engine::Msi).unwrap(),
            "MsiExec.exe /X{1111} /qn /norestart"
        );

        let inno = prog(
            "HKLM\\X\\Contoso_is1",
            "\"C:\\Program Files\\Contoso\\unins000.exe\"",
        );
        assert_eq!(quick_engine(&inno), Engine::Inno);
        assert_eq!(
            silent_command(&inno, Engine::Inno).unwrap(),
            "\"C:\\Program Files\\Contoso\\unins000.exe\" /VERYSILENT /SUPPRESSMSGBOXES /NORESTART"
        );

        let nsis = prog(
            "HKLM\\X\\Contoso",
            "\"C:\\Program Files\\Contoso\\Uninstall.exe\"",
        );
        assert_eq!(quick_engine(&nsis), Engine::Manual);
        assert_eq!(
            silent_command(&nsis, Engine::Nsis).unwrap(),
            "\"C:\\Program Files\\Contoso\\Uninstall.exe\" /S _?=C:\\Program Files\\Contoso"
        );

        let burn = prog(
            "HKLM\\X\\{2222}",
            "\"C:\\ProgramData\\Package Cache\\{2222}\\setup.exe\" /uninstall",
        );
        assert_eq!(quick_engine(&burn), Engine::Burn);
        assert_eq!(
            silent_command(&burn, Engine::Burn).unwrap(),
            "\"C:\\ProgramData\\Package Cache\\{2222}\\setup.exe\" /uninstall /quiet /norestart"
        );

        let sq = prog(
            "HKCU\\X\\Discord",
            "\"C:\\Users\\a\\AppData\\Local\\Discord\\Update.exe\" --uninstall",
        );
        assert_eq!(quick_engine(&sq), Engine::Squirrel);
        assert!(silent_command(&sq, Engine::Squirrel)
            .unwrap()
            .ends_with("--uninstall -s"));

        let mut vendor = prog("HKLM\\X\\V", "\"C:\\V\\uninst.exe\"");
        vendor.quiet_uninstall_string = "\"C:\\V\\uninst.exe\" /quiet".into();
        assert_eq!(quick_engine(&vendor), Engine::Vendor);

        let store = InstalledProgram {
            source: Source::Store,
            ..Default::default()
        };
        assert_eq!(quick_engine(&store), Engine::Store);
        assert!(silent_command(&store, Engine::Store).is_none());
    }

    #[test]
    fn recognises_nsis_files() {
        let dir = crate::util::tempdir().unwrap();
        let yes = dir.path().join("Uninstall.exe");
        let mut bytes = vec![0u8; 5000];
        bytes.extend_from_slice(b"Nullsoft.NSIS.exehead");
        std::fs::write(&yes, bytes).unwrap();
        let no = dir.path().join("other.exe");
        std::fs::write(&no, vec![1u8; 5000]).unwrap();
        assert!(is_nsis_file(&yes.to_string_lossy()));
        assert!(!is_nsis_file(&no.to_string_lossy()));
        let p = prog("HKLM\\X\\N", &format!("\"{}\"", yes.to_string_lossy()));
        assert_eq!(detect(&p), Engine::Nsis);
    }

    #[test]
    fn exit_codes_and_reboot() {
        assert_eq!(exit_meaning(Engine::Msi, 0), ExitMeaning::Success);
        assert_eq!(exit_meaning(Engine::Msi, 3010), ExitMeaning::RebootRequired);
        assert_eq!(exit_meaning(Engine::Msi, 1618), ExitMeaning::Busy);
        assert_eq!(exit_meaning(Engine::Inno, 1618), ExitMeaning::Failed);
        assert_eq!(exit_meaning(Engine::Msi, 1605), ExitMeaning::Success);

        let reg = MemRegistry::new();
        assert!(!reboot_pending(&reg));
        reg.put(
            KeyData::new("HKLM\\SYSTEM\\CurrentControlSet\\Control\\Session Manager")
                .with(RegValue::string("PendingFileRenameOperations", "x")),
        );
        assert!(reboot_pending(&reg));
    }
}
