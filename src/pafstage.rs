//! Reviewed source-file staging into a new package directory. No source mutation,
//! tool execution, archive extraction, elevation or automatic cleanup is performed.
//! Trusted, privately controlled workspace/source trees are required: path checks
//! and hashes detect ordinary changes; this is not a hostile-filesystem sandbox.
use crate::{
    pafassets::{self, IconAssets},
    pafproject::Project,
};
use anyhow::{anyhow, Context, Result};
use std::collections::BTreeSet;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MAX_ENTRIES: usize = 100_000;
const MAX_BYTES: u64 = 64 * 1024 * 1024 * 1024;
const MAX_PATH_BYTES: usize = 32 * 1024 * 1024;
#[derive(Clone, Debug)]
struct Entry {
    source: PathBuf,
    destination: PathBuf,
    bytes: u64,
    hash: Option<[u8; 32]>,
}
#[derive(Clone, Debug)]
pub struct StagePlan {
    project: Project,
    source: PathBuf,
    workspace: PathBuf,
    entries: Vec<Entry>,
    file_count: usize,
    drafts: Vec<(String, String)>,
    output: PathBuf,
    pub icon: IconAssets,
    pub total_bytes: u64,
}
#[derive(Clone, Debug)]
pub struct Prepared {
    pub package: PathBuf,
    pub record: PathBuf,
    pub report: PathBuf,
    pub files: usize,
    pub bytes: u64,
}
/// Reconnect only the original companion snapshot of a completed preparation.
/// Rechecks copied application bytes; this is not build or portability validation.
pub fn reopen(
    record: &Path,
    project: &Project,
    cancel: &dyn Fn() -> bool,
) -> Result<Option<Prepared>> {
    if record.file_name().and_then(|n| n.to_str()) != Some("project.json") {
        return Ok(None);
    }
    let root = record.parent().context("Project has no parent folder")?;
    let marker = root.join("preparation-complete.txt");
    if !marker.try_exists()? {
        return Ok(None);
    }
    cancelled(cancel)?;
    plain(root)?;
    check_chain(root, record)?;
    let read = |path: &Path, limit: u64| -> Result<Vec<u8>> {
        check_chain(root, path)?;
        if !plain(path)?.is_file() {
            return Err(anyhow!("Expected a regular file: {}", path.display()));
        }
        let mut bytes = Vec::new();
        File::open(path)?.take(limit + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > limit {
            return Err(anyhow!("Preparation companion exceeds its size limit"));
        }
        Ok(bytes)
    };
    let expected = b"Preparation completed. This is not a build, portability test, or official approval. Original sources were not modified.\r\n";
    if read(&marker, 1024)? != expected {
        return Err(anyhow!("Unrecognized preparation completion marker"));
    }
    // drafts() validates the identifier before it can become a filesystem path.
    let drafts = project.drafts()?;
    let package = root.join(&project.app_id);
    check_chain(root, &package)?;
    for (name, text) in drafts {
        cancelled(cancel)?;
        let mut expected = vec![0xff, 0xfe];
        for word in text.encode_utf16() {
            expected.extend_from_slice(&word.to_le_bytes());
        }
        if read(&package.join(name), 128 * 1024)? != expected {
            return Err(anyhow!("Prepared configuration differs from the saved answers; review the package before building"));
        }
    }
    let report = root.join("preparation-report.json");
    let value = crate::json::parse_bytes(&read(&report, 64 * 1024 * 1024)?)?;
    if value["schema"].as_u64() != Some(1) {
        return Err(anyhow!("Unsupported preparation report schema"));
    }
    let entries = value["source_files"]
        .as_array()
        .context("Missing prepared file list")?;
    if entries.len() > MAX_ENTRIES {
        return Err(anyhow!("Too many prepared files"));
    }
    let mut bytes = 0u64;
    let mut seen = BTreeSet::new();
    for entry in entries {
        cancelled(cancel)?;
        let relative = entry["package_relative"]
            .as_str()
            .context("Missing package path")?;
        let validated = portable_builder_kit::relative_path(relative)?;
        let path = validated.as_path();
        if path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
            || !seen.insert(relative.to_uppercase())
        {
            return Err(anyhow!("Unsafe or duplicate prepared file path"));
        }
        let destination = package.join(path);
        check_chain(root, &destination)?;
        let size = entry["bytes"]
            .as_u64()
            .context("Missing prepared file size")?;
        bytes = bytes
            .checked_add(size)
            .filter(|n| *n <= MAX_BYTES)
            .context("Prepared contents exceed size limit")?;
        if !plain(&destination)?.is_file() || plain(&destination)?.len() != size {
            return Err(anyhow!("Prepared file is missing or changed: {relative}"));
        }
        let digest: String = hash(&destination, size, cancel)?
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        if entry["sha256"].as_str() != Some(digest.as_str()) {
            return Err(anyhow!("Prepared file changed: {relative}"));
        }
    }
    Ok(Some(Prepared {
        package,
        record: record.to_path_buf(),
        report,
        files: entries.len(),
        bytes,
    }))
}
fn cancelled(cancel: &dyn Fn() -> bool) -> Result<()> {
    if cancel() {
        Err(anyhow!("Preparation cancelled"))
    } else {
        Ok(())
    }
}
fn plain(path: &Path) -> Result<Metadata> {
    let m =
        fs::symlink_metadata(path).with_context(|| format!("Cannot inspect {}", path.display()))?;
    #[cfg(windows)]
    let reparse = {
        use std::os::windows::fs::MetadataExt;
        m.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let reparse = false;
    if m.file_type().is_symlink() || reparse || !(m.is_dir() || m.is_file()) {
        return Err(anyhow!(
            "Links, reparse points and special files are not staged: {}",
            path.display()
        ));
    }
    Ok(m)
}
struct Cancellable<'a> {
    file: File,
    cancel: &'a dyn Fn() -> bool,
}
impl Read for Cancellable<'_> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if (self.cancel)() {
            return Err(std::io::Error::other("Preparation cancelled"));
        }
        self.file.read(bytes)
    }
}
fn hash(path: &Path, limit: u64, cancel: &dyn Fn() -> bool) -> Result<[u8; 32]> {
    Ok(crate::util::sha256_reader(
        Cancellable {
            file: File::open(path)?,
            cancel,
        }
        .take(limit.saturating_add(1)),
    )?)
}
fn check_chain(root: &Path, path: &Path) -> Result<()> {
    let relative = path
        .strip_prefix(root)
        .context("File escaped selected source")?;
    plain(root)?;
    let mut current = root.to_path_buf();
    for part in relative.components() {
        current.push(part);
        plain(&current)?;
    }
    Ok(())
}
impl StagePlan {
    pub fn output(&self) -> &Path {
        &self.output
    }
    pub fn project(&self) -> &Project {
        &self.project
    }
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
    pub fn mapping(&self, index: usize) -> Option<(&Path, &Path, u64)> {
        self.entries
            .get(index)
            .map(|e| (e.source.as_path(), e.destination.as_path(), e.bytes))
    }
    pub fn mappings(&self) -> impl Iterator<Item = (&Path, &Path, u64)> {
        self.entries
            .iter()
            .map(|e| (e.source.as_path(), e.destination.as_path(), e.bytes))
    }
    pub fn file_count(&self) -> usize {
        self.file_count
    }
    pub fn create(
        project: &Project,
        cancel: &dyn Fn() -> bool,
        report: &dyn Fn(&str),
    ) -> Result<Self> {
        cancelled(cancel)?;
        let findings = project.findings();
        if let Some(f) = findings.iter().find(|f| f.blocking && f.step != 6) {
            return Err(anyhow!("{}: {}", f.field, f.message));
        }
        let drafts = project.drafts()?;
        let source = PathBuf::from(&project.source);
        let workspace = PathBuf::from(&project.workspace);
        if !plain(&source)?.is_dir() || !plain(&workspace)?.is_dir() {
            return Err(anyhow!("Choose source and workspace folders"));
        }
        let source = fs::canonicalize(source)?;
        let workspace = fs::canonicalize(workspace)?;
        if source.starts_with(&workspace) || workspace.starts_with(&source) {
            return Err(anyhow!(
                "Keep source and workspace separate, with neither inside the other"
            ));
        }
        let program = portable_builder_kit::relative_path(&project.program)?;
        let mut parts = program.components();
        let folder = parts
            .next()
            .ok_or_else(|| anyhow!("Choose the application subfolder"))?;
        let relative = parts.as_path();
        if relative.as_os_str().is_empty() {
            return Err(anyhow!("Use an application subfolder in Program executable, for example Example\\example.exe. The source folder's contents will be copied beneath App\\Example."));
        }
        let folder = folder
            .as_os_str()
            .to_str()
            .ok_or_else(|| anyhow!("Application folder must be Unicode"))?;
        if ["APPINFO", "DEFAULTDATA"].contains(&folder.to_uppercase().as_str()) {
            return Err(anyhow!("Choose an application subfolder other than AppInfo or DefaultData; those are reserved by this wizard's layout."));
        }
        let executable = source.join(relative);
        check_chain(&source, &executable)?;
        portable_builder_kit::validate_executable(&executable)?;
        report("Reading and converting the app icon");
        let icon = pafassets::import(Path::new(&project.icon), cancel)?;
        let base = Path::new("App").join(folder);
        let mut entries = Vec::new();
        let mut queue = vec![(source.clone(), base)];
        let mut names = BTreeSet::new();
        let mut total_bytes = 0u64;
        let mut path_bytes = 0usize;
        while let Some((from, to)) = queue.pop() {
            cancelled(cancel)?;
            check_chain(&source, &from)?;
            let mut children = Vec::new();
            for item in fs::read_dir(&from)? {
                cancelled(cancel)?;
                let item = item?;
                if entries.len() + children.len() >= MAX_ENTRIES {
                    return Err(anyhow!(
                        "Source exceeds the wizard limit of 100,000 entries"
                    ));
                }
                children.push(item);
            }
            children.sort_by_key(|item| item.file_name());
            for item in children {
                cancelled(cancel)?;
                let name = item.file_name();
                let name = name
                    .to_str()
                    .ok_or_else(|| anyhow!("A source filename cannot be represented as Unicode"))?;
                portable_builder_kit::validate_component(name).with_context(|| {
                    format!(
                        "Source filename is not Windows-safe: {}",
                        item.path().display()
                    )
                })?;
                let destination = to.join(name);
                path_bytes = path_bytes
                    .checked_add(item.path().as_os_str().len())
                    .and_then(|n| n.checked_add(destination.as_os_str().len()))
                    .filter(|&n| n <= MAX_PATH_BYTES)
                    .ok_or_else(|| {
                        anyhow!("Source paths exceed the wizard's 32 MiB metadata budget")
                    })?;
                let key = destination
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_uppercase();
                if !names.insert(key) {
                    return Err(anyhow!(
                        "Source has names that collide under case-insensitive staging: {}",
                        item.path().display()
                    ));
                }
                let m = plain(&item.path())?;
                if m.is_dir() {
                    entries.push(Entry {
                        source: item.path(),
                        destination: destination.clone(),
                        bytes: 0,
                        hash: None,
                    });
                    queue.push((item.path(), destination));
                } else {
                    total_bytes = total_bytes
                        .checked_add(m.len())
                        .filter(|&n| n <= MAX_BYTES)
                        .ok_or_else(|| anyhow!("Source exceeds the wizard limit of 64 GiB"))?;
                    report(&format!("Reviewing {}", item.path().display()));
                    let digest = hash(&item.path(), m.len(), cancel)?;
                    if plain(&item.path())?.len() != m.len() {
                        return Err(anyhow!(
                            "Source changed while being reviewed: {}",
                            item.path().display()
                        ));
                    }
                    entries.push(Entry {
                        source: item.path(),
                        destination,
                        bytes: m.len(),
                        hash: Some(digest),
                    });
                }
            }
        }
        if entries.is_empty() {
            return Err(anyhow!("Source folder contains no application files"));
        }
        let output = workspace.join(format!(
            "{}-{}-{}",
            project.app_id,
            std::process::id(),
            crate::util::new_id()
        ));
        if output.try_exists()? {
            return Err(anyhow!(
                "Output folder already exists; create another review plan"
            ));
        }
        let file_count = entries.iter().filter(|e| e.hash.is_some()).count();
        Ok(Self {
            file_count,
            output,
            project: project.clone(),
            source,
            workspace,
            entries,
            drafts,
            icon,
            total_bytes,
        })
    }

