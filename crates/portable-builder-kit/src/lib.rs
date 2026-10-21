//! Independent PortableApps.com build integration primitives; no additional runtime installation.
//!
//! The host supplies SHA-256 and UI policy; the optional process module supplies
//! bounded compiler execution without adding a shell or automatic elevation.
//! Static validation is preflight, not certification or a runtime portability test.
pub mod process;

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, Metadata};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Validate a single Windows-safe component, even when running on Linux.
pub fn validate_component(name: &str) -> io::Result<()> {
    if name.is_empty()
        || name == "."
        || name == ".."
        || name.ends_with([' ', '.'])
        || name
            .chars()
            .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
    {
        return Err(invalid("invalid Windows filename component"));
    }
    let stem = name
        .split('.')
        .next()
        .unwrap_or("")
        .trim_end()
        .to_uppercase();
    if ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"].contains(&stem.as_str())
        || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix).is_some_and(|n| {
                matches!(
                    n,
                    "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                )
            })
        })
    {
        return Err(invalid("reserved Windows device name"));
    }
    Ok(())
}

pub fn validate_app_id(id: &str) -> io::Result<()> {
    validate_component(id)?;
    if !id.as_bytes()[0].is_ascii_alphanumeric()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
    {
        return Err(invalid(
            "AppID must start with a letter/digit and contain only ASCII letters, digits, . _ + -",
        ));
    }
    Ok(())
}

/// Validate a relative Windows path without platform-dependent Path semantics.
pub fn relative_path(value: &str) -> io::Result<PathBuf> {
    let mut out = PathBuf::new();
    for part in value.split(['/', '\\']) {
        validate_component(part)?;
        out.push(part);
    }
    Ok(out)
}

fn plain_file(path: &Path) -> io::Result<Metadata> {
    let m = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if !m.is_file() || m.file_type().is_symlink() || reparse {
        return Err(invalid(format!(
            "not a regular, non-reparse file: {}",
            path.display()
        )));
    }
    Ok(m)
}

/// Bounded DOS/PE signature check. This checks structure, not trust or authenticity.
pub fn validate_executable(path: &Path) -> io::Result<()> {
    let m = plain_file(path)?;
    let mut f = File::open(path)?;
    let mut dos = [0u8; 64];
    f.read_exact(&mut dos)?;
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    if dos[..2] != *b"MZ" || offset < 64 || offset.checked_add(24).is_none_or(|end| end > m.len()) {
        return Err(invalid("invalid DOS/PE header"));
    }
    f.seek(SeekFrom::Start(offset))?;
    let mut coff = [0; 24];
    f.read_exact(&mut coff)?;
    if coff[..4] != *b"PE\0\0" {
        return Err(invalid("missing PE signature"));
    }
    Ok(())
}

/// One named, actionable static validation finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub path: PathBuf,
    pub message: String,
}

/// Read the small appinfo.ini and return its safe AppID. UTF-8/UTF-16LE supported.
/// Duplicate sections and keys are rejected rather than interpreted differently by tools.
pub fn package_app_id(package: &Path) -> io::Result<String> {
    let path = package.join("App/AppInfo/appinfo.ini");
    plain_file(&path)?;
    let mut bytes = Vec::new();
    File::open(&path)?.take(65537).read_to_end(&mut bytes)?;
    if bytes.len() > 65536 {
        return Err(invalid("appinfo.ini exceeds 64 KiB"));
    }
    let text = if bytes.starts_with(&[255, 254]) {
        if bytes.len() % 2 != 0 {
            return Err(invalid("truncated UTF-16LE"));
        }
        let units: Vec<u16> = bytes[2..]
            .chunks_exact(2)
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        String::from_utf16(&units).map_err(|_| invalid("invalid UTF-16LE"))?
    } else {
        String::from_utf8(
            bytes
                .strip_prefix(&[239, 187, 191])
                .unwrap_or(&bytes)
                .to_vec(),
        )
        .map_err(|_| invalid("appinfo.ini must be UTF-8 or UTF-16LE"))?
    };
    if text.contains('\0') {
        return Err(invalid("NUL in appinfo.ini"));
    }
    let mut section = String::new();
    let mut sections = BTreeSet::new();
    let mut fields = BTreeMap::new();
    for line in text
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.starts_with(';'))
    {
        if let Some(s) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = s.trim().to_ascii_lowercase();
            if section.is_empty() || !sections.insert(section.clone()) {
                return Err(invalid("duplicate/empty INI section"));
            }
        } else {
            let (k, v) = line
                .split_once('=')
                .ok_or_else(|| invalid("invalid INI line"))?;
            let key = (section.clone(), k.trim().to_ascii_lowercase());
            if section.is_empty()
                || key.1.is_empty()
                || fields.insert(key, v.trim().to_string()).is_some()
            {
                return Err(invalid("duplicate key or key outside INI section"));
            }
        }
    }
    let get = |s: &str, k: &str| fields.get(&(s.to_string(), k.to_string()));
    if get("format", "type").map(String::as_str) != Some("PortableApps.comFormat") {
        return Err(invalid("missing PortableApps.comFormat type"));
    }
    let id = get("details", "appid").ok_or_else(|| invalid("missing AppID"))?;
    validate_app_id(id)?;
    let start = format!("{id}.exe");
    if get("control", "start").map(String::as_str) != Some(start.as_str()) {
        return Err(invalid("Control Start must name the package launcher"));
    }
    Ok(id.clone())
}

