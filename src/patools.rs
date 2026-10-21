//! The PortableApps.com toolchain, built in: InstallScope compiles launchers
//! with the official PortableApps.com Launcher source (GPL, from GitHub,
//! which includes the NSIS compiler it needs) exactly as the Launcher
//! Generator does, and builds `.paf.exe` installers with its own NSIS
//! script, so neither tool has to be installed.
//!
//! The Launcher source is looked for next to InstallScope.exe (release
//! downloads include it in `tools\PortableApps.com Launcher`), then in
//! InstallScope's data folder, and is downloaded from GitHub when missing. The
//! compiler is only used when its SHA-256 matches the pinned release.

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

/// PortableApps/Launcher on GitHub, pinned (2024-04-16, Launcher 2.2.x).
pub const LAUNCHER_COMMIT: &str = "5470210a6841c38950c348e1190ad92787b8b727";
/// App\NSIS\makensis.exe at that commit.
pub const MAKENSIS_SHA256: &str =
    "9bffbceea9af5cdd15299ec957e5ee479a048bfb824dd45ea51414c1c2c750e0";
/// Folder name used next to InstallScope.exe and in the data folder.
/// (No spaces: parts of the Launcher source don't quote their paths.)
pub const TOOLS_DIR: &str = "PortableApps.comLauncher";

pub fn download_url() -> String {
    format!("https://codeload.github.com/PortableApps/Launcher/zip/{LAUNCHER_COMMIT}")
}

/// A usable copy of the Launcher source and its NSIS compiler.
#[derive(Clone, Debug, PartialEq)]
pub struct Toolchain {
    pub root: PathBuf,
}

fn sha256_file(p: &Path) -> Option<String> {
    let file = std::fs::File::open(p).ok()?;
    Some(
        crate::util::sha256_reader(file)
            .ok()?
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    )
}

impl Toolchain {
    pub fn makensis(&self) -> PathBuf {
        self.root.join("App").join("NSIS").join("makensis.exe")
    }

    fn launcher_nsi(&self) -> PathBuf {
        self.root
            .join("Other")
            .join("Source")
            .join("PortableApps.comLauncher.nsi")
    }

    /// The pinned compiler and the Launcher source are both there.
    pub fn is_valid(&self) -> bool {
        self.launcher_nsi().is_file()
            && sha256_file(&self.makensis())
                .is_some_and(|h| h.eq_ignore_ascii_case(MAKENSIS_SHA256))
    }

    /// The Launcher's own version, which the Generator stamps into launchers.
    pub fn launcher_version(&self) -> String {
        ini_value(
            &self.root.join("App").join("AppInfo").join("appinfo.ini"),
            "Version",
            "PackageVersion",
        )
        .unwrap_or_else(|| "2.2.0.0".into())
    }
}

