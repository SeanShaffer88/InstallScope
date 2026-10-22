//! Read-only, bounded project declarations and local toolchain evidence.
mod constraints;
mod dotnet;
mod manifests;
mod desktop;
use crate::{devcheck, snapshot::Progress};
use anyhow::{bail, Context, Result};
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Component, Path, PathBuf},
};

const MAX_FILE: usize = 256 * 1024;
const MAX_TOTAL: usize = 2 * 1024 * 1024;
const MAX_ANCESTORS: usize = 32;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Candidate,
    Different,
    Missing,
    Unknown,
    Review,
}
impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Candidate => "Candidate metadata matches",
            Self::Different => "Metadata differs",
            Self::Missing => "Not found in checked locations",
            Self::Unknown => "Unknown / unsupported",
            Self::Review => "Review declaration",
        }
    }
}
#[derive(Clone, Debug)]
pub struct Finding {
    pub source: PathBuf,
    pub tool: String,
    pub declared: String,
    pub state: State,
    pub detail: String,
    pub evidence: Vec<String>,
    sdk: Option<dotnet::Policy>,
}
impl Finding {
    fn new(source: &Path, tool: &str, declared: &str, detail: impl Into<String>) -> Self {
        Self {
            source: source.into(),
            tool: tool.into(),
            declared: declared.into(),
            state: State::Review,
            detail: detail.into(),
            evidence: vec![],
            sdk: None,
        }
    }
}
pub struct Request {
    pub root: PathBuf,
    pub tools: devcheck::Request,
    pub dotnet_roots: Vec<PathBuf>,
}
impl Request {
    pub fn current(root: PathBuf, target: devcheck::Target) -> Result<Self> {
        let mut dotnet_roots: Vec<_> = [
            "DOTNET_ROOT",
            "DOTNET_ROOT_X64",
            "DOTNET_ROOT_X86",
            "DOTNET_ROOT_ARM64",
        ]
        .iter()
        .filter_map(std::env::var_os)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect();
        for key in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(p) = std::env::var_os(key) {
                dotnet_roots.push(PathBuf::from(p).join("dotnet"));
            }
        }
        Ok(Self {
            root,
            tools: devcheck::Request::current(devcheck::Profile::RustMsvc, target, String::new())?,
            dotnet_roots,
        })
    }
}
#[derive(Clone, Debug)]
pub struct Report {
    pub root: PathBuf,
    pub observed_at: String,
    pub target: devcheck::Target,
    pub findings: Vec<Finding>,
    pub notes: Vec<String>,
    pub files: Vec<PathBuf>,
    pub tooling: devcheck::Report,
    pub partial: bool,
    pub cancelled: bool,
}
impl Report {
    pub fn text(&self) -> String {
        let mut text=format!("SetupScope Project Doctor\nRoot: {}\nTarget: {}\nPartial: {}; cancelled: {}\nRead-only metadata evidence; no scripts/tools executed. A candidate match does not establish build readiness or actual resolver selection.\n",self.root.display(),self.target.label(),self.partial,self.cancelled);
        text.push_str(&format!("Observed: {} (local time)\n", self.observed_at));
        for row in &self.findings {
            text.push_str(&format!(
                "\n{} | {} | {}\n{}\n{}\n",
                row.tool,
                row.state.label(),
                row.declared,
                row.source.display(),
                row.detail
            ));
            for line in &row.evidence {
                text.push_str(&format!("  {line}\n"));
            }
        }
        for note in &self.notes {
            text.push_str(&format!("\nNote: {note}\n"));
        }
        text.push_str(&format!("\nPrerequisite profile: {} (mixed projects may need other profiles in Developer diagnostics)\n",self.tooling.profile.label()));
        for check in &self.tooling.checks {
            text.push_str(&format!("{}: {}\n", check.name, check.status.label()));
            text.push_str(&format!("  {}\n", check.guidance));
            for evidence in &check.evidence {
                text.push_str(&format!("  {evidence}\n"));
            }
        }
        for note in &self.tooling.notes {
            text.push_str(&format!("\nPATH note: {note}\n"));
        }
        for tool in &self.tooling.tools {
            if !tool.candidates.is_empty() {
                text.push_str(&format!(
                    "\nPATH candidates for {} (observed order):\n",
                    tool.name
                ));
                for path in &tool.candidates {
                    text.push_str(&format!("  {}\n", path.display()));
                }
                for note in &tool.notes {
                    text.push_str(&format!("  {note}\n"));
                }
            }
        }
        text
    }
}
fn guard(path: &Path) -> Result<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        bail!("Choose an absolute path without dot components");
    }
    for part in path.ancestors() {
        match fs::symlink_metadata(part) {
            Ok(m) => {
                let mut linked = m.file_type().is_symlink();
                #[cfg(windows)]
                {
                    use std::os::windows::fs::MetadataExt;
                    linked |= m.file_attributes() & 0x400 != 0;
                }
                if linked {
                    bail!("Linked/reparse paths are not inspected: {}", part.display());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
struct Reader<'a> {
    progress: &'a Progress,
    bytes: usize,
    files: Vec<PathBuf>,
    partial: bool,
    notes: Vec<String>,
}
impl Reader<'_> {
    fn read(&mut self, path: &Path) -> Result<Option<String>> {
        if self.progress.cancelled() {
            bail!("Inspection cancelled");
        }
        guard(path)?;
        let meta = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        if !meta.is_file() || meta.len() > MAX_FILE as u64 {
            bail!("Manifest is not a regular file or exceeds 256 KiB");
        }
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take((MAX_FILE + 1) as u64)
            .read_to_end(&mut bytes)?;
        if bytes.len() > MAX_FILE || self.bytes + bytes.len() > MAX_TOTAL {
            bail!("Project manifest budget exceeded (256 KiB/file, 2 MiB total)");
        }
        self.bytes += bytes.len();
        self.files.push(path.into());
        self.progress.tick();
        let text = String::from_utf8(bytes).context("Manifest must be UTF-8")?;
        Ok(Some(text.trim_start_matches('\u{feff}').to_owned()))
    }
    fn nearest(&mut self, root: &Path, names: &[&str]) -> Option<(PathBuf, String)> {
        for (depth, parent) in root.ancestors().enumerate() {
            if self.progress.cancelled() {
                return None;
            }
            if depth == MAX_ANCESTORS {
                self.partial = true;
                self.notes.push("Ancestor search stopped at 32 folders; effective declarations may be outside that bound.".into());
                return None;
            }
            for name in names {
                let path = parent.join(name);
                match self.read(&path) {
                    Ok(Some(text)) => return Some((path, text)),
                    Ok(None) => {}
                    Err(e) => {
                        self.partial = true;
                        self.notes.push(format!("{}: {e:#}. Search stopped here; an ancestor declaration cannot safely substitute for this file.",path.display()));
                        return None;
                    }
                }
            }
        }
        None
    }
}
fn field(text: &str) -> Result<&str> {
    if text.is_empty() || text.len() > 256 || text.chars().any(char::is_control) {
        bail!("Declaration must be 1–256 characters without control characters");
    }
    Ok(text)
}
fn arch(path: &Path) -> Result<&'static str> {
    guard(path)?;
    if !fs::symlink_metadata(path)?.is_file() {
        bail!("PE inspection requires a regular file");
    }
    let mut file = fs::File::open(path)?;
    let mut dos = [0u8; 64];
    file.read_exact(&mut dos)?;
    if &dos[..2] != b"MZ" {
        bail!("Not a Windows PE executable");
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap());
    if !(64..=1024 * 1024).contains(&offset) {
        bail!("PE header outside the inspection bound");
    }
    file.seek(SeekFrom::Start(offset as u64))?;
    let mut pe = [0u8; 6];
    file.read_exact(&mut pe)?;
    if &pe[..4] != b"PE\0\0" {
        bail!("PE signature unavailable");
    }
    Ok(match u16::from_le_bytes([pe[4], pe[5]]) {
        0x8664 => "x64",
        0xaa64 => "arm64",
        0x14c => "x86",
        _ => "other/unknown",
    })
}
fn regular_checked(path: &Path) -> Result<bool> {
    guard(path)?;
    match fs::symlink_metadata(path) {
        Ok(meta) => Ok(meta.is_file()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}
fn regular(path: &Path) -> bool {
    regular_checked(path).unwrap_or(false)
}

pub fn inspect(request: &Request, progress: &Progress) -> Result<Report> {
    guard(&request.root)?;
    if !fs::metadata(&request.root)?.is_dir() {
        bail!("Choose an existing project folder");
    }
    progress.set_phase("Reading project declarations");
    let mut reader = Reader {
        progress,
        bytes: 0,
        files: vec![],
        partial: false,
        notes: vec![],
    };
    let mut findings = manifests::collect(&request.root, &mut reader);
    findings.extend(desktop::collect(&request.root, &mut reader));
    progress.set_phase("Inspecting local tool metadata");
    let mut tool_request = request.tools.clone();
    tool_request.profile = if findings.iter().any(|r| r.tool.starts_with("Rust")) {
        devcheck::Profile::RustMsvc
    } else if findings.iter().any(|r| r.tool == ".NET SDK") {
        devcheck::Profile::DotNet
    } else if findings.iter().any(|r| r.tool.starts_with("CMake") || r.tool=="MSVC toolset") {
        devcheck::Profile::Cmake
    } else if findings.iter().any(|r| r.tool.starts_with(".NET")) {
        devcheck::Profile::DotNet
    } else if findings.iter().any(|r| r.tool.starts_with("Node")) {
        devcheck::Profile::Node
    } else if findings.iter().any(|r| r.tool.starts_with("Python")) {
        devcheck::Profile::Python
    } else {
        request.tools.profile
    };
    let tooling = devcheck::inspect(&tool_request, progress)?;
    for row in &mut findings {
        if progress.cancelled() {
            break;
        }
        if row.state == State::Unknown {
            continue;
        }
        if row.tool == ".NET SDK" {
            dotnet::assess(row, request, &tooling, progress, &mut reader);
        } else if row.tool == "Rust toolchain" {
            assess_rust(row, request, &mut reader);
        } else {
            assess_tool(row, &tooling, request.tools.target, progress);
        }
    }
    let cancelled = progress.cancelled() || tooling.cancelled;
    let partial = reader.partial
        || tooling.path_partial
        || cancelled
        || findings
            .iter()
            .any(|r| matches!(r.state, State::Unknown | State::Review));
    reader.notes.push("Only named files in the selected folder and nearest .NET/Rust declarations in up to 32 ancestors are read. No recursive project scan, scripts, builds, repairs, installs or environment changes. Monorepo/member/workspace inheritance and resolver overrides can change actual selection.".into());
    reader.notes.push("Installed files and directory names are evidence, not proof of authentic or usable SDK contents. PATH order reflects this process only; shells, IDEs, version managers, environment overrides and custom paths may select differently. Prerequisite diagnostics remain available separately.".into());
    if findings.is_empty() {
        reader.notes.push("No supported requirement declaration was found. This does not establish that the project has no prerequisites.".into());
    }
    Ok(Report {
        root: request.root.clone(),
        observed_at: crate::util::now_string(),
        target: request.tools.target,
        findings,
        notes: reader.notes,
        files: reader.files,
        tooling,
        partial,
        cancelled,
    })
}
fn assess_tool(
    row: &mut Finding,
    tooling: &devcheck::Report,
    target: devcheck::Target,
    progress: &Progress,
) {
    let command = match row.tool.as_str() {
        "Node.js" | "Node.js pin" => "node",
        "Python" | "Python pin" => "python",
        "Rust minimum" => "rustc",
        _ => return,
    };
    if let Err(e) = constraints::matches(&row.tool, &row.declared, &semver::Version::new(0, 0, 0)) {
        row.state = State::Unknown;
        row.detail
            .push_str(&format!(" Unsupported comparison: {e:#}"));
        return;
    }
    let Some(tool) = tooling.tools.iter().find(|t| t.name == command) else {
        row.state = State::Unknown;
        return;
    };
    if tool.candidates.is_empty() {
        row.state = if tooling.path_partial {
            State::Unknown
        } else {
            State::Missing
        };
        row.detail.push_str(" No candidate appeared in the checked PATH locations; custom locations and version managers may supply one.");
        return;
    }
    row.state = State::Unknown;
    let mut matched = false;
    let mut different = false;
    let mut unknown = tool.candidates.len() > 8;
    for path in tool.candidates.iter().take(8) {
        if progress.cancelled() {
            break;
        }
        let result = (|| -> Result<(bool, String)> {
            guard(path)?;
            if !regular(path) {
                bail!("Not an ordinary file");
            }
            let architecture = arch(path)?;
            let evidence = crate::devversion::read(path)?;
            let (trust, why) = crate::devversion::assess(
                command,
                &evidence,
                &crate::devversion::Range {
                    minimum: None,
                    maximum_exclusive: None,
                },
            );
            if trust == crate::devversion::Compatibility::Unknown {
                bail!("{why}");
            }
            if evidence.prerelease || evidence.version.0[3] != 0 {
                bail!("Prerelease or four-part product version cannot establish the declared runtime version");
            }
            let [major, minor, patch, _] = evidence.version.0;
            let version = semver::Version::new(major.into(), minor.into(), patch.into());
            let agrees = constraints::matches(&row.tool, &row.declared, &version)?;
            Ok((agrees,format!("{}: product metadata {}; PE {}; {} target. Actual runtime/version-manager selection remains unverified.",path.display(),version,architecture,target.label())))
        })();
        match result {
            Ok((yes, detail)) => {
                matched |= yes;
                different |= !yes;
                row.evidence.push(detail);
            }
            Err(e) => {
                unknown = true;
                row.evidence
                    .push(format!("{}: unknown ({e:#})", path.display()));
            }
        }
    }
    row.state = if matched && !different && !unknown {
        State::Candidate
    } else if different && !matched && !unknown {
        State::Different
    } else {
        State::Unknown
    };
    if tool.candidates.len() > 8 {
        row.evidence
            .push("Version reads stopped at 8 candidates; others remain unverified.".into());
    }
}
fn assess_rust(row: &mut Finding, request: &Request, reader: &mut Reader<'_>) {
    row.state = State::Unknown;
    let Some(home) = &request.tools.rustup_home else {
        row.detail.push_str(" Rustup home was not available.");
        return;
    };
    let root = home.join("toolchains");
    if let Err(e) = guard(&root) {
        row.evidence
            .push(format!("Rustup store unavailable: {e:#}"));
        return;
    }
    let entries = match fs::read_dir(&root) {
        Ok(e) => e,
        Err(e) => {
            row.evidence.push(format!("{}: {e}", root.display()));
            return;
        }
    };
    let mut found = false;
    let mut complete = true;
    for (index, entry) in entries.enumerate() {
        if reader.progress.cancelled() {
            return;
        }
        if index == 256 {
            reader.partial = true;
            row.evidence
                .push("Rustup inventory stopped at 256 entries.".into());
            break;
        }
        let Ok(entry) = entry else {
            reader.partial = true;
            complete = false;
            continue;
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == row.declared
            || name
                .strip_prefix(&row.declared)
                .is_some_and(|s| s.starts_with('-'))
        {
            match regular_checked(&entry.path().join("bin/rustc.exe")) {
                Ok(true) => {
                    found = true;
                    row.evidence.push(format!("Toolchain-name candidate: {}. Components, target libraries, aliases and effective rustup override are not verified.",entry.path().display()));
                }
                Ok(false) => {}
                Err(e) => {
                    complete = false;
                    reader.partial = true;
                    if row.evidence.len() < 8 {
                        row.evidence
                            .push(format!("{}: unknown ({e:#})", entry.path().display()));
                    }
                }
            }
            if row.evidence.len() >= 8 {
                reader.partial = true;
                complete = false;
                break;
            }
        }
    }

    row.state = if found {
        State::Candidate
    } else if complete {
        State::Missing
    } else {
        State::Unknown
    };
    row.detail.push_str(" File declaration only: command-line/environment/directory overrides can take precedence. No rustup command or download ran.");
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(root: &Path) -> Request {
        Request {
            root: root.into(),
            tools: devcheck::Request {
                path: root.join("bin").display().to_string(),
                pathext: ".EXE".into(),
                ..Default::default()
            },
            dotnet_roots: vec![],
        }
    }
    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fn pe(path: &Path, machine: u16) {
        let mut bytes = vec![0; 134];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[60..64].copy_from_slice(&128u32.to_le_bytes());
        bytes[128..132].copy_from_slice(b"PE\0\0");
        bytes[132..].copy_from_slice(&machine.to_le_bytes());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
    }
    #[test]
    fn sdk_inventory_selects_candidates_without_touching_project_files() {
        let td = crate::util::tempdir().unwrap();
        let root = td.path().join("project");
        write(
            &root.join("global.json"),
            r#"{"sdk":{"version":"8.0.100","rollForward":"latestPatch","allowPrerelease":false}}"#,
        );
        let sdk = td.path().join("dotnet");
        pe(&sdk.join("dotnet.exe"), 0x8664);
        write(&sdk.join("sdk/8.0.105/dotnet.dll"), "fixture");
        write(&sdk.join("sdk/9.0.100/dotnet.dll"), "fixture");
        let before = fs::read(root.join("global.json")).unwrap();
        let mut request = fixture(&root);
        request.dotnet_roots.push(sdk);
        let report = inspect(&request, &Progress::default()).unwrap();
        let row = report
            .findings
            .iter()
            .find(|r| r.tool == ".NET SDK")
            .unwrap();
        assert_eq!(row.state, State::Candidate);
        assert!(row.evidence.iter().any(|s| s.contains("8.0.105")));
        assert_eq!(fs::read(root.join("global.json")).unwrap(), before);
        assert_eq!(fs::read_dir(root).unwrap().count(), 1);
        assert!(report.text().contains("not proof"));
    }
    #[test]
    fn unreadable_or_excess_manifest_blocks_ancestor_substitution() {
        let td = crate::util::tempdir().unwrap();
        let root = td.path().join("nested");
        fs::create_dir_all(&root).unwrap();
        write(
            &td.path().join("global.json"),
            r#"{"sdk":{"version":"8.0.100"}}"#,
        );
        write(&root.join("global.json"), &" ".repeat(MAX_FILE + 1));
        let report = inspect(&fixture(&root), &Progress::default()).unwrap();
        assert!(report.partial);
        assert!(!report.files.contains(&td.path().join("global.json")));
        assert!(report
            .notes
            .iter()
            .any(|n| n.contains("Search stopped here")));
    }
    #[test]
    fn nearest_legacy_rust_file_wins_and_store_is_inspected_only_for_presence() {
        let td = crate::util::tempdir().unwrap();
        let root = td.path().join("project");
        write(&root.join("rust-toolchain"), "1.91.0\n");
        write(
            &root.join("rust-toolchain.toml"),
            "[toolchain]\nchannel='nightly'",
        );
        let store = td.path().join("rustup");
        write(
            &store.join("toolchains/1.91.0-x86_64-pc-windows-msvc/bin/rustc.exe"),
            "fixture",
        );
        let mut req = fixture(&root);
        req.tools.rustup_home = Some(store);
        let report = inspect(&req, &Progress::default()).unwrap();
        assert_eq!(report.findings[0].declared, "1.91.0");
        assert_eq!(report.findings[0].state, State::Candidate);
        assert!(!report.files.contains(&root.join("rust-toolchain.toml")));
    }
    #[test]
    fn malformed_requirement_and_pre_cancelled_scans_remain_unknown() {
        let td = crate::util::tempdir().unwrap();
        write(
            &td.path().join("package.json"),
            r#"{"engines":{"node":"* || unsupported"},"scripts":{"postinstall":"do not run"}}"#,
        );
        let report = inspect(&fixture(td.path()), &Progress::default()).unwrap();
        assert_eq!(report.findings[0].state, State::Unknown);
        let progress = Progress::default();
        progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let report = inspect(&fixture(td.path()), &progress).unwrap();
        assert!(report.cancelled);
        assert!(report.files.is_empty());
    }
    #[test]
    fn pe_architecture_is_bounded_and_never_executes() {
        let td = crate::util::tempdir().unwrap();
        let path = td.path().join("tool.exe");
        pe(&path, 0xaa64);
        assert_eq!(arch(&path).unwrap(), "arm64");
        fs::write(&path, b"not an executable").unwrap();
        assert!(arch(&path).is_err());
    }
    #[test]
    fn sdk_custom_paths_are_not_followed() {
        let td = crate::util::tempdir().unwrap();
        write(
            &td.path().join("global.json"),
            r#"{"sdk":{"version":"8.0.100","paths":["C:/private"]}}"#,
        );
        let report = inspect(&fixture(td.path()), &Progress::default()).unwrap();
        assert_eq!(report.findings[0].state, State::Unknown);
        assert!(report.findings[0].detail.contains("not followed"));
    }
}
