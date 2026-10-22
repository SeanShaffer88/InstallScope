//! Read-only developer prerequisite and PATH inspection. Never executes tools,
//! reads project scripts, changes the environment, or downloads anything.
use crate::snapshot::Progress;
use anyhow::{bail, Result};
use std::fs;
use std::path::{Component, Path, PathBuf};

const PATH_LIMIT: usize = 256;
const CANDIDATE_LIMIT: usize = 64;
const DISCOVERY_LIMIT: usize = 256;
pub const TOOLS: &[&str] = &[
    "git", "cargo", "rustc", "dotnet", "node", "npm", "python", "python3", "py", "pip", "cmake",
    "ninja", "cl", "link", "msbuild", "gcc", "g++", "clang", "clang++",
];

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Profile {
    #[default]
    RustMsvc,
    DotNet,
    Node,
    Python,
    Cmake,
}
impl Profile {
    pub const ALL: [Self; 5] = [
        Self::RustMsvc,
        Self::DotNet,
        Self::Node,
        Self::Python,
        Self::Cmake,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::RustMsvc => "Rust / MSVC",
            Self::DotNet => ".NET",
            Self::Node => "Node.js",
            Self::Python => "Python",
            Self::Cmake => "CMake / C++",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Target {
    #[default]
    X64,
    Arm64,
}
impl Target {
    pub fn label(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::Arm64 => "arm64",
        }
    }
    fn rust(self) -> &'static str {
        match self {
            Self::X64 => "x86_64-pc-windows-msvc",
            Self::Arm64 => "aarch64-pc-windows-msvc",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Found,
    Review,
    Missing,
    Unknown,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Found => "Found",
            Self::Review => "Review",
            Self::Missing => "Not found in checked locations",
            Self::Unknown => "Unknown / partial",
        }
    }
}

#[derive(Clone, Debug)]
pub struct PathEntry {
    pub order: usize,
    pub raw: String,
    pub path: Option<PathBuf>,
    pub status: Status,
    pub detail: String,
}
#[derive(Clone, Debug)]
pub struct Tool {
    pub name: String,
    pub candidates: Vec<PathBuf>,
    pub notes: Vec<String>,
}
#[derive(Clone, Debug)]
pub struct Check {
    pub name: String,
    pub status: Status,
    pub evidence: Vec<String>,
    pub guidance: &'static str,
}
#[derive(Clone, Debug)]
pub struct Report {
    pub profile: Profile,
    pub target: Target,
    pub entries: Vec<PathEntry>,
    pub tools: Vec<Tool>,
    pub checks: Vec<Check>,
    pub notes: Vec<String>,
    pub cancelled: bool,
    pub path_partial: bool,
    pub versions: Vec<VersionCandidate>,
    pub version_command: String,
    pub version_range: crate::devversion::Range,
}

#[derive(Clone, Debug)]
pub struct VersionCandidate {
    pub path: PathBuf,
    pub evidence: Option<crate::devversion::Evidence>,
    pub compatibility: crate::devversion::Compatibility,
    pub detail: String,
}

#[derive(Default, Clone)]
pub struct Request {
    pub profile: Profile,
    pub target: Target,
    pub path: String,
    pub pathext: String,
    pub command: String,
    pub vs_roots: Vec<PathBuf>,
    pub vc_tools: Option<PathBuf>,
    pub sdk_roots: Vec<PathBuf>,
    pub rustup_home: Option<PathBuf>,
    pub minimum_version: String,
    pub maximum_version: String,
}
impl Request {
    pub fn current(profile: Profile, target: Target, command: String) -> Result<Self> {
        let path = match std::env::var("PATH") {
            Ok(s) => s,
            Err(std::env::VarError::NotPresent) => String::new(),
            Err(error) => return Err(error.into()),
        };
        let pathext = std::env::var("PATHEXT").unwrap_or_default();
        let env_path = |key: &str| {
            std::env::var_os(key)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
        };
        let vs_roots = ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(|key| env_path(key))
            .map(|p| p.join("Microsoft Visual Studio"))
            .collect();
        let mut sdk_roots: Vec<_> = env_path("WindowsSdkDir").into_iter().collect();
        if let Some(p) = env_path("ProgramFiles(x86)") {
            sdk_roots.push(p.join("Windows Kits/10"));
        }
        Ok(Self {
            profile,
            target,
            path,
            pathext,
            command,
            vs_roots,
            vc_tools: env_path("VCToolsInstallDir"),
            sdk_roots,
            rustup_home: env_path("RUSTUP_HOME")
                .or_else(|| env_path("USERPROFILE").map(|p| p.join(".rustup"))),
            minimum_version: String::new(),
            maximum_version: String::new(),
        })
    }
}

