//! Will this program work as a portable app? Looks at a captured
//! installation and the program's files for things that complicate or
//! prevent portable use: Java, .NET, Visual C++ runtimes, services, drivers,
//! shell extensions, machine-wide settings and administrator rights.

use crate::model::{ChangeSet, FileEntry};
use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Worth knowing; nothing to do.
    Info,
    /// The package works, with a condition (e.g. Java must be installed).
    Warning,
    /// Part of the program can't work portably.
    Problem,
}
crate::json_enum!(Severity {
    Info,
    Warning,
    Problem
});

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Info => "Note",
            Severity::Warning => "Needs attention",
            Severity::Problem => "Won't work portably",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    pub title: String,
    pub detail: String,
}
crate::json_struct!(Issue, { severity, title, detail });

/// What the package should declare because of what was found.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Needs {
    /// appinfo.ini [Dependencies] UsesJava=yes and [Activate] Java=require.
    pub java: bool,
    /// appinfo.ini [Dependencies] Requires64bitOS=yes.
    pub x64_only: bool,
    /// appinfo.ini [Dependencies] RequiresAdmin=yes.
    pub admin: bool,
    /// Runtime DLLs the installer put in System32 that should be copied next
    /// to the program (app-local deployment).
    pub runtime_dlls: Vec<String>,
}
crate::json_struct!(Needs, { java, x64_only, admin, runtime_dlls });

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    pub issues: Vec<Issue>,
    pub needs: Needs,
}
crate::json_struct!(Report, { issues, needs });

impl Report {
    fn add(&mut self, severity: Severity, title: &str, detail: impl Into<String>) {
        self.issues.push(Issue {
            severity,
            title: title.into(),
            detail: detail.into(),
        });
    }

    pub fn worst(&self) -> Option<Severity> {
        self.issues.iter().map(|i| i.severity).max()
    }
}

// ---- Program files ------------------------------------------------------

/// What a Windows program file is built for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeInfo {
    pub machine: u16,
    /// A .NET (managed) assembly.
    pub managed: bool,
}

pub const MACHINE_X86: u16 = 0x14c;
pub const MACHINE_X64: u16 = 0x8664;
pub const MACHINE_ARM64: u16 = 0xaa64;