    /// Must be called only after displaying mappings and explicit user action.
    /// New output is retained on failure for inspection; no existing tree is deleted.
    pub fn prepare(&self, cancel: &dyn Fn() -> bool, report: &dyn Fn(&str)) -> Result<Prepared> {
        cancelled(cancel)?;
        plain(&self.workspace)?;
        plain(&self.source)?;
        if fs::canonicalize(&self.workspace)? != self.workspace
            || fs::canonicalize(&self.source)? != self.source
        {
            return Err(anyhow!(
                "Selected folders changed; create a new review plan"
            ));
        }
        let wrapper = self.output.clone();
        fs::create_dir(&wrapper)
            .context("Could not create a new output folder; nothing was overwritten")?;
        let package = wrapper.join(&self.project.app_id);
        let record = wrapper.join("project.json");
        let work = || -> Result<Prepared> {
            self.project.save_new(&record)?;
            fs::create_dir(&package)?;
            for dir in [
                "App/AppInfo/Launcher",
                "App/DefaultData",
                "Data",
                "Other/Source",
            ] {
                fs::create_dir_all(package.join(dir))?;
            }
            for entry in &self.entries {
                cancelled(cancel)?;
                check_chain(&self.source, &entry.source)?;
                let destination = package.join(&entry.destination);
                if let Some(expected) = entry.hash {
                    report(&format!("Copying {}", entry.destination.display()));
                    fs::create_dir_all(destination.parent().unwrap())?;
                    let mut from = File::open(&entry.source)?;
                    let mut to = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&destination)?;
                    let mut buffer = [0u8; 65536];
                    let mut copied = 0u64;
                    loop {
                        cancelled(cancel)?;
                        let count = from.read(&mut buffer)?;
                        if count == 0 {
                            break;
                        }
                        copied += count as u64;
                        if copied > entry.bytes {
                            return Err(anyhow!(
                                "Source grew after review: {}",
                                entry.source.display()
                            ));
                        }
                        to.write_all(&buffer[..count])?;
                    }
                    to.sync_all()?;
                    drop(to);
                    if copied != entry.bytes || hash(&destination, entry.bytes, cancel)? != expected
                    {
                        return Err(anyhow!("Source contents changed after review: {}. Review again before retrying.",entry.source.display()));
                    }
                } else {
                    fs::create_dir_all(destination)?;
                }
            }
            for (name, text) in &self.drafts {
                cancelled(cancel)?;
                let mut bytes = vec![255, 254];
                for word in text.encode_utf16() {
                    bytes.extend_from_slice(&word.to_le_bytes());
                }
                write_new(&package.join(name), &bytes)?;
            }
            for (name, bytes) in &self.icon.files {
                cancelled(cancel)?;
                write_new(&package.join("App/AppInfo").join(name), bytes)?;
            }
            let escape = |s: &str| {
                s.replace('&', "&amp;")
                    .replace('<', "&lt;")
                    .replace('>', "&gt;")
                    .replace('"', "&quot;")
            };
            write_new(&package.join("help.html"),format!("<!doctype html><html lang=\"en\"><meta charset=\"utf-8\"><title>{0}</title><h1>{0}</h1><p>{1}</p><p>Publisher: {2}</p><p>Homepage: {3}</p><p>This is a prepared package. Build with the official PortableApps.com tools and complete portability testing before distribution.</p></html>",escape(&self.project.name),escape(&self.project.description),escape(&self.project.publisher),escape(&self.project.homepage)).as_bytes())?;
            let source_files = self
                .entries
                .iter()
                .filter_map(|entry| {
                    entry.hash.map(|digest| {
                        crate::json::Value::Object(vec![
                            (
                                "source_relative".into(),
                                crate::json::Value::String(
                                    entry
                                        .source
                                        .strip_prefix(&self.source)
                                        .unwrap()
                                        .to_string_lossy()
                                        .into_owned(),
                                ),
                            ),
                            (
                                "package_relative".into(),
                                crate::json::Value::String(
                                    entry.destination.to_string_lossy().into_owned(),
                                ),
                            ),
                            ("bytes".into(), crate::json::Value::UInt(entry.bytes)),
                            (
                                "sha256".into(),
                                crate::json::Value::String(
                                    digest.iter().map(|b| format!("{b:02x}")).collect(),
                                ),
                            ),
                        ])
                    })
                })
                .collect();
            let report=crate::json::Value::Object(vec![
                ("schema".into(),crate::json::Value::UInt(1)),
                ("status".into(),crate::json::Value::String("source copies verified; preparation requires the completion marker; build and portability testing remain required".into())),
                ("source_files".into(),crate::json::Value::Array(source_files)),
                ("notes".into(),crate::json::Value::String("Copied bytes matched the reviewed SHA-256 values. Original sources were not changed. Permission declarations are user-supplied. This is not official approval or a compiler log. Trusted private directories are required; filesystem race containment, ACLs and alternate data streams are not provided.".into())),
            ]);
            let report_path = wrapper.join("preparation-report.json");
            write_new(
                &report_path,
                crate::json::to_json_string_pretty(&report).as_bytes(),
            )?;
            cancelled(cancel)?;
            let pending_marker = wrapper.join("preparation-complete.pending");
            let marker = wrapper.join("preparation-complete.txt");
            write_new(&pending_marker,b"Preparation completed. This is not a build, portability test, or official approval. Original sources were not modified.\r\n")?;
            cancelled(cancel)?;
            if marker.try_exists()? {
                return Err(anyhow!(
                    "Completion marker unexpectedly exists; review the new output folder"
                ));
            }
            fs::rename(pending_marker, marker)?;
            Ok(Prepared {
                package: package.clone(),
                record: record.clone(),
                report: report_path,
                files: self.file_count(),
                bytes: self.total_bytes,
            })
        };
        work().with_context(||format!("Preparation stopped. New partial output is retained at {}. Original source files were not changed; review again and prepare into another new folder.",wrapper.display()))
    }
}
fn write_new(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut f = OpenOptions::new().write(true).create_new(true).open(path)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (crate::util::TestDir, Project) {
        let temp = crate::util::tempdir().unwrap();
        let source = temp.path().join("source");
        let workspace = temp.path().join("workspace");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&workspace).unwrap();
        let mut exe = vec![0u8; 88];
        exe[..2].copy_from_slice(b"MZ");
        exe[60..64].copy_from_slice(&64u32.to_le_bytes());
        exe[64..68].copy_from_slice(b"PE\0\0");
        fs::write(source.join("example.exe"), exe).unwrap();
        let icon = temp.path().join("icon.png");
        fs::write(
            &icon,
            crate::png::encode_rgba(256, 256, &vec![255; 256 * 256 * 4]).unwrap(),
        )
        .unwrap();
        let p = Project {
            source: source.to_string_lossy().into(),
            workspace: workspace.to_string_lossy().into(),
            name: "Example 工具".into(),
            app_id: "ExamplePortable".into(),
            publisher: "Example".into(),
            homepage: "https://example.org".into(),
            description: "Example".into(),
            category: "Utilities".into(),
            language: "English".into(),
            package_version: "1.0.0.0".into(),
            display_version: "1.0".into(),
            program: "Example/example.exe".into(),
            portability_notes: "Needs clean-machine testing".into(),
            icon: icon.to_string_lossy().into(),
            rights: Some(true),
            open_source: Some(false),
            freeware: Some(true),
            commercial_use: Some(false),
            shareable: Some(false),
            ..Default::default()
        };
        (temp, p)
    }
    #[test]
    fn build_review_detects_changed_tools_and_requires_custom_code_consent() {
        let (_temp, mut p) = fixture();
        let tool = Path::new(&p.source).join("example.exe");
        p.generator = tool.to_string_lossy().into_owned();
        p.installer = p.generator.clone();
        let prepared = StagePlan::create(&p, &|| false, &|_| {})
            .unwrap()
            .prepare(&|| false, &|_| {})
            .unwrap();
        let review = crate::pafbuildsession::Review::create(&prepared, &p, &|| false).unwrap();
        assert!(review.run(false, &|| true, &|_| {}).is_err());
        let mut bytes = fs::read(&tool).unwrap();
        bytes[80] ^= 1;
        fs::write(&tool, bytes).unwrap();
        assert!(review
            .run(false, &|| false, &|_| {})
            .unwrap_err()
            .to_string()
            .contains("changed"));
        let custom = prepared.package.join("App/AppInfo/Launcher/Custom.nsh");
        fs::write(custom, b"; must be reviewed").unwrap();
        let review = crate::pafbuildsession::Review::create(&prepared, &p, &|| false).unwrap();
        assert!(review
            .run(false, &|| false, &|_| {})
            .unwrap_err()
            .to_string()
            .contains("permit"));
        assert!(!fs::read_dir(prepared.record.parent().unwrap())
            .unwrap()
            .any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("build-")));
    }
    #[test]
    fn reconnects_only_complete_unchanged_preparation() {
        let (_temp, p) = fixture();
        let output = StagePlan::create(&p, &|| false, &|_| {})
            .unwrap()
            .prepare(&|| false, &|_| {})
            .unwrap();
        let restored = reopen(&output.record, &p, &|| false).unwrap().unwrap();
        assert_eq!(restored.package, output.package);
        assert_eq!(
            (restored.files, restored.bytes),
            (output.files, output.bytes)
        );
        assert!(reopen(&output.record, &p, &|| true).is_err());
        let application = output.package.join("App/Example/example.exe");
        let original = fs::read(&application).unwrap();
        let mut changed = original.clone();
        changed[80] ^= 1;
        fs::write(&application, changed).unwrap();
        assert!(reopen(&output.record, &p, &|| false)
            .unwrap_err()
            .to_string()
            .contains("changed"));
        fs::write(&application, original).unwrap();
        fs::write(output.package.join("App/AppInfo/appinfo.ini"), b"changed").unwrap();
        assert!(reopen(&output.record, &p, &|| false).is_err());
        fs::remove_file(
            output
                .record
                .parent()
                .unwrap()
                .join("preparation-complete.txt"),
        )
        .unwrap();
        assert!(reopen(&output.record, &p, &|| false).unwrap().is_none());
    }
    #[test]
    fn reconnect_rejects_report_traversal_and_invalid_marker() {
        let (_temp, p) = fixture();
        let output = StagePlan::create(&p, &|| false, &|_| {})
            .unwrap()
            .prepare(&|| false, &|_| {})
            .unwrap();
        let mut report = crate::json::parse_bytes(&fs::read(&output.report).unwrap()).unwrap();
        if let crate::json::Value::Object(fields) = &mut report {
            let entries = fields
                .iter_mut()
                .find(|(k, _)| k == "source_files")
                .unwrap();
            if let crate::json::Value::Array(entries) = &mut entries.1 {
                if let crate::json::Value::Object(fields) = &mut entries[0] {
                    fields
                        .iter_mut()
                        .find(|(k, _)| k == "package_relative")
                        .unwrap()
                        .1 = crate::json::Value::String("../project.json".into());
                }
            }
        }
        fs::write(&output.report, crate::json::to_json_string_pretty(&report)).unwrap();
        assert!(reopen(&output.record, &p, &|| false).is_err());
        fs::write(
            output
                .record
                .parent()
                .unwrap()
                .join("preparation-complete.txt"),
            b"incomplete",
        )
        .unwrap();
        assert!(reopen(&output.record, &p, &|| false).is_err());
    }
    #[test]
    fn stages_into_new_folder_preserving_originals_and_unicode_metadata() {
        let (_temp, p) = fixture();
        let plan = StagePlan::create(&p, &|| false, &|_| {}).unwrap();
        let original = fs::read(Path::new(&p.source).join("example.exe")).unwrap();
        let output = plan.prepare(&|| false, &|_| {}).unwrap();
        assert_eq!(
            fs::read(output.package.join("App/Example/example.exe")).unwrap(),
            original
        );
        assert_eq!(
            portable_builder_kit::package_app_id(&output.package).unwrap(),
            p.app_id
        );
        assert_eq!(Project::load(&output.record).unwrap(), p);
        assert_eq!(
            fs::read(Path::new(&p.source).join("example.exe")).unwrap(),
            original
        );
        assert!(!output
            .package
            .join("App/AppInfo/pac_installer_log.ini")
            .exists());
        assert!(plan.prepare(&|| false, &|_| {}).is_err());
        assert!(output.package.is_dir());
    }
    #[test]
    fn cancellation_and_changed_source_cannot_be_success() {
        let (_temp, p) = fixture();
        assert!(StagePlan::create(&p, &|| true, &|_| {}).is_err());
        let plan = StagePlan::create(&p, &|| false, &|_| {}).unwrap();
        assert!(plan.prepare(&|| true, &|_| {}).is_err());
        assert_eq!(fs::read_dir(&p.workspace).unwrap().count(), 0);
        fs::write(Path::new(&p.source).join("example.exe"), b"changed").unwrap();
        assert!(plan.prepare(&|| false, &|_| {}).is_err());
        let wrapper = fs::read_dir(&p.workspace)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert!(!wrapper.join("preparation-complete.txt").exists());
        assert!(wrapper.join("project.json").is_file());
    }
    #[test]
    fn same_size_edits_are_detected_by_the_reviewed_hash() {
        let (_temp, p) = fixture();
        let plan = StagePlan::create(&p, &|| false, &|_| {}).unwrap();
        let source = Path::new(&p.source).join("example.exe");
        let mut data = fs::read(&source).unwrap();
        data[87] ^= 1;
        fs::write(&source, &data).unwrap();
        assert!(plan.prepare(&|| false, &|_| {}).is_err());
        assert_eq!(fs::read(source).unwrap(), data);
        assert!(!plan.output().join("preparation-complete.txt").exists());
    }
    #[test]
    fn cancelling_after_copy_starts_retains_only_incomplete_new_output() {
        let (_temp, p) = fixture();
        let source = Path::new(&p.source).join("example.exe");
        let mut data = fs::read(&source).unwrap();
        data.resize(200_000, 42);
        fs::write(&source, &data).unwrap();
        let plan = StagePlan::create(&p, &|| false, &|_| {}).unwrap();
        let staged = plan
            .output()
            .join(&p.app_id)
            .join("App/Example/example.exe");
        let cancel = || fs::metadata(&staged).is_ok_and(|m| m.len() > 0);
        assert!(plan.prepare(&cancel, &|_| {}).is_err());
        let length = fs::metadata(staged).unwrap().len();
        assert!(length > 0 && length < 200_000);
        assert!(!plan.output().join("preparation-complete.txt").exists());
        assert_eq!(fs::read(source).unwrap(), data);
    }
    #[test]
    fn cancellation_during_final_report_does_not_publish_completion() {
        let (_temp, p) = fixture();
        let plan = StagePlan::create(&p, &|| false, &|_| {}).unwrap();
        let cancel = || plan.output().join("preparation-report.json").exists();
        assert!(plan.prepare(&cancel, &|_| {}).is_err());
        assert!(!plan.output().join("preparation-complete.txt").exists());
    }
    #[test]
    fn rejects_case_collisions_and_reserved_layout_folders() {
        let (_temp, mut p) = fixture();
        p.program = "AppInfo/example.exe".into();
        assert!(StagePlan::create(&p, &|| false, &|_| {}).is_err());
        #[cfg(unix)]
        {
            p.program = "Example/example.exe".into();
            fs::write(Path::new(&p.source).join("EXAMPLE.EXE"), b"other").unwrap();
            assert!(StagePlan::create(&p, &|| false, &|_| {}).is_err());
        }
    }
    #[cfg(unix)]
    #[test]
    fn rejects_links_even_when_they_point_inside_the_source() {
        let (_temp, p) = fixture();
        std::os::unix::fs::symlink("example.exe", Path::new(&p.source).join("linked.exe")).unwrap();
        assert!(StagePlan::create(&p, &|| false, &|_| {}).is_err());
    }
}