#[derive(Debug)]
pub(crate) enum Probe {
    File,
    Dir,
    Missing,
    Unknown(String),
}

fn reparse(meta: &fs::Metadata) -> bool {
    if meta.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.file_attributes() & 0x400 != 0 {
            return true;
        }
    }
    false
}

/// Reject network/device paths before any filesystem query, and observed links
/// along the path before following descendants. Concurrent changes can race
/// these read-only checks; this is not a containment/security boundary.
fn local_path(path: &Path) -> Result<()> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("Relative or parent-traversing path skipped");
    }
    if path.components().count() > 256 {
        bail!("Path component limit reached");
    }
    #[cfg(windows)]
    {
        use std::path::Prefix;
        use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
        const DRIVE_REMOTE: u32 = 4;
        let Some(Component::Prefix(prefix)) = path.components().next() else {
            bail!("Unsupported path prefix");
        };
        let drive = match prefix.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => d,
            _ => bail!("Network or device path skipped"),
        };
        let root: Vec<u16> = format!("{}:\\", drive as char)
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let kind = unsafe { GetDriveTypeW(root.as_ptr()) };
        if kind == DRIVE_REMOTE {
            bail!("Mapped network drive skipped");
        }
        if ![2, 3, 5, 6].contains(&kind) {
            bail!("Drive type unavailable; metadata inspection skipped");
        }
    }
    #[cfg(not(windows))]
    {
        if path.to_string_lossy().starts_with("//") {
            bail!("Network path skipped");
        }
    }
    Ok(())
}