/// Read the PE headers of a program file or DLL.
pub fn pe_info(bytes: &[u8]) -> Option<PeInfo> {
    let u16_at = |o: usize| {
        bytes
            .get(o..o + 2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
    };
    let u32_at = |o: usize| {
        bytes
            .get(o..o + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    if bytes.get(0..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(0x3c)? as usize;
    if bytes.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let machine = u16_at(pe + 4)?;
    let opt = pe + 24;
    let magic = u16_at(opt)?;
    // Data directories start at 96 (PE32) or 112 (PE32+); entry 14 is the CLR header.
    let dirs = opt + if magic == 0x20b { 112 } else { 96 };
    let clr = dirs + 14 * 8;
    let managed = u32_at(clr).unwrap_or(0) != 0 && u32_at(clr + 4).unwrap_or(0) != 0;
    Some(PeInfo { machine, managed })
}

fn read_head(path: &Path, max: usize) -> Vec<u8> {
    let mut buf = Vec::new();
    if let Ok(f) = std::fs::File::open(path) {
        let _ = f.take(max as u64).read_to_end(&mut buf);
    }
    buf
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle))
}

fn lower(p: &Path) -> String {
    p.to_string_lossy().to_lowercase()
}

/// The .NET runtime a framework-dependent app asks for, from its
/// *.runtimeconfig.json: ("Microsoft.WindowsDesktop.App", "8.0.0").
fn runtime_config(json: &str) -> Option<(String, String)> {
    let v: crate::json::Value = crate::json::compat::from_str(json).ok()?;
    let opts = v.get("runtimeOptions")?;
    let fw = opts.get("framework").cloned().or_else(|| {
        opts.get("frameworks")
            .and_then(|f| f.as_array())
            .and_then(|a| a.last().cloned())
    })?;
    Some((
        fw.get("name")?.as_str()?.to_string(),
        fw.get("version")?.as_str()?.to_string(),
    ))
}

const VC_RUNTIME: [&str; 6] = ["vcruntime", "msvcp", "concrt", "vccorlib", "vcomp", "mfc"];

/// Look at the program's folder and the captured changes.
pub fn analyze(cs: &ChangeSet, program_dir: &Path, main_exe: &Path) -> Report {
    let mut r = Report::default();
    let files: Vec<PathBuf> = crate::fswalk::WalkDir::new(program_dir)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();
    let has_file = |name: &str| {
        files.iter().any(|f| {
            f.file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
        })
    };

    // Java.
    let jars = files.iter().filter(|f| lower(f).ends_with(".jar")).count();
    let bundled_jre = has_file("java.exe") || has_file("javaw.exe") || has_file("jvm.dll");
    let main = read_head(main_exe, 8 * 1024 * 1024);
    let wrapper = contains(&main, b"launch4j")
        || (contains(&main, b"JAVA_HOME") && contains(&main, b"javaw"));
    if jars > 0 || wrapper {
        if bundled_jre {
            r.add(Severity::Info, "Java (included)", "The program brings its own Java runtime, so it runs without Java installed. The package is larger because of it.");
        } else {
            r.needs.java = true;
            r.add(
                Severity::Warning,
                "Needs Java",
                "The program is written in Java and doesn't include a Java runtime. The package is marked as needing Java: the PortableApps.com Launcher uses jPortable (CommonFiles\\Java) or an installed Java, and asks for it when there is none.",
            );
        }
    }

    // .NET.
    let mut managed = false;
    let mut x64 = false;
    for f in files.iter().filter(|f| {
        let l = lower(f);
        l.ends_with(".exe") || l.ends_with(".dll")
    }) {
        let head = read_head(f, 4096);
        if let Some(pe) = pe_info(&head) {
            managed |= pe.managed;
            if f == main_exe {
                x64 = pe.machine == MACHINE_X64 || pe.machine == MACHINE_ARM64;
            }
        }
    }
    let self_contained = has_file("coreclr.dll") || has_file("hostfxr.dll");
    let config = files
        .iter()
        .find(|f| lower(f).ends_with(".runtimeconfig.json"))
        .and_then(|f| std::fs::read_to_string(f).ok())
        .and_then(|j| runtime_config(&j));
    match (&config, self_contained) {
        (_, true) => r.add(Severity::Info, ".NET (included)", "The program brings its own .NET runtime, so it runs without .NET installed."),
        (Some((name, version)), false) => r.add(
            Severity::Warning,
            "Needs the .NET runtime",
            format!(
                "The program needs {} {} (or a newer {} release) installed on every PC it runs on. Windows 11 doesn't include it. On a PC without it, Windows offers to download it when the program starts.",
                if name.contains("WindowsDesktop") { ".NET Desktop Runtime" } else if name.contains("AspNetCore") { "the ASP.NET Core Runtime" } else { ".NET Runtime" },
                version,
                version.split('.').next().unwrap_or(version),
            ),
        ),
        (None, false) if managed => r.add(
            Severity::Info,
            ".NET Framework",
            "The program uses the .NET Framework, which is part of Windows 11, so it runs anywhere.",
        ),
        _ => {}
    }

    if x64 {
        r.needs.x64_only = true;
        r.add(Severity::Info, "64-bit only", "The program only runs on 64-bit Windows, which every Windows 11 PC is. The package says so.");
    }

    // Administrator rights.
    if contains(&main, b"requireAdministrator") {
        r.needs.admin = true;
        r.add(Severity::Warning, "Needs administrator rights", "The program always asks for administrator rights when it starts, so it won't run on PCs where you can't approve that, such as school or work computers.");
    }

    // Visual C++ runtimes installed into Windows.
    let windir = std::env::var("windir")
        .unwrap_or_else(|_| "C:\\Windows".into())
        .to_lowercase();
    let system_dlls: Vec<&FileEntry> = cs
        .files_added
        .iter()
        .filter(|f| {
            let l = f.path.to_lowercase();
            (l.starts_with(&format!("{windir}\\system32\\"))
                || l.starts_with(&format!("{windir}\\syswow64\\")))
                && l.ends_with(".dll")
                && !l[windir.len() + 10..].contains('\\')
        })
        .collect();
    let runtimes: Vec<String> = system_dlls
        .iter()
        .filter(|f| {
            let name = f.path.rsplit('\\').next().unwrap_or("").to_lowercase();
            VC_RUNTIME.iter().any(|p| name.starts_with(p))
                || name.starts_with("ucrtbase")
                || name.starts_with("api-ms-win-crt")
        })
        .map(|f| f.path.clone())
        .collect();
    let other_dlls: Vec<String> = system_dlls
        .iter()
        .map(|f| f.path.clone())
        .filter(|p| !runtimes.contains(p))
        .collect();
    if !runtimes.is_empty() {
        r.add(
            Severity::Info,
            "Visual C++ runtime",
            format!("The installer added {} Visual C++ runtime files to Windows. They are copied next to the program, so the portable version doesn't need the Visual C++ Redistributable.", runtimes.len()),
        );
        r.needs.runtime_dlls = runtimes;
    }
    if !other_dlls.is_empty() {
        r.add(
            Severity::Warning,
            "Files in the Windows folder",
            format!("The installer put these files in the Windows folder, which a portable app can't do: {}. If the program needs them, copy them next to its program file.", short_list(&other_dlls)),
        );
    }

    // Services and drivers.
    let services: Vec<String> = cs
        .keys_added
        .iter()
        .filter_map(|k| {
            let l = k.path.to_lowercase();
            let rest = l.strip_prefix("hklm\\system\\currentcontrolset\\services\\")?;
            (!rest.contains('\\')).then(|| k.path.rsplit('\\').next().unwrap_or("").to_string())
        })
        .collect();
    let drivers = cs
        .files_added
        .iter()
        .any(|f| f.path.to_lowercase().ends_with(".sys"));
    if drivers {
        r.add(Severity::Problem, "Installs a driver", "The program installs a driver. Drivers can't be portable, so the features that need it won't work.");
    }
    if !services.is_empty() {
        r.add(
            Severity::Problem,
            "Installs a Windows service",
            format!("The program installs {} ({}). Portable apps can't install services, so features that rely on it (often updates, sync or background tasks) won't work.", if services.len() == 1 { "a service" } else { "services" }, short_list(&services)),
        );
    }

    // COM classes and shell extensions.
    let com = cs.keys_added.iter().any(|k| {
        let l = k.path.to_lowercase();
        (l.contains("\\classes\\clsid\\{") || l.contains("\\classes\\wow6432node\\clsid\\{"))
            && l.ends_with("\\inprocserver32")
    });
    let shellex = cs.keys_added.iter().any(|k| {
        let l = k.path.to_lowercase();
        l.contains("\\shellex\\contextmenuhandlers\\")
            || l.contains("\\shellex\\propertysheethandlers\\")
            || l.contains("\\shelliconoverlayidentifiers\\")
    });
    if shellex {
        r.add(Severity::Warning, "Explorer integration", "The program adds itself to File Explorer (right-click menu, icons or property pages). The portable version won't, but the program itself works.");
    } else if com {
        r.add(Severity::Warning, "Registers components", "The program registers COM components with Windows. If it needs them at runtime, some features may not work portably; test the package on another PC.");
    }

    // Machine-wide settings.
    let hklm_settings = cs.keys_added.iter().any(|k| {
        let l = k.path.to_lowercase();
        l.starts_with("hklm\\software\\")
            && !l.contains("\\classes\\")
            && !l.contains("\\microsoft\\windows\\currentversion\\uninstall\\")
            && !l.contains("\\microsoft\\windows\\currentversion\\installer\\")
            && !l.contains("\\microsoft\\")
    });
    if hklm_settings {
        r.add(Severity::Info, "Machine-wide settings", "The installer wrote settings for all users (HKLM). Portable apps normally don't need them; they are listed in the Launcher settings to review.");
    }

    // File types.
    let types: Vec<String> = cs
        .keys_added
        .iter()
        .filter_map(|k| {
            let l = k.path.to_lowercase();
            let rest = l
                .strip_prefix("hkcu\\software\\classes\\")
                .or_else(|| l.strip_prefix("hklm\\software\\classes\\"))?;
            (rest.starts_with('.') && !rest.contains('\\'))
                .then(|| rest.trim_start_matches('.').to_string())
        })
        .collect();
    if !types.is_empty() {
        r.add(
            Severity::Info,
            "File types",
            format!("The program opens these file types: {}. The PortableApps.com Platform can associate them with the portable version ([Associations] in appinfo.ini).", short_list(&types)),
        );
    }

    if r.issues.is_empty() {
        r.add(
            Severity::Info,
            "No complications found",
            "Nothing in this installation is known to stop it from working as a portable app.",
        );
    }
    r.issues.sort_by_key(|i| std::cmp::Reverse(i.severity));
    r
}

fn short_list(items: &[String]) -> String {
    let mut v: Vec<&str> = items
        .iter()
        .map(|s| s.rsplit('\\').next().unwrap_or(s))
        .collect();
    v.dedup();
    if v.len() > 6 {
        format!("{} and {} more", v[..6].join(", "), v.len() - 6)
    } else {
        v.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KeyData;

    fn pe(machine: u16, plus: bool, managed: bool) -> Vec<u8> {
        let mut b = vec![0u8; 512];
        b[0] = b'M';
        b[1] = b'Z';
        b[0x3c] = 0x80;
        b[0x80..0x84].copy_from_slice(b"PE\0\0");
        b[0x84..0x86].copy_from_slice(&machine.to_le_bytes());
        let opt = 0x80 + 24;
        b[opt..opt + 2].copy_from_slice(&(if plus { 0x20bu16 } else { 0x10b }).to_le_bytes());
        if managed {
            let clr = opt + if plus { 112 } else { 96 } + 14 * 8;
            b[clr..clr + 4].copy_from_slice(&0x2000u32.to_le_bytes());
            b[clr + 4..clr + 8].copy_from_slice(&0x48u32.to_le_bytes());
        }
        b
    }

    #[test]
    fn reads_pe_headers() {
        assert_eq!(
            pe_info(&pe(MACHINE_X64, true, false)),
            Some(PeInfo {
                machine: MACHINE_X64,
                managed: false
            })
        );
        assert_eq!(
            pe_info(&pe(MACHINE_X86, false, true)),
            Some(PeInfo {
                machine: MACHINE_X86,
                managed: true
            })
        );
        assert_eq!(pe_info(b"not a program"), None);
    }

    #[test]
    fn reads_runtime_config() {
        let j = r#"{"runtimeOptions":{"tfm":"net8.0","framework":{"name":"Microsoft.WindowsDesktop.App","version":"8.0.0"}}}"#;
        assert_eq!(
            runtime_config(j),
            Some(("Microsoft.WindowsDesktop.App".into(), "8.0.0".into()))
        );
        let j = r#"{"runtimeOptions":{"frameworks":[{"name":"Microsoft.NETCore.App","version":"6.0.0"},{"name":"Microsoft.WindowsDesktop.App","version":"6.0.0"}]}}"#;
        assert_eq!(runtime_config(j).unwrap().0, "Microsoft.WindowsDesktop.App");
    }

    #[test]
    fn finds_complications() {
        let dir = std::env::temp_dir().join(format!("setupscope-port-{}", crate::util::new_id()));
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("app.exe"), pe(MACHINE_X64, true, true)).unwrap();
        std::fs::write(dir.join("lib").join("app.jar"), b"PK").unwrap();
        std::fs::write(dir.join("app.runtimeconfig.json"), r#"{"runtimeOptions":{"framework":{"name":"Microsoft.WindowsDesktop.App","version":"8.0.0"}}}"#).unwrap();
        let cs = ChangeSet {
            keys_added: vec![
                KeyData::new("HKLM\\SYSTEM\\CurrentControlSet\\Services\\AppUpdater"),
                KeyData::new("HKCU\\Software\\Classes\\.foo"),
            ],
            files_added: vec![FileEntry {
                path: "C:\\Windows\\System32\\vcruntime140.dll".into(),
                is_dir: false,
                size: 1,
                mtime: 0,
            }],
            ..Default::default()
        };
        std::env::set_var("windir", "C:\\Windows");
        let r = analyze(&cs, &dir, &dir.join("app.exe"));
        let titles: Vec<&str> = r.issues.iter().map(|i| i.title.as_str()).collect();
        assert_eq!(r.issues[0].severity, Severity::Problem, "{titles:?}");
        for want in [
            "Needs Java",
            "Needs the .NET runtime",
            "Installs a Windows service",
            "64-bit only",
            "Visual C++ runtime",
            "File types",
        ] {
            assert!(titles.contains(&want), "{want} missing from {titles:?}");
        }
        assert!(r.needs.java && r.needs.x64_only);
        assert_eq!(
            r.needs.runtime_dlls,
            vec!["C:\\Windows\\System32\\vcruntime140.dll".to_string()]
        );
        assert!(r
            .issues
            .iter()
            .any(|i| i.detail.contains(".NET Desktop Runtime 8.0.0")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
