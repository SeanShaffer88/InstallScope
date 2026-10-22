//! Numeric compatibility from embedded Windows product-version resources only.
//! No executable, shell, wrapper, build script, or package manager is run.
use anyhow::{bail, Context, Result};
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub [u16; 4]);
impl Version {
    pub fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if text.is_empty() || text.len() > 23 {
            bail!("Use 1–4 numeric version components (0–65535), such as 22.0. No wildcards, operators or prerelease tags.");
        }
        let parts: Vec<_> = text.split('.').collect();
        if parts.len() > 4
            || parts
                .iter()
                .any(|p| p.is_empty() || !p.bytes().all(|c| c.is_ascii_digit()))
        {
            bail!("Use 1–4 numeric version components, such as 22.0. No wildcards, operators or prerelease tags.");
        }
        let mut value = [0; 4];
        for (index, part) in parts.into_iter().enumerate() {
            value[index] = part
                .parse()
                .context("Version components must be between 0 and 65535")?;
        }
        Ok(Self(value))
    }
}
impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}.{}", self.0[0], self.0[1], self.0[2], self.0[3])
    }
}

#[derive(Clone, Debug)]
pub struct Range {
    pub minimum: Option<Version>,
    pub maximum_exclusive: Option<Version>,
}
impl Range {
    pub fn parse(minimum: &str, maximum: &str) -> Result<Self> {
        let parse = |s: &str| {
            if s.trim().is_empty() {
                Ok(None)
            } else {
                Version::parse(s).map(Some)
            }
        };
        let range = Self {
            minimum: parse(minimum)?,
            maximum_exclusive: parse(maximum)?,
        };
        if range
            .minimum
            .zip(range.maximum_exclusive)
            .is_some_and(|(a, b)| a >= b)
        {
            bail!("Minimum version must be lower than the exclusive maximum version");
        }
        Ok(range)
    }
    fn contains(&self, value: Version) -> bool {
        self.minimum.is_none_or(|min| value >= min)
            && self.maximum_exclusive.is_none_or(|max| value < max)
    }
}

#[derive(Clone, Debug)]
pub struct Evidence {
    pub version: Version,
    pub prerelease: bool,
    pub identity: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compatibility {
    Matches,
    Outside,
    NoRange,
    Unknown,
}
impl Compatibility {
    pub fn label(self) -> &'static str {
        match self {
            Self::Matches => "Numeric metadata matches requested range",
            Self::Outside => "Numeric metadata outside requested range",
            Self::NoRange => "Metadata available — no range requested",
            Self::Unknown => "Compatibility unknown",
        }
    }
}

pub fn assess(command: &str, evidence: &Evidence, range: &Range) -> (Compatibility, &'static str) {
    let name = command.to_ascii_lowercase();
    let stem = name.strip_suffix(".exe").unwrap_or(&name);
    let identity = evidence.identity.to_ascii_lowercase();
    if ["rustup", "launcher", "shim", "proxy"]
        .iter()
        .any(|name| identity.contains(name))
        || matches!(stem, "rustc" | "cargo" | "py" | "dotnet")
    {
        return (Compatibility::Unknown, "A proxy, launcher or host version does not establish the selected compiler, interpreter or SDK version. No tool was executed to resolve it.");
    }
    if evidence.prerelease {
        return (Compatibility::Unknown, "Prerelease, private or special-build metadata cannot establish release-version compatibility.");
    }
    if evidence.identity.trim().is_empty() {
        return (
            Compatibility::Unknown,
            "No product identity strings were available to contextualize this numeric metadata.",
        );
    }
    if evidence.version.0 == [0; 4] {
        return (
            Compatibility::Unknown,
            "Product-version metadata is zero/unspecified.",
        );
    }
    if range.minimum.is_none() && range.maximum_exclusive.is_none() {
        return (
            Compatibility::NoRange,
            "Embedded numeric product version only; no requirement was supplied.",
        );
    }
    (if range.contains(evidence.version) { Compatibility::Matches } else { Compatibility::Outside }, "Comparison applies only to this file's embedded numeric product version. It does not prove identity, integrity, project compatibility, runtime behavior, or that this candidate wins shell resolution.")
}