pub(crate) fn probe(path: &Path) -> Probe {
    if let Err(error) = local_path(path) {
        return Probe::Unknown(error.to_string());
    }
    for ancestor in path.ancestors().collect::<Vec<_>>().into_iter().rev() {
        match fs::symlink_metadata(ancestor) {
            Ok(meta) if reparse(&meta) => {
                return Probe::Unknown("Link, alias, or reparse point skipped".into())
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Probe::Missing,
            Err(e) => return Probe::Unknown(e.to_string()),
        }
    }
    leaf(path)
}

pub(crate) fn version_path_guard(path: &Path) -> Result<()> {
    match probe(path) {
        Probe::File => Ok(()),
        other => bail!("Version inspection skipped: {other:?}"),
    }
}

fn leaf(path: &Path) -> Probe {
    match fs::symlink_metadata(path) {
        Ok(meta) if reparse(&meta) => {
            Probe::Unknown("Link, alias, or reparse point skipped".into())
        }
        Ok(meta) if meta.is_file() => Probe::File,
        Ok(meta) if meta.is_dir() => Probe::Dir,
        Ok(_) => Probe::Unknown("Special filesystem entry skipped".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Probe::Missing,
        Err(e) => Probe::Unknown(e.to_string()),
    }
}

fn valid_command(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 64
        || !name.bytes().next().unwrap().is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._+-".contains(&b))
    {
        return false;
    }
    let stem = name.split('.').next().unwrap().to_ascii_uppercase();
    !matches!(
        stem.as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

fn same_path(a: &Path, b: &Path) -> bool {
    #[cfg(windows)]
    {
        use windows_sys::Win32::Globalization::CompareStringOrdinal;
        let text = |p: &Path| {
            p.to_string_lossy()
                .replace('/', "\\")
                .trim_end_matches('\\')
                .encode_utf16()
                .collect::<Vec<_>>()
        };
        let a = text(a);
        let b = text(b);
        unsafe {
            CompareStringOrdinal(a.as_ptr(), a.len() as i32, b.as_ptr(), b.len() as i32, 1) == 2
        }
    }
    #[cfg(not(windows))]
    {
        a == b
    }
}

fn path_parts(input: &str) -> impl Iterator<Item = &str> {
    let mut remaining = Some(input);
    std::iter::from_fn(move || {
        let current = remaining.take()?;
        let mut quoted = false;
        for (index, ch) in current.char_indices() {
            if ch == '"' {
                quoted = !quoted;
            }
            if ch == ';' && !quoted {
                remaining = Some(&current[index + 1..]);
                return Some(&current[..index]);
            }
        }
        Some(current)
    })
}

pub fn inspect(request: &Request, progress: &Progress) -> Result<Report> {
    let version_range =
        crate::devversion::Range::parse(&request.minimum_version, &request.maximum_version)?;
    if request.path.len() > 128 * 1024 || request.pathext.len() > 4096 {
        bail!("PATH or PATHEXT exceeds the inspection limit");
    }
    let custom = request.command.trim();
    if !custom.is_empty() && !valid_command(custom) {
        bail!("Enter one command name only (up to 64 ASCII letters, digits, dots, underscores, + or -). Paths, arguments, and Windows device names are not accepted.");
    }
    let version_command = if custom.is_empty() {
        match request.profile {
            Profile::RustMsvc => "rustc",
            Profile::DotNet => "dotnet",
            Profile::Node => "node",
            Profile::Python => "python",
            Profile::Cmake => "cmake",
        }
    } else {
        custom
    };
    let mut report = Report {
        profile: request.profile,
        target: request.target,
        entries: vec![],
        tools: vec![],
        checks: vec![],
        notes: vec![],
        cancelled: false,
        path_partial: false,
        versions: vec![],
        version_command: version_command.into(),
        version_range,
    };
    let mut extensions = Vec::new();
    for ext in request.pathext.split(';').map(str::trim) {
        let ext = ext.to_ascii_lowercase();
        if ext.len() < 2
            || ext.len() > 16
            || !ext.starts_with('.')
            || !ext[1..].bytes().all(|b| b.is_ascii_alphanumeric())
        {
            continue;
        }
        if !extensions.contains(&ext) && extensions.len() < 8 {
            extensions.push(ext);
        }
    }
    if extensions.is_empty() {
        extensions = vec![".com".into(), ".exe".into(), ".bat".into(), ".cmd".into()];
        report.notes.push(
            "PATHEXT was empty or invalid; using .COM;.EXE;.BAT;.CMD as an inspection assumption."
                .into(),
        );
        report.path_partial = true;
    } else if request
        .pathext
        .split(';')
        .filter(|s| !s.trim().is_empty())
        .count()
        > extensions.len()
    {
        report
            .notes
            .push("Duplicate, invalid, or excess PATHEXT entries were omitted (maximum 8).".into());
        report.path_partial = true;
    }
    report.notes.push(format!(
        "Inspected extension order: {}. No files were executed.",
        extensions.join(";")
    ));
    let mut seen: Vec<(PathBuf, usize)> = vec![];
    for (index, raw) in path_parts(&request.path).enumerate() {
        if progress.cancelled() {
            report.cancelled = true;
            break;
        }
        if index == PATH_LIMIT {
            report.path_partial = true;
            report
                .notes
                .push("PATH inspection stopped at 256 entries.".into());
            break;
        }
        progress.tick();
        let raw = raw.trim();
        let expanded = crate::platform::expand_env(
            raw.strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .unwrap_or(raw),
        );
        let path = PathBuf::from(&expanded);
        let (status, detail) = if expanded.is_empty() || expanded.contains(['%', '"', '\0']) {
            (Status::Unknown, "Empty, unresolved, or malformed entry; shell current-directory behavior is not modeled.".into())
        } else if !path.is_absolute()
            || path.components().any(|c| matches!(c, Component::ParentDir))
        {
            (
                Status::Unknown,
                "Relative or parent-traversing PATH entry skipped".into(),
            )
        } else if let Some((_, previous)) = seen.iter().find(|(p, _)| same_path(p, &path)) {
            (
                Status::Review,
                format!("Duplicate of PATH entry {previous}; searched once."),
            )
        } else {
            match probe(&path) {
                Probe::Dir => {
                    seen.push((path.clone(), index + 1));
                    (Status::Found, "Local directory observed".into())
                }
                Probe::Missing => (
                    Status::Missing,
                    "Directory is missing; stale PATH entry".into(),
                ),
                Probe::File => (
                    Status::Review,
                    "PATH entry is a file, not a directory".into(),
                ),
                Probe::Unknown(reason) => (Status::Unknown, reason),
            }
        };
        if status == Status::Unknown {
            report.path_partial = true;
        }
        report.entries.push(PathEntry {
            order: index + 1,
            raw: raw.into(),
            path: (status == Status::Found).then_some(path),
            status,
            detail,
        });
    }
    let mut names: Vec<_> = TOOLS.iter().map(|s| s.to_string()).collect();
    if !custom.is_empty() && !names.iter().any(|s| s.eq_ignore_ascii_case(custom)) {
        names.push(custom.into());
    }
    for name in names {
        let mut tool = Tool {
            name: name.clone(),
            candidates: vec![],
            notes: vec![],
        };
        let suffixes = if Path::new(&name).extension().is_some() {
            vec![String::new()]
        } else {
            extensions.clone()
        };
        'directories: for entry in &report.entries {
            let Some(dir) = &entry.path else {
                continue;
            };
            for ext in &suffixes {
                if progress.cancelled() {
                    report.cancelled = true;
                    break 'directories;
                }
                progress.tick();
                let file = dir.join(format!("{name}{ext}"));
                match leaf(&file) {
                    Probe::File => {
                        if tool.candidates.len() == CANDIDATE_LIMIT {
                            tool.notes
                                .push("Candidate limit (64) reached; results are partial.".into());
                            break 'directories;
                        }
                        tool.candidates.push(file);
                    }
                    Probe::Unknown(reason) if tool.notes.len() < 16 => {
                        tool.notes.push(format!("{}: {reason}", file.display()))
                    }
                    _ => {}
                }
            }
        }
        report.tools.push(tool);
        if report.cancelled {
            break;
        }
    }
    if !report.cancelled {
        readiness(request, progress, &mut report);
    }
    if !report.cancelled {
        if let Some(tool) = report
            .tools
            .iter()
            .find(|t| t.name.eq_ignore_ascii_case(version_command))
        {
            for path in &tool.candidates {
                if progress.cancelled() {
                    report.cancelled = true;
                    break;
                }
                progress.tick();
                let (evidence, compatibility, detail) = match crate::devversion::read(path) {
                    Ok(evidence) => {
                        let (compatibility, detail) = crate::devversion::assess(
                            version_command,
                            &evidence,
                            &report.version_range,
                        );
                        (Some(evidence), compatibility, detail.into())
                    }
                    Err(error) => (
                        None,
                        crate::devversion::Compatibility::Unknown,
                        format!("{error:#}"),
                    ),
                };
                report.versions.push(VersionCandidate {
                    path: path.clone(),
                    evidence,
                    compatibility,
                    detail,
                });
            }
        }
    }
    report.cancelled |= progress.cancelled();
    Ok(report)
}

struct Discovery<'a> {
    remaining: usize,
    progress: &'a Progress,
    issues: Vec<String>,
}
impl Discovery<'_> {
    fn dirs(&mut self, path: &Path) -> Vec<PathBuf> {
        if self.remaining == 0 || self.progress.cancelled() {
            self.note("Discovery limit or cancellation reached".into());
            return vec![];
        }
        self.remaining -= 1;
        self.progress.tick();
        match probe(path) {
            Probe::Dir => {}
            Probe::Missing => return vec![],
            other => {
                self.note(format!("{}: {other:?}", path.display()));
                return vec![];
            }
        }
        let entries = match fs::read_dir(path) {
            Ok(e) => e,
            Err(error) => {
                self.note(format!("{}: {error}", path.display()));
                return vec![];
            }
        };
        let mut dirs = Vec::new();
        for (i, entry) in entries.enumerate() {
            if i == 64 || self.progress.cancelled() {
                self.note("Directory discovery truncated at 64 entries or cancelled".into());
                break;
            }
            match entry {
                Ok(e) => match leaf(&e.path()) {
                    Probe::Dir => dirs.push(e.path()),
                    Probe::Unknown(error) => self.note(format!("{}: {error}", e.path().display())),
                    _ => {}
                },
                Err(e) => self.note(e.to_string()),
            }
        }
        dirs.sort();
        dirs
    }
    fn note(&mut self, note: String) {
        if self.issues.len() < 32 {
            self.issues.push(note);
        }
    }
    fn files(&mut self, root: &Path, files: &[String]) -> bool {
        let mut complete = true;
        for file in files {
            if self.remaining == 0 || self.progress.cancelled() {
                self.note("Discovery limit or cancellation reached".into());
                return false;
            }
            self.remaining -= 1;
            self.progress.tick();
            match probe(&root.join(file)) {
                Probe::File => {}
                Probe::Unknown(error) => {
                    self.note(format!("{}: {error}", root.join(file).display()));
                    complete = false;
                }
                _ => complete = false,
            }
        }
        complete
    }
}

fn readiness(request: &Request, progress: &Progress, report: &mut Report) {
    let required: &[(&str, &[&str])] = match request.profile {
        Profile::RustMsvc => &[
            ("Cargo PATH candidate", &["cargo"]),
            ("Rust compiler PATH candidate", &["rustc"]),
        ],
        Profile::DotNet => &[(".NET host PATH candidate", &["dotnet"])],
        Profile::Node => &[
            ("Node.js PATH candidate", &["node"]),
            ("npm PATH candidate", &["npm"]),
        ],
        Profile::Python => &[("Python interpreter PATH candidate", &["python", "python3"])],
        Profile::Cmake => &[
            ("CMake PATH candidate", &["cmake"]),
            ("Generator PATH candidate", &["ninja", "msbuild"]),
        ],
    };
    for (name, commands) in required {
        let tools: Vec<_> = report
            .tools
            .iter()
            .filter(|t| commands.contains(&t.name.as_str()))
            .collect();
        let evidence: Vec<_> = tools
            .iter()
            .flat_map(|t| t.candidates.iter().map(|p| p.display().to_string()))
            .collect();
        let uncertain = report.path_partial || tools.iter().any(|t| !t.notes.is_empty());
        let status = if evidence.is_empty() {
            if uncertain {
                Status::Unknown
            } else {
                Status::Missing
            }
        } else if uncertain || evidence.len() > 1 {
            Status::Review
        } else {
            Status::Found
        };
        report.checks.push(Check { name: name.to_string(), status, evidence, guidance: "Candidate presence does not verify its version, architecture, dependencies, or successful execution. Restart SetupScope from the intended developer environment after changing it." });
    }
    let mut discovery = Discovery {
        remaining: DISCOVERY_LIMIT,
        progress,
        issues: vec![],
    };
    let arch = request.target.label();
    if matches!(request.profile, Profile::RustMsvc | Profile::Cmake) {
        let mut roots: Vec<_> = request.vc_tools.iter().cloned().collect();
        for root in &request.vs_roots {
            for version in discovery.dirs(root) {
                for edition in discovery.dirs(&version) {
                    roots.extend(discovery.dirs(&edition.join("VC/Tools/MSVC")));
                }
            }
        }
        roots.sort();
        roots.dedup();
        let mut evidence = vec![];
        for root in roots {
            for host in ["Hostx64", "Hostarm64"] {
                if discovery.files(
                    &root,
                    &[
                        format!("bin/{host}/{arch}/cl.exe"),
                        format!("bin/{host}/{arch}/link.exe"),
                        "include/vcruntime.h".into(),
                        format!("lib/{arch}/libcmt.lib"),
                    ],
                ) {
                    evidence.push(root.display().to_string());
                    break;
                }
            }
        }
        report.checks.push(Check { name: format!("MSVC {arch} compiler/linker/library files"), status: if !evidence.is_empty() { Status::Found } else if discovery.issues.is_empty() { Status::Missing } else { Status::Unknown }, evidence, guidance: "Checks conventional Visual Studio folders and VCToolsInstallDir only. Custom locations may be missed. Use the installed Developer Command Prompt for MSVC environment activation; a PATH cl.exe alone is not sufficient." });
        let mut evidence = vec![];
        for root in &request.sdk_roots {
            for include in discovery.dirs(&root.join("Include")) {
                let Some(version) = include.file_name().and_then(|v| v.to_str()) else {
                    continue;
                };
                if discovery.files(
                    root,
                    &[
                        format!("Include/{version}/um/Windows.h"),
                        format!("Include/{version}/ucrt/stdio.h"),
                        format!("Lib/{version}/um/{arch}/kernel32.lib"),
                        format!("Lib/{version}/ucrt/{arch}/ucrt.lib"),
                    ],
                ) {
                    evidence.push(format!(
                        "{} — SDK folder {version} ({arch})",
                        root.display()
                    ));
                }
            }
        }
        report.checks.push(Check { name: format!("Windows SDK {arch} header/library files"), status: if !evidence.is_empty() { Status::Found } else if discovery.issues.is_empty() { Status::Missing } else { Status::Unknown }, evidence, guidance: "Checks WindowsSdkDir and the conventional Windows Kits/10 folder. Folder names are discovery hints, not verified versions or a project compatibility guarantee." });
    }
    if request.profile == Profile::RustMsvc {
        let mut evidence = vec![];
        if let Some(root) = &request.rustup_home {
            for toolchain in discovery.dirs(&root.join("toolchains")) {
                if discovery.files(&toolchain, &["bin/rustc.exe".into()])
                    && matches!(
                        probe(
                            &toolchain.join(format!("lib/rustlib/{}/lib", request.target.rust()))
                        ),
                        Probe::Dir
                    )
                {
                    evidence.push(toolchain.display().to_string());
                }
            }
        }
        report.checks.push(Check { name: format!("Installed rustup {} target directory", request.target.rust()), status: if !evidence.is_empty() { Status::Found } else { Status::Unknown }, evidence, guidance: "PATH cargo/rustc can be rustup proxies. Checks conventional rustup files without invoking rustup. Custom Rust installs, active overrides, pinned versions, and target integrity remain unverified." });
    }
    if request.profile == Profile::DotNet {
        let parents: Vec<_> = report
            .tools
            .iter()
            .filter(|t| t.name == "dotnet")
            .flat_map(|t| {
                t.candidates
                    .iter()
                    .filter_map(|p| p.parent().map(Path::to_path_buf))
            })
            .collect();
        let mut evidence = vec![];
        for parent in parents {
            for sdk in discovery.dirs(&parent.join("sdk")) {
                if discovery.files(&sdk, &["MSBuild.dll".into(), "dotnet.dll".into()]) {
                    evidence.push(sdk.display().to_string());
                }
            }
        }
        report.checks.push(Check { name: ".NET SDK files beside observed hosts".into(), status: if !evidence.is_empty() { Status::Found } else if discovery.issues.is_empty() { Status::Missing } else { Status::Unknown }, evidence, guidance: "A .NET runtime/host alone does not establish SDK availability. Checks sdk subfolders beside PATH candidates only; global.json, architecture, and SDK compatibility remain unverified." });
    }
    report.notes.extend(discovery.issues);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_version_ranges_fail_before_inspection_and_valid_ranges_retain_unknowns() {
        let tmp = crate::util::tempdir().unwrap();
        let mut input = request(tmp.path().display().to_string());
        file(&tmp.path().join("node.exe"));
        input.minimum_version = "23".into();
        input.maximum_version = "22".into();
        assert!(inspect(&input, &Progress::default()).is_err());
        input.minimum_version = "22".into();
        input.maximum_version = "23".into();
        let report = inspect(&input, &Progress::default()).unwrap();
        assert_eq!(report.version_command, "node");
        assert_eq!(report.versions.len(), 1);
        assert_eq!(
            report.versions[0].compatibility,
            crate::devversion::Compatibility::Unknown
        );
        assert!(report.versions[0].evidence.is_none());
    }

    fn file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"not an executable; inspection must never run this").unwrap();
    }
    fn request(path: String) -> Request {
        Request {
            profile: Profile::Node,
            path,
            pathext: ".CMD;.EXE".into(),
            ..Default::default()
        }
    }
    fn tool<'a>(report: &'a Report, name: &str) -> &'a Tool {
        report.tools.iter().find(|t| t.name == name).unwrap()
    }

    #[test]
    fn candidate_order_honors_directories_then_extensions_and_collapses_duplicate_dirs() {
        let tmp = crate::util::tempdir().unwrap();
        let a = tmp.path().join("with spaces");
        let b = tmp.path().join("second");
        for path in [a.join("node.exe"), a.join("node.cmd"), b.join("node.cmd")] {
            file(&path);
        }
        let input = request(format!(
            "\"{}\";{};{}",
            a.display(),
            b.display(),
            a.display()
        ));
        let report = inspect(&input, &Progress::default()).unwrap();
        assert_eq!(
            tool(&report, "node").candidates,
            [a.join("node.cmd"), a.join("node.exe"), b.join("node.cmd")]
        );
        assert_eq!(report.entries[2].status, Status::Review);
        assert!(report.entries[2]
            .detail
            .contains("Duplicate of PATH entry 1"));
        assert_eq!(report.checks[0].status, Status::Review);
    }

    #[test]
    fn quoted_semicolon_paths_are_single_entries_and_malformed_quotes_stay_unknown() {
        let tmp = crate::util::tempdir().unwrap();
        let folder = tmp.path().join("tools;with delimiter");
        file(&folder.join("node.exe"));
        let report = inspect(
            &request(format!("\"{}\"", folder.display())),
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(report.entries.len(), 1);
        assert_eq!(tool(&report, "node").candidates, [folder.join("node.exe")]);
        let report = inspect(
            &request(format!("\"{}", folder.display())),
            &Progress::default(),
        )
        .unwrap();
        assert!(report.path_partial);
        assert!(tool(&report, "node").candidates.is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn windows_case_and_slash_variants_are_duplicates_but_drive_relative_paths_are_unknown() {
        let tmp = crate::util::tempdir().unwrap();
        file(&tmp.path().join("node.exe"));
        let folder = tmp.path().display().to_string();
        let variant = folder.to_uppercase().replace('\\', "/");
        let report = inspect(
            &request(format!("{folder};{variant}")),
            &Progress::default(),
        )
        .unwrap();
        assert_eq!(report.entries[1].status, Status::Review);
        assert_eq!(tool(&report, "node").candidates.len(), 1);
        let report = inspect(&request("C:\\;C:".into()), &Progress::default()).unwrap();
        assert_eq!(report.entries[1].status, Status::Unknown);
    }

    #[test]
    fn absent_relative_empty_unresolved_and_network_paths_are_not_false_winners() {
        let tmp = crate::util::tempdir().unwrap();
        file(&tmp.path().join("node.exe"));
        let input = request(format!("relative;;%SETUPSCOPE_NONEXISTENT_DIAGNOSTIC_VAR%;\\\\not-a-server.invalid\\share;{};{}", tmp.path().join("missing").display(), tmp.path().display()));
        let report = inspect(&input, &Progress::default()).unwrap();
        assert!(report.path_partial);
        assert!(report.entries[..4]
            .iter()
            .all(|e| e.status == Status::Unknown));
        assert_eq!(report.entries[4].status, Status::Missing);
        assert_eq!(
            tool(&report, "node").candidates,
            [tmp.path().join("node.exe")]
        );
        assert_eq!(report.checks[0].status, Status::Review);
    }

    #[test]
    fn custom_commands_are_names_only_and_exact_extensions_are_not_reappended() {
        let tmp = crate::util::tempdir().unwrap();
        file(&tmp.path().join("compiler.exe"));
        let mut input = request(tmp.path().display().to_string());
        input.command = "compiler.exe".into();
        let report = inspect(&input, &Progress::default()).unwrap();
        assert_eq!(
            tool(&report, "compiler.exe").candidates,
            [tmp.path().join("compiler.exe")]
        );
        for invalid in [
            "node --version",
            "../node",
            "C:\\node.exe",
            "$(node)",
            "con.exe",
            "LPT1",
            "--help",
        ] {
            input.command = invalid.into();
            assert!(inspect(&input, &Progress::default()).is_err(), "{invalid}");
        }
    }

    #[test]
    fn msvc_and_sdk_readiness_requires_complete_target_specific_file_groups() {
        let tmp = crate::util::tempdir().unwrap();
        let vc = tmp.path().join("VC");
        let sdk = tmp.path().join("SDK");
        for name in [
            "bin/Hostx64/x64/cl.exe",
            "bin/Hostx64/x64/link.exe",
            "include/vcruntime.h",
            "lib/x64/libcmt.lib",
        ] {
            file(&vc.join(name));
        }
        for name in [
            "Include/10.test/um/Windows.h",
            "Include/10.test/ucrt/stdio.h",
            "Lib/10.test/um/x64/kernel32.lib",
            "Lib/10.test/ucrt/x64/ucrt.lib",
        ] {
            file(&sdk.join(name));
        }
        let mut input = Request {
            path: tmp.path().display().to_string(),
            pathext: ".EXE".into(),
            vc_tools: Some(vc),
            sdk_roots: vec![sdk.clone()],
            ..Default::default()
        };
        let report = inspect(&input, &Progress::default()).unwrap();
        assert_eq!(report.checks[2].status, Status::Found);
        assert_eq!(report.checks[3].status, Status::Found);
        fs::remove_file(sdk.join("Lib/10.test/ucrt/x64/ucrt.lib")).unwrap();
        assert_eq!(
            inspect(&input, &Progress::default()).unwrap().checks[3].status,
            Status::Missing
        );
        input.target = Target::Arm64;
        assert_eq!(
            inspect(&input, &Progress::default()).unwrap().checks[2].status,
            Status::Missing
        );
    }

    #[test]
    fn dotnet_host_does_not_imply_sdk_and_scan_does_not_execute_scripts_or_write_files() {
        let tmp = crate::util::tempdir().unwrap();
        file(&tmp.path().join("dotnet.exe"));
        let marker = tmp.path().join("must-not-exist");
        fs::write(
            tmp.path().join("node.cmd"),
            format!("@echo hacked > \"{}\"", marker.display()),
        )
        .unwrap();
        let original = fs::read(tmp.path().join("node.cmd")).unwrap();
        let mut input = request(tmp.path().display().to_string());
        input.profile = Profile::DotNet;
        let report = inspect(&input, &Progress::default()).unwrap();
        assert_eq!(report.checks[0].status, Status::Found);
        assert_eq!(report.checks[1].status, Status::Missing);
        file(&tmp.path().join("sdk/9.test/MSBuild.dll"));
        file(&tmp.path().join("sdk/9.test/dotnet.dll"));
        assert_eq!(
            inspect(&input, &Progress::default()).unwrap().checks[1].status,
            Status::Found
        );
        assert!(!marker.exists());
        assert_eq!(fs::read(tmp.path().join("node.cmd")).unwrap(), original);
    }

    #[test]
    fn cancellation_and_resource_limits_are_explicit() {
        let tmp = crate::util::tempdir().unwrap();
        let input = request(tmp.path().display().to_string());
        let progress = Progress::default();
        progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let report = inspect(&input, &progress).unwrap();
        assert!(report.cancelled);
        assert!(report.checks.is_empty());
        let input = request(
            std::iter::repeat_n(tmp.path().display().to_string(), PATH_LIMIT + 1)
                .collect::<Vec<_>>()
                .join(";"),
        );
        let report = inspect(&input, &Progress::default()).unwrap();
        assert!(report.path_partial);
        assert_eq!(report.entries.len(), PATH_LIMIT);
        assert!(inspect(&request("x".repeat(128 * 1024 + 1)), &Progress::default()).is_err());
        let mut input = request(tmp.path().display().to_string());
        input.pathext = ";invalid".into();
        assert!(inspect(&input, &Progress::default()).unwrap().path_partial);
    }

    #[test]
    fn candidate_limit_marks_partial_instead_of_claiming_complete_resolution() {
        let tmp = crate::util::tempdir().unwrap();
        let mut paths = vec![];
        for n in 0..=CANDIDATE_LIMIT {
            let dir = tmp.path().join(n.to_string());
            file(&dir.join("node.exe"));
            paths.push(dir.display().to_string());
        }
        let report = inspect(&request(paths.join(";")), &Progress::default()).unwrap();
        assert_eq!(tool(&report, "node").candidates.len(), CANDIDATE_LIMIT);
        assert!(!tool(&report, "node").notes.is_empty());
    }

    #[test]
    fn discovery_is_bounded_and_errors_are_visible() {
        let tmp = crate::util::tempdir().unwrap();
        for i in 0..65 {
            fs::create_dir(tmp.path().join(i.to_string())).unwrap();
        }
        let progress = Progress::default();
        let mut discovery = Discovery {
            remaining: 1,
            progress: &progress,
            issues: vec![],
        };
        assert!(discovery.dirs(tmp.path()).len() <= 64);
        assert!(discovery.dirs(tmp.path()).is_empty());
        assert!(!discovery.issues.is_empty());
    }
}