/// Read `key` from `[section]` of an INI file (UTF-8 or UTF-16).
pub fn ini_value(path: &Path, section: &str, key: &str) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let text = if bytes.starts_with(&[0xFF, 0xFE]) {
        let u: Vec<u16> = bytes[2..]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], *c.get(1).unwrap_or(&0)]))
            .collect();
        String::from_utf16_lossy(&u)
    } else {
        String::from_utf8_lossy(bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(&bytes))
            .into_owned()
    };
    let mut in_section = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with('[') {
            in_section = l.trim_matches(['[', ']']).eq_ignore_ascii_case(section);
        } else if in_section {
            if let Some((k, v)) = l.split_once('=') {
                if k.trim().eq_ignore_ascii_case(key) {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// Where a copy could be: next to InstallScope.exe, then in the data folder.
pub fn candidates(data_root: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        v.push(dir.join("tools").join(TOOLS_DIR));
    }
    v.push(
        data_root
            .join("tools")
            .join(format!("{TOOLS_DIR}-{}", &LAUNCHER_COMMIT[..7])),
    );
    v
}

pub fn find(data_root: &Path) -> Option<Toolchain> {
    let t = candidates(data_root)
        .into_iter()
        .map(|root| Toolchain { root })
        .find(|t| t.is_valid())?;
    // Best effort: a read-only copy (Program Files) was patched when it was made.
    let _ = patch_source(&t.root);
    Some(t)
}

/// Parts of the source archive that aren't needed to compile.
fn skip_entry(rel: &str) -> bool {
    let l = rel.to_lowercase().replace('\\', "/");
    l.starts_with("app/nsis/docs/")
        || l.starts_with("app/nsis/examples/")
        || l.ends_with(".chm")
        || l.starts_with("other/source/manual/")
        || l.starts_with(".github/")
}

/// Unpack the GitHub source archive (one top folder) into `dest`.
pub fn extract(zip_path: &Path, dest: &Path) -> Result<()> {
    let f = std::fs::File::open(zip_path)?;
    let mut z = crate::zip::ZipArchive::new(f).context("the download isn't a valid archive")?;
    for i in 0..z.len() {
        let mut e = z.by_index(i)?;
        let Some(name) = e.enclosed_name() else {
            continue;
        };
        let rel: PathBuf = name.components().skip(1).collect();
        if rel.as_os_str().is_empty() || skip_entry(&rel.to_string_lossy()) {
            continue;
        }
        let out = dest.join(&rel);
        if e.is_dir() {
            std::fs::create_dir_all(&out)?;
        } else {
            if let Some(p) = out.parent() {
                std::fs::create_dir_all(p)?;
            }
            let mut w = std::fs::File::create(&out)?;
            std::io::copy(&mut e, &mut w)?;
        }
    }
    Ok(())
}

/// The Launcher source includes NSIS's Registry.nsh without quotes, which
/// breaks when the folder path has a space (such as a user name with one).
pub fn patch_source(root: &Path) -> Result<()> {
    let p = root.join("Other").join("Source").join("Registry.nsh");
    if let Ok(text) = std::fs::read_to_string(&p) {
        let fixed = text.replace(
            "!include ${NSISDIR}\\Include\\Registry.nsh",
            "!include \"${NSISDIR}\\Include\\Registry.nsh\"",
        );
        if fixed != text {
            std::fs::write(&p, fixed)?;
        }
    }
    Ok(())
}

/// Download the pinned Launcher source from GitHub into the data folder.
/// Uses curl.exe, which is part of Windows 11.
pub fn download(data_root: &Path, report: &dyn Fn(&str)) -> Result<Toolchain> {
    download_with_cancel(data_root, report, &|| false)
}

fn download_with_cancel(
    data_root: &Path,
    report: &dyn Fn(&str),
    cancel: &dyn Fn() -> bool,
) -> Result<Toolchain> {
    if cancel() {
        return Err(anyhow!("Build cancelled"));
    }

    let dest = candidates(data_root)
        .pop()
        .ok_or_else(|| anyhow!("no data folder"))?;
    let tmp_dir = data_root.join("tools");
    std::fs::create_dir_all(&tmp_dir)?;
    let zip = tmp_dir.join("launcher-download.zip");
    report("Downloading the PortableApps.com Launcher (about 6 MB, from GitHub)");
    #[cfg(windows)]
    let curl = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("Windows system folder is unavailable"))?
        .join("System32/curl.exe");
    #[cfg(not(windows))]
    let curl = PathBuf::from("/usr/bin/curl");
    let request = portable_builder_kit::process::ToolRequest {
        executable: curl,
        arguments: [
            "-L",
            "--fail",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "15",
            "--max-time",
            "120",
            "--retry",
            "2",
            "-o",
        ]
        .into_iter()
        .map(std::ffi::OsString::from)
        .chain([zip.clone().into_os_string(), download_url().into()])
        .collect(),
        working_directory: None,
        timeout: std::time::Duration::from_secs(180),
    };
    if let Err(error) = portable_builder_kit::process::run_tool(&request, cancel) {
        let _ = std::fs::remove_file(&zip);
        return Err(anyhow!(
            "couldn't download the PortableApps.com Launcher: {error}"
        ));
    }
    if cancel() {
        let _ = std::fs::remove_file(&zip);
        return Err(anyhow!("Build cancelled"));
    }
    report("Unpacking the PortableApps.com Launcher");
    let staging = tmp_dir.join("launcher-staging");
    let _ = std::fs::remove_dir_all(&staging);
    let r = extract(&zip, &staging);
    let _ = std::fs::remove_file(&zip);
    r?;
    patch_source(&staging)?;
    let t = Toolchain {
        root: staging.clone(),
    };
    if !t.is_valid() {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(anyhow!("the downloaded PortableApps.com Launcher didn't match the expected files, so it wasn't used"));
    }
    let _ = std::fs::remove_dir_all(&dest);
    std::fs::rename(&staging, &dest)?;
    Ok(Toolchain { root: dest })
}

/// Find the toolchain, downloading it when needed.
pub fn ensure(data_root: &Path, report: &dyn Fn(&str)) -> Result<Toolchain> {
    ensure_with_cancel(data_root, report, &|| false)
}

pub fn ensure_with_cancel(
    data_root: &Path,
    report: &dyn Fn(&str),
    cancel: &dyn Fn() -> bool,
) -> Result<Toolchain> {
    if cancel() {
        return Err(anyhow!("Build cancelled"));
    }
    match find(data_root) {
        Some(t) => Ok(t),
        None => download_with_cancel(data_root, report, cancel),
    }
}

fn run_makensis(tc: &Toolchain, args: &[String], cancel: &dyn Fn() -> bool) -> Result<String> {
    use std::io::{Read, Seek};
    let log = std::env::temp_dir().join(format!("installscope-nsis-{}.log", crate::util::new_id()));
    // Reserve our own diagnostic path before handing it to the trusted compiler.
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&log)?;
    let mut arguments: Vec<std::ffi::OsString> = vec![
        format!("/O{}", log.display()).into(),
        "/OUTPUTCHARSET=UTF8".into(),
    ];
    arguments.extend(args.iter().map(std::ffi::OsString::from));
    let request = portable_builder_kit::process::ToolRequest {
        executable: tc.makensis(),
        arguments,
        working_directory: None,
        timeout: std::time::Duration::from_secs(600),
    };
    let result = portable_builder_kit::process::run_tool(&request, cancel);
    let diagnostics = (|| -> std::io::Result<String> {
        let mut file = std::fs::File::open(&log)?;
        let size = file.metadata()?.len();
        file.seek(std::io::SeekFrom::Start(size.saturating_sub(65536)))?;
        let mut bytes = Vec::new();
        file.take(65536).read_to_end(&mut bytes)?;
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    })()
    .unwrap_or_else(|error| format!("Could not read compiler diagnostics: {error}"));
    if let Err(error) = result {
        let tail: Vec<_> = diagnostics
            .lines()
            .rev()
            .filter(|l| !l.trim().is_empty())
            .take(6)
            .collect();
        return Err(anyhow!(
            "NSIS: {error}: {} (full log: {})",
            tail.into_iter().rev().collect::<Vec<_>>().join(" / "),
            log.display()
        ));
    }
    let _ = std::fs::remove_file(log);
    Ok(diagnostics)
}

