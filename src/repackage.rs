//! Repackage a monitored installation as a deployable folder:
//!
//! ```text
//! <name>/
//!   files/<FOLDER>/…      the installed files, grouped by destination folder
//!   install.reg           registry entries the installation created or changed
//!   install.ps1           silent install: copies files, registers services,
//!                         imports install.reg, adds PATH entries
//!   uninstall.ps1         precise removal (the same script SetupScope exports)
//!   README.txt
//! ```
//!
//! Folders are stored by variable (ProgramFiles, APPDATA, …), so the script
//! puts files in the right place on any PC and for any user. Files are taken
//! as they are now, so settings changed since installing are included.

use crate::model::{fold, InstallLog, KeyData, RegValue};
use crate::platform::{self, expand_env, Registry};
use crate::report::{long_path, reg_data, reg_name, to_ps1};
use crate::transfer::Vars;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Summary {
    pub files: usize,
    pub bytes: u64,
    pub keys: usize,
    pub values: usize,
    pub services: Vec<String>,
    /// Files that were recorded but no longer exist.
    pub missing: usize,
    pub folder: PathBuf,
}

/// `%ProgramFiles%\Contoso\app.exe` → (`ProgramFiles`, `Contoso\app.exe`);
/// an absolute path without a variable → (`_D`, `rest`) for drive D:.
pub fn split_placeholder(p: &str) -> Option<(String, String)> {
    if let Some(rest) = p.strip_prefix('%') {
        let (var, tail) = rest.split_once('%')?;
        return Some((
            var.to_string(),
            tail.trim_start_matches(['\\', '/']).to_string(),
        ));
    }
    let b = p.as_bytes();
    if b.len() > 3 && b[0].is_ascii_alphabetic() && b[1] == b':' {
        return Some((
            format!("_{}", (b[0] as char).to_ascii_uppercase()),
            p[3..].to_string(),
        ));
    }
    None
}

/// A forward .reg file: creates the keys and sets the values.
pub fn to_reg_install(keys: &[KeyData], values: &[(String, RegValue)], title: &str) -> String {
    let mut s = format!(
        "Windows Registry Editor Version 5.00\r\n\r\n; {title}\r\n; Registry entries created or changed by the installation (SetupScope repackage).\r\n\r\n"
    );
    for k in keys {
        s.push_str(&format!("[{}]\r\n", long_path(&k.path)));
        for v in k.values.values() {
            s.push_str(&format!("{}={}\r\n", reg_name(&v.name), reg_data(v)));
        }
        s.push_str("\r\n");
    }
    let mut last = String::new();
    for (key, v) in values {
        if *key != last {
            s.push_str(&format!("\r\n[{}]\r\n", long_path(key)));
            last = key.clone();
        }
        s.push_str(&format!("{}={}\r\n", reg_name(&v.name), reg_data(v)));
    }
    s
}

fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The silent installer script.
pub fn install_script(
    log: &InstallLog,
    services: &[(String, String, u32, String)],
    path_entries: &[(bool, String)],
    folders: &[String],
) -> String {
    let mut s = format!(
        "# Silent installer for {name} {ver}, made by SetupScope (repackage).\r\n\
# Run in an elevated PowerShell:  powershell -ExecutionPolicy Bypass -File install.ps1\r\n\
#Requires -RunAsAdministrator\r\n\
$ErrorActionPreference = 'Stop'\r\n\
$root = Split-Path -Parent $MyInvocation.MyCommand.Path\r\n\r\n\
function Resolve-Folder([string]$name) {{\r\n\
    if ($name -like '_*') {{ return ($name.Substring(1) + ':\\') }}\r\n\
    $v = [Environment]::GetEnvironmentVariable($name)\r\n\
    if (-not $v) {{ throw \"Unknown folder variable $name\" }}\r\n\
    return $v\r\n\
}}\r\n\r\n\
# Files\r\n",
        name = log.name,
        ver = log.version
    );
    for f in folders {
        s.push_str(&format!(
            "$dest = Resolve-Folder {q}\r\nNew-Item -ItemType Directory -Force -Path $dest | Out-Null\r\nCopy-Item -Path (Join-Path $root (Join-Path 'files' {q}) | Join-Path -ChildPath '*') -Destination $dest -Recurse -Force\r\n",
            q = ps_quote(f)
        ));
    }
    if !services.is_empty() {
        s.push_str("\r\n# Services\r\n");
        for (name, image, start, display) in services {
            let start = match start {
                2 => "auto",
                4 => "disabled",
                _ => "demand",
            };
            s.push_str(&format!(
                "if (-not (Get-Service -Name {n} -ErrorAction SilentlyContinue)) {{ sc.exe create {n} binPath= {i} start= {st} DisplayName= {d} | Out-Null }}\r\n",
                n = ps_quote(name),
                i = ps_quote(image),
                st = start,
                d = ps_quote(display)
            ));
        }
    }
    s.push_str("\r\n# Registry\r\nreg.exe import (Join-Path $root 'install.reg') /reg:64\r\n");
    if !path_entries.is_empty() {
        s.push_str("\r\n# PATH\r\n");
        for (machine, entry) in path_entries {
            let scope = if *machine { "Machine" } else { "User" };
            s.push_str(&format!(
                "$p = [Environment]::GetEnvironmentVariable('Path', '{scope}')\r\n\
$e = [Environment]::ExpandEnvironmentVariables({q})\r\n\
if (($p -split ';') -notcontains $e) {{ [Environment]::SetEnvironmentVariable('Path', ($p.TrimEnd(';') + ';' + $e), '{scope}') }}\r\n",
                q = ps_quote(entry)
            ));
        }
    }
    s.push_str(&format!(
        "\r\nWrite-Host 'Installed {}.'\r\n",
        log.name.replace('\'', "")
    ));
    s
}