/// Check required package assets and (when requested) the compiled launcher.
/// Does not validate every format field, image encoding, licensing or application behavior.
pub fn preflight(package: &Path, require_launcher: bool) -> Vec<Finding> {
    let mut findings = Vec::new();
    let id = match package_app_id(package) {
        Ok(id) => Some(id),
        Err(e) => {
            findings.push(Finding {
                path: package.join("App/AppInfo/appinfo.ini"),
                message: e.to_string(),
            });
            None
        }
    };
    for rel in [
        "help.html",
        "App/AppInfo/appicon.ico",
        "App/AppInfo/appicon_16.png",
        "App/AppInfo/appicon_32.png",
        "App/AppInfo/appicon_128.png",
    ] {
        if let Err(e) = plain_file(&package.join(rel)) {
            findings.push(Finding {
                path: package.join(rel),
                message: e.to_string(),
            });
        }
    }
    if let Some(id) = id {
        let launcher = if require_launcher {
            package.join(format!("{id}.exe"))
        } else {
            package.join(format!("App/AppInfo/Launcher/{id}.ini"))
        };
        let check = if require_launcher {
            validate_executable(&launcher)
        } else {
            plain_file(&launcher).map(|_| ())
        };
        if let Err(e) = check {
            findings.push(Finding {
                path: launcher,
                message: e.to_string(),
            });
        }
    }
    findings
}

/// SHA-256 provider from the host's maintained crypto implementation.
/// The callback must hash the complete file and propagate read failures.
pub type Sha256 = [u8; 32];

/// Capture candidate artifacts before running a compiler. Unlike a timestamp
/// cutoff, content fingerprints do not accept unchanged recent/stale output.
/// Use private build directories; this API is not a concurrency sandbox.
pub struct ArtifactWatch {
    before: BTreeMap<PathBuf, (Sha256, SystemTime)>,
    directories: Vec<PathBuf>,
    prefix: String,
}

impl ArtifactWatch {
    pub fn capture(
        directories: &[PathBuf],
        app_id: &str,
        hash: &impl Fn(&Path) -> io::Result<Sha256>,
    ) -> io::Result<Self> {
        validate_app_id(app_id)?;
        let directories: BTreeSet<PathBuf> = directories
            .iter()
            .map(fs::canonicalize)
            .collect::<io::Result<_>>()?;
        let mut watch = Self {
            before: BTreeMap::new(),
            directories: directories.into_iter().collect(),
            prefix: format!("{app_id}_").to_ascii_lowercase(),
        };
        for p in watch.candidates()? {
            watch
                .before
                .insert(p.clone(), (hash(&p)?, plain_file(&p)?.modified()?));
        }
        Ok(watch)
    }

    fn candidates(&self) -> io::Result<Vec<PathBuf>> {
        let mut paths = Vec::new();
        for dir in &self.directories {
            let mut count = 0;
            for entry in fs::read_dir(dir)? {
                count += 1;
                if count > 10000 {
                    return Err(invalid(
                        "build directory exceeds 10000 entries; use an isolated build directory",
                    ));
                }
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if name.starts_with(&self.prefix) && name.ends_with(".paf.exe") {
                    plain_file(&entry.path())?;
                    paths.push(entry.path());
                }
            }
        }
        Ok(paths)
    }

    /// Require successful process completion and exactly one new/changed artifact.
    /// Byte-identical output is accepted only when its modification time changed.
    /// Private build directories prevent unrelated jobs from touching candidates.
    pub fn finish(
        self,
        exit_code: u32,
        hash: &impl Fn(&Path) -> io::Result<Sha256>,
    ) -> io::Result<BuildArtifact> {
        require_success("PortableApps.com Installer", exit_code)?;
        let mut changed = Vec::new();
        for path in self.candidates()? {
            let sha256 = hash(&path)?;
            let modified = plain_file(&path)?.modified()?;
            if self.before.get(&path) != Some(&(sha256, modified)) {
                changed.push(BuildArtifact { path, sha256 });
            }
        }
        if changed.len() != 1 {
            return Err(invalid(format!("expected exactly one new/changed installer for {}; found {} (use an isolated build directory)", self.prefix, changed.len())));
        }
        let artifact = changed.pop().unwrap();
        validate_executable(&artifact.path)?;
        Ok(artifact)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildArtifact {
    pub path: PathBuf,
    pub sha256: Sha256,
}

pub fn require_success(tool: &str, exit_code: u32) -> io::Result<()> {
    if exit_code != 0 {
        return Err(io::Error::other(format!(
            "{tool} failed with exit code {exit_code}"
        )));
    }
    Ok(())
}