/// Compile `<package>\<AppID>.exe` from the official Launcher source, the
/// way the PortableApps.com Launcher Generator does.
pub fn build_launcher(tc: &Toolchain, pkg: &Path) -> Result<PathBuf> {
    build_launcher_with_cancel(tc, pkg, &|| false)
}

pub fn build_launcher_with_cancel(
    tc: &Toolchain,
    pkg: &Path,
    cancel: &dyn Fn() -> bool,
) -> Result<PathBuf> {
    if cancel() {
        return Err(anyhow!("Build cancelled"));
    }

    let info = pkg.join("App").join("AppInfo").join("appinfo.ini");
    let name =
        ini_value(&info, "Details", "Name").ok_or_else(|| anyhow!("appinfo.ini has no Name"))?;
    let id =
        ini_value(&info, "Details", "AppID").ok_or_else(|| anyhow!("appinfo.ini has no AppID"))?;
    let launcher_ini = pkg
        .join("App")
        .join("AppInfo")
        .join("Launcher")
        .join(format!("{id}.ini"));
    // The launcher's icon comes from the package; use a generic one if it has none.
    let icon = pkg.join("App").join("AppInfo").join("appicon.ico");
    if !icon.is_file() {
        let _ = std::fs::copy(
            tc.root.join("App").join("AppInfo").join("appicon.ico"),
            &icon,
        );
    }
    let out = pkg.join(format!("{id}.exe"));
    if out.exists() {
        std::fs::remove_file(&out)
            .with_context(|| format!("{} is in use; close it and try again", out.display()))?;
    }
    let mut args = vec![
        "/V2".to_string(),
        format!("/DPACKAGE={}", pkg.display()),
        format!("/DNamePortable={name}"),
        format!("/DAppID={id}"),
        format!("/DVersion={}", tc.launcher_version()),
    ];
    if ini_value(&launcher_ini, "Activate", "XML").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
        args.push("/DXML_ENABLED".into());
    }
    if ini_value(&launcher_ini, "Launch", "RunAsAdmin")
        .is_some_and(|v| v.eq_ignore_ascii_case("compile-force"))
    {
        args.push("/DRUNASADMIN_COMPILEFORCE".into());
    }
    args.push(tc.launcher_nsi().to_string_lossy().to_string());
    run_makensis(tc, &args, cancel)?;
    if !out.is_file() {
        return Err(anyhow!("the launcher wasn't created"));
    }
    Ok(out)
}