/// Write the folder. `out` is the parent folder; a subfolder named after
/// the program is created inside it.
pub fn repackage(
    log: &InstallLog,
    reg: &dyn Registry,
    out: &Path,
    progress: &dyn Fn(String),
) -> Result<Summary> {
    repackage_with(log, reg, out, progress, &Vars::current())
}

pub fn repackage_with(
    log: &InstallLog,
    reg: &dyn Registry,
    out: &Path,
    progress: &dyn Fn(String),
    vars: &Vars,
) -> Result<Summary> {
    let dir = out.join(crate::util::safe_file_name(
        format!("{} {}", log.name, log.version).trim(),
    ));
    std::fs::create_dir_all(dir.join("files"))
        .with_context(|| format!("can't create {}", dir.display()))?;
    let mut sum = Summary {
        folder: dir.clone(),
        ..Default::default()
    };
    let sysroot = fold(&expand_env("%SystemRoot%"));
    let cs = &log.changes;
    let mut folders = std::collections::BTreeSet::new();

    // Files, as they are now.
    for (n, f) in cs.files_added.iter().enumerate() {
        if !sysroot.contains('%') && fold(&f.path).starts_with(&sysroot) {
            continue;
        }
        let Some((var, rest)) = split_placeholder(&vars.to_placeholder(&f.path)) else {
            continue;
        };
        let rest = rest.replace(['\\', '/'], std::path::MAIN_SEPARATOR_STR);
        let dest = dir
            .join("files")
            .join(&var)
            .join(rest.trim_start_matches(std::path::MAIN_SEPARATOR));
        folders.insert(var);
        if n % 50 == 0 {
            progress(f.path.clone());
        }
        if f.is_dir {
            std::fs::create_dir_all(&dest)?;
            continue;
        }
        let src = Path::new(&f.path);
        if !src.is_file() {
            sum.missing += 1;
            continue;
        }
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        sum.bytes += std::fs::copy(src, &dest)
            .with_context(|| format!("could not copy captured file {}", src.display()))?;
        sum.files += 1;
    }

    // Registry: added keys as they are now, added and changed values.
    progress("registry".into());
    let mut keys: Vec<KeyData> = Vec::new();
    let mut services = Vec::new();
    for k in &cs.keys_added {
        if crate::util::is_protected_key(&k.path) {
            continue;
        }
        let current = reg.read_key(&k.path).unwrap_or_else(|| k.clone());
        let f = fold(&k.path);
        if let Some(name) = f
            .strip_prefix("hklm\\system\\currentcontrolset\\services\\")
            .filter(|r| !r.contains('\\'))
        {
            let _ = name;
            services.push((
                platform::leaf_name(&k.path).to_string(),
                current.get_string("ImagePath").unwrap_or_default(),
                current.get_u32("Start").unwrap_or(3),
                current
                    .get_string("DisplayName")
                    .unwrap_or_else(|| platform::leaf_name(&k.path).to_string()),
            ));
        }
        keys.push(current);
    }
    let mut values: Vec<(String, RegValue)> = Vec::new();
    let mut path_entries: Vec<(bool, String)> = Vec::new();
    for v in &cs.values_added {
        values.push((v.key.clone(), v.value.clone()));
    }
    for v in &cs.values_modified {
        // PATH edits become "append these entries" instead of a full overwrite.
        if v.after.name.eq_ignore_ascii_case("Path") && fold(&v.key).contains("environment") {
            let before: std::collections::BTreeSet<String> = v
                .before
                .as_string()
                .unwrap_or_default()
                .split(';')
                .map(|e| fold(e.trim()))
                .collect();
            for e in v.after.as_string().unwrap_or_default().split(';') {
                let e = e.trim();
                if !e.is_empty() && !before.contains(&fold(e)) {
                    path_entries.push((v.key.starts_with("HKLM"), vars.to_placeholder(e)));
                }
            }
            continue;
        }
        values.push((v.key.clone(), v.after.clone()));
    }
    sum.keys = keys.len();
    sum.values = values.len();
    sum.services = services.iter().map(|s| s.0.clone()).collect();

    let title = format!("{} {}", log.name, log.version);
    let write_utf16 = |p: PathBuf, text: &str| -> Result<()> {
        let mut b = vec![0xFF, 0xFE];
        for u in text.encode_utf16() {
            b.extend_from_slice(&u.to_le_bytes());
        }
        std::fs::write(p, b)?;
        Ok(())
    };
    write_utf16(
        dir.join("install.reg"),
        &to_reg_install(&keys, &values, &title),
    )?;
    let folders: Vec<String> = folders.into_iter().collect();
    std::fs::write(
        dir.join("install.ps1"),
        install_script(log, &services, &path_entries, &folders),
    )?;
    std::fs::write(dir.join("uninstall.ps1"), to_ps1(log))?;
    std::fs::write(
        dir.join("README.txt"),
        format!(
            "{title}\r\n\r\nRepackaged by SetupScope from a monitored installation on {}.\r\n\r\n\
install.ps1    installs silently: copies the files, registers services, imports install.reg\r\n\
               and adds PATH entries. Run it in an elevated PowerShell:\r\n\
               powershell -ExecutionPolicy Bypass -File install.ps1\r\n\
uninstall.ps1  removes everything again.\r\n\
install.reg    the registry entries on their own.\r\n\
files\\         the installed files, by destination folder (ProgramFiles, APPDATA, ...).\r\n\r\n\
Licences tied to the original PC may need to be activated again.\r\n",
            log.created
        ),
    )?;
    Ok(sum)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChangeSet, FileEntry, ValueChange};
    use crate::platform::MemRegistry;

    #[test]
    fn placeholders_split() {
        assert_eq!(
            split_placeholder("%ProgramFiles%\\Contoso\\a.exe").unwrap(),
            ("ProgramFiles".into(), "Contoso\\a.exe".into())
        );
        assert_eq!(
            split_placeholder("D:\\Tools\\x.exe").unwrap(),
            ("_D".into(), "Tools\\x.exe".into())
        );
        assert!(split_placeholder("relative\\x").is_none());
    }

    #[test]
    fn builds_a_package_folder() {
        let src = crate::util::tempdir().unwrap();
        let out = crate::util::tempdir().unwrap();
        let app = src.path().join("Contoso");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::write(app.join("app.exe"), b"bin").unwrap();
        let reg = MemRegistry::new();
        let key = "HKCU\\Software\\Contoso";
        reg.put(KeyData::new(key).with(RegValue::string("Theme", "dark")));
        let env = "HKCU\\Environment";
        let log = InstallLog {
            name: "Contoso".into(),
            version: "1.0".into(),
            changes: ChangeSet {
                files_added: vec![
                    FileEntry {
                        path: app.to_string_lossy().into(),
                        is_dir: true,
                        size: 0,
                        mtime: 0,
                    },
                    FileEntry {
                        path: app.join("app.exe").to_string_lossy().into(),
                        is_dir: false,
                        size: 3,
                        mtime: 0,
                    },
                    FileEntry {
                        path: app.join("gone.dll").to_string_lossy().into(),
                        is_dir: false,
                        size: 3,
                        mtime: 0,
                    },
                ],
                keys_added: vec![KeyData::new(key)],
                values_modified: vec![ValueChange {
                    key: env.into(),
                    before: RegValue::string("Path", "C:\\A"),
                    after: RegValue::string("Path", "C:\\A;C:\\Contoso\\bin"),
                }],
                ..Default::default()
            },
            ..Default::default()
        };
        let vars = Vars::from_pairs(vec![(
            "ProgramFiles".into(),
            src.path().to_string_lossy().into(),
        )]);
        let s = repackage_with(&log, &reg, out.path(), &|_| {}, &vars).unwrap();
        assert!(s
            .folder
            .join("files")
            .join("ProgramFiles")
            .join("Contoso")
            .join("app.exe")
            .exists());
        assert_eq!(s.files, 1);
        assert_eq!(s.missing, 1);
        assert_eq!(s.keys, 1);
        let dir = &s.folder;
        for f in ["install.reg", "install.ps1", "uninstall.ps1", "README.txt"] {
            assert!(dir.join(f).exists(), "{f}");
        }
        let script = std::fs::read_to_string(dir.join("install.ps1")).unwrap();
        assert!(script.contains("reg.exe import"));
        assert!(script.contains("C:\\Contoso\\bin"), "{script}");
        let reg_bytes = std::fs::read(dir.join("install.reg")).unwrap();
        let units: Vec<u16> = reg_bytes[2..]
            .chunks(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let reg_text = String::from_utf16_lossy(&units);
        assert!(reg_text.contains("\"Theme\"=\"dark\""), "{reg_text}");
    }
}