#[cfg(windows)]
pub fn read(path: &Path) -> Result<Evidence> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
    };
    let ext = path.extension().and_then(|v| v.to_str()).unwrap_or("");
    if !ext.eq_ignore_ascii_case("exe") && !ext.eq_ignore_ascii_case("com") {
        bail!("Script/wrapper or unsupported file type; never executed to obtain a version");
    }
    crate::devcheck::version_path_guard(path)?;
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut unused = 0;
    let size = unsafe { GetFileVersionInfoSizeW(wide.as_ptr(), &mut unused) };
    if size == 0 {
        bail!(
            "No readable embedded version resource ({})",
            std::io::Error::last_os_error()
        );
    }
    if size > 512 * 1024 {
        bail!("Version resource exceeds the 512 KiB inspection limit");
    }
    let mut data = vec![0u32; (size as usize).div_ceil(4)];
    if unsafe { GetFileVersionInfoW(wide.as_ptr(), 0, size, data.as_mut_ptr().cast()) } == 0 {
        bail!(
            "Cannot read version resource: {}",
            std::io::Error::last_os_error()
        );
    }
    fn query(data: &[u32], size: usize, key: &str) -> Option<(*const u8, usize)> {
        let key: Vec<u16> = key.encode_utf16().chain(Some(0)).collect();
        let mut pointer = std::ptr::null_mut();
        let mut count = 0;
        if unsafe { VerQueryValueW(data.as_ptr().cast(), key.as_ptr(), &mut pointer, &mut count) }
            == 0
            || pointer.is_null()
        {
            return None;
        }
        let offset = (pointer as usize).checked_sub(data.as_ptr() as usize)?;
        if offset >= size {
            return None;
        }
        Some((pointer.cast(), count as usize))
    }
    fn inside(data: &[u32], size: usize, pointer: *const u8, bytes: usize) -> bool {
        (pointer as usize)
            .checked_sub(data.as_ptr() as usize)
            .and_then(|offset| offset.checked_add(bytes))
            .is_some_and(|end| end <= size)
    }
    let (fixed, length) =
        query(&data, size as usize, "\\").context("Missing fixed product-version metadata")?;
    if length < std::mem::size_of::<VS_FIXEDFILEINFO>()
        || !inside(
            &data,
            size as usize,
            fixed,
            std::mem::size_of::<VS_FIXEDFILEINFO>(),
        )
    {
        bail!("Invalid fixed version resource bounds");
    }
    let fixed = unsafe { std::ptr::read_unaligned(fixed.cast::<VS_FIXEDFILEINFO>()) };
    if fixed.dwSignature != 0xFEEF04BD {
        bail!("Invalid version resource signature");
    }
    let ms = fixed.dwProductVersionMS;
    let ls = fixed.dwProductVersionLS;
    let mut identity = String::new();
    if let Some((translations, bytes)) = query(&data, size as usize, "\\VarFileInfo\\Translation") {
        let bytes = bytes.min(16 * 4);
        if inside(&data, size as usize, translations, bytes) {
            for index in 0..bytes / 4 {
                let lang =
                    unsafe { std::ptr::read_unaligned(translations.add(index * 4).cast::<u16>()) };
                let page = unsafe {
                    std::ptr::read_unaligned(translations.add(index * 4 + 2).cast::<u16>())
                };
                for field in ["ProductName", "OriginalFilename", "FileDescription"] {
                    let key = format!("\\StringFileInfo\\{lang:04x}{page:04x}\\{field}");
                    if let Some((text, count)) = query(&data, size as usize, &key) {
                        if count <= 4096 && inside(&data, size as usize, text, count * 2) {
                            let text: Vec<u16> = (0..count)
                                .map(|i| unsafe {
                                    std::ptr::read_unaligned(text.add(i * 2).cast::<u16>())
                                })
                                .take_while(|c| *c != 0)
                                .collect();
                            identity.push_str(&String::from_utf16_lossy(&text));
                            identity.push(' ');
                        }
                    }
                }
            }
        }
    }
    Ok(Evidence {
        version: Version([(ms >> 16) as u16, ms as u16, (ls >> 16) as u16, ls as u16]),
        prerelease: fixed.dwFileFlags & fixed.dwFileFlagsMask & (2 | 8 | 32) != 0,
        identity,
    })
}

#[cfg(not(windows))]
pub fn read(_path: &Path) -> Result<Evidence> {
    bail!("Embedded Windows product-version reading is available only on Windows");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ranges_compare_numerically_with_inclusive_minimum_and_exclusive_maximum() {
        let range = Range::parse("2.9", "3").unwrap();
        for (version, matches) in [
            ("2.8.99", false),
            ("2.9", true),
            ("2.10", true),
            ("3", false),
        ] {
            assert_eq!(range.contains(Version::parse(version).unwrap()), matches);
        }
        for value in [
            "1..2",
            "1.2.3.4.5",
            "-1",
            "65536",
            "1.2-beta",
            ">=2",
            "*",
            "é",
        ] {
            assert!(Version::parse(value).is_err());
        }
        assert!(Range::parse("3", "2").is_err());
        assert!(Range::parse("3", "3").is_err());
        assert!(Range::parse("", "2")
            .unwrap()
            .contains(Version::parse("1").unwrap()));
    }
    #[test]
    fn proxies_missing_release_identity_and_special_builds_cannot_pass() {
        let range = Range::parse("1", "3").unwrap();
        let mut evidence = Evidence {
            version: Version::parse("2").unwrap(),
            prerelease: false,
            identity: "Real product".into(),
        };
        assert_eq!(assess("node", &evidence, &range).0, Compatibility::Matches);
        for name in ["rustc", "cargo.exe", "py", "dotnet"] {
            assert_eq!(assess(name, &evidence, &range).0, Compatibility::Unknown);
        }
        evidence.identity = "rustup.exe".into();
        assert_eq!(
            assess("renamed", &evidence, &range).0,
            Compatibility::Unknown
        );
        evidence.identity.clear();
        evidence.prerelease = true;
        assert_eq!(assess("node", &evidence, &range).0, Compatibility::Unknown);
        evidence.prerelease = false;
        evidence.version = Version([0; 4]);
        assert_eq!(assess("node", &evidence, &range).0, Compatibility::Unknown);
    }
    #[test]
    fn scripts_and_missing_files_never_execute_or_get_assumed_versions() {
        let tmp = crate::util::tempdir().unwrap();
        let marker = tmp.path().join("must-not-exist");
        let script = tmp.path().join("compiler.cmd");
        std::fs::write(&script, format!("echo fail > {}", marker.display())).unwrap();
        assert!(read(&script).is_err());
        assert!(read(&tmp.path().join("absent.exe")).is_err());
        assert!(!marker.exists());
        let fake = tmp.path().join("fake.exe");
        std::fs::write(&fake, b"not PE metadata").unwrap();
        assert!(read(&fake).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn reads_real_windows_resource_without_executing_the_program() {
        let root = std::env::var_os("SystemRoot").unwrap();
        let evidence = read(&std::path::PathBuf::from(root).join("System32/cmd.exe")).unwrap();
        assert_ne!(evidence.version, Version([0; 4]));
        assert!(!evidence.identity.is_empty());
    }
}