/// InstallScope's installer script (MIT): installs a PortableApps.com Format
/// package like PortableApps.com installers do. It honours /DESTINATION=
/// and /AUTOCLOSE= from the PortableApps.com Platform and /S, keeps Data on
/// upgrades and replaces App and Other.
pub const INSTALLER_NSI: &str = include_str!("paf_installer.nsi");

/// Build `<AppID>_<version>.paf.exe` next to the package folder.
pub fn build_installer(tc: &Toolchain, pkg: &Path, work: &Path) -> Result<PathBuf> {
    build_installer_with_cancel(tc, pkg, work, &|| false)
}

pub fn build_installer_with_cancel(
    tc: &Toolchain,
    pkg: &Path,
    work: &Path,
    cancel: &dyn Fn() -> bool,
) -> Result<PathBuf> {
    if cancel() {
        return Err(anyhow!("Build cancelled"));
    }

    let info = pkg.join("App").join("AppInfo").join("appinfo.ini");
    let name =
        ini_value(&info, "Details", "Name").ok_or_else(|| anyhow!("appinfo.ini has no Name"))?;
    let id =
        ini_value(&info, "Details", "AppID").ok_or_else(|| anyhow!("appinfo.ini has no AppID"))?;
    let pv = ini_value(&info, "Version", "PackageVersion").unwrap_or_else(|| "1.0.0.0".into());
    let dv = ini_value(&info, "Version", "DisplayVersion").unwrap_or_else(|| pv.clone());
    if !pkg.join(format!("{id}.exe")).is_file() {
        return Err(anyhow!("make the launcher first"));
    }
    let safe_dv: String = dv
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let parent = pkg.parent().unwrap_or(pkg);
    let out = parent.join(format!("{id}_{safe_dv}.paf.exe"));
    std::fs::create_dir_all(work)?;
    let script = work.join("paf_installer.nsi");
    // NSIS reads UTF-8 scripts that start with a BOM.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(INSTALLER_NSI.as_bytes());
    std::fs::write(&script, bytes)?;
    let args = vec![
        "/V2".to_string(),
        format!("/DPACKAGE={}", pkg.display()),
        format!("/DAPPID={id}"),
        format!("/DNAME={name}"),
        format!("/DPACKAGEVERSION={pv}"),
        format!("/DDISPLAYVERSION={dv}"),
        format!("/DOUTFILE={}", out.display()),
        script.to_string_lossy().to_string(),
    ];
    run_makensis(tc, &args, cancel)?;
    if !out.is_file() {
        return Err(anyhow!("the installer wasn't created"));
    }
    Ok(out)
}

/// InstallScope's silent setup script (MIT): one file that redeploys a
/// repackaged installation without questions.
pub const SILENT_SETUP_NSI: &str = include_str!("silent_setup.nsi");

/// Compile a repackaged folder (with install.ps1) into one silent
/// `<name> Setup.exe` next to it.
pub fn build_silent_setup(
    tc: &Toolchain,
    package: &Path,
    name: &str,
    version: &str,
    work: &Path,
) -> Result<PathBuf> {
    if !package.join("install.ps1").is_file() {
        return Err(anyhow!("{} has no install.ps1", package.display()));
    }
    let safe = crate::util::safe_file_name(name);
    let out = package.parent().unwrap_or(package).join(format!(
        "{} Setup.exe",
        crate::util::safe_file_name(format!("{name} {version}").trim())
    ));
    std::fs::create_dir_all(work)?;
    let script = work.join("silent_setup.nsi");
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(SILENT_SETUP_NSI.as_bytes());
    std::fs::write(&script, bytes)?;
    let numeric = crate::pafbuild::package_version(if version.trim().is_empty() {
        "1.0"
    } else {
        version
    });
    let args = vec![
        "/V2".to_string(),
        format!("/DPACKAGE={}", package.display()),
        format!("/DNAME={name}"),
        format!("/DSAFENAME={safe}"),
        format!("/DVERSION={numeric}"),
        format!("/DOUTFILE={}", out.display()),
        script.to_string_lossy().to_string(),
    ];
    run_makensis(tc, &args, &|| false)?;
    if !out.is_file() {
        return Err(anyhow!("the setup file wasn't created"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn reads_ini_values() {
        let dir = std::env::temp_dir().join(format!("installscope-ini-{}", crate::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("appinfo.ini");
        std::fs::write(&p, "\u{feff}[Details]\r\nName=Foo Portable\r\nAppID = FooPortable\r\n[Version]\r\nPackageVersion=1.2.0.0\r\n").unwrap();
        assert_eq!(ini_value(&p, "details", "AppID").unwrap(), "FooPortable");
        assert_eq!(
            ini_value(&p, "Version", "PackageVersion").unwrap(),
            "1.2.0.0"
        );
        assert!(ini_value(&p, "Version", "Missing").is_none());
        let utf16: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(
                "[Version]\r\nPackageVersion=2.2.9.0\r\n"
                    .encode_utf16()
                    .flat_map(|u| u.to_le_bytes()),
            )
            .collect();
        std::fs::write(&p, utf16).unwrap();
        assert_eq!(
            ini_value(&p, "Version", "PackageVersion").unwrap(),
            "2.2.9.0"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_without_docs_and_rejects_wrong_compiler() {
        let dir = std::env::temp_dir().join(format!("installscope-pal-{}", crate::util::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zp = dir.join("l.zip");
        {
            let mut z = crate::zip::ZipWriter::new(std::fs::File::create(&zp).unwrap()).unwrap();
            for (name, data) in [
                (
                    "Launcher-abc/App/NSIS/makensis.exe",
                    &b"not the real one"[..],
                ),
                ("Launcher-abc/App/NSIS/Docs/readme.txt", b"docs"),
                (
                    "Launcher-abc/Other/Source/PortableApps.comLauncher.nsi",
                    b"; nsi",
                ),
                ("Launcher-abc/Other/Source/Manual/index.rst", b"manual"),
            ] {
                z.start_file(name).unwrap();
                z.write_all(data).unwrap();
            }
            z.finish().unwrap();
        }
        let out = dir.join("out");
        extract(&zp, &out).unwrap();
        assert!(out.join("App/NSIS/makensis.exe").is_file());
        assert!(out
            .join("Other/Source/PortableApps.comLauncher.nsi")
            .is_file());
        assert!(!out.join("App/NSIS/Docs").exists());
        assert!(!out.join("Other/Source/Manual").exists());
        // The compiler's fingerprint doesn't match, so it isn't trusted.
        assert!(!Toolchain { root: out }.is_valid());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quotes_the_registry_include() {
        let dir =
            std::env::temp_dir().join(format!("installscope-patch-{}", crate::util::new_id()));
        let src = dir.join("Other").join("Source");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("Registry.nsh"),
            "!include ${NSISDIR}\\Include\\Registry.nsh\r\n",
        )
        .unwrap();
        patch_source(&dir).unwrap();
        let t = std::fs::read_to_string(src.join("Registry.nsh")).unwrap();
        assert!(
            t.starts_with("!include \"${NSISDIR}\\Include\\Registry.nsh\""),
            "{t}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn installer_script_takes_the_platform_options() {
        for want in [
            "/DESTINATION=",
            "/AUTOCLOSE=",
            "${OUTFILE}",
            "\\Data",
            "Unicode true",
        ] {
            assert!(INSTALLER_NSI.contains(want), "{want}");
        }
    }
}
