//! Reviewable official-tool builds in a previously prepared private workspace.
//! No downloads, shell commands, original-source edits, or certification claims.
use crate::{
    pafbuild::{self, Compiled, PaTools},
    pafproject::Project,
    pafstage::{self, Prepared},
};
use anyhow::{anyhow, Context, Result};
use portable_builder_kit::process::ToolRequest;
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

#[derive(Clone, Debug)]
pub struct Review {
    pub project: Project,
    pub prepared: Prepared,
    pub requests: [ToolRequest; 2],
    pub custom_code: Vec<PathBuf>,
    hashes: [[u8; 32]; 2],
}
#[derive(Clone, Debug)]
pub struct Attempt {
    pub directory: PathBuf,
    pub journal: PathBuf,
    pub result: Result<Compiled, String>,
    pub logs: Vec<PathBuf>,
    pub validation_report: Option<PathBuf>,
    pub validation_errors: Option<usize>,
    pub warnings: Vec<String>,
}
fn digest(path: &Path, cancel: &dyn Fn() -> bool) -> Result<[u8; 32]> {
    let size = fs::metadata(path)?.len();
    if size > 256 * 1024 * 1024 {
        return Err(anyhow!(
            "Selected tool exceeds the wizard's 256 MiB executable limit"
        ));
    }
    let digest = pafbuild::build_hash_limit(path, size + 1, cancel)?;
    if fs::metadata(path)?.len() != size {
        return Err(anyhow!("Selected tool or log changed while hashing"));
    }
    Ok(digest)
}
impl Review {
    pub fn create(
        prepared: &Prepared,
        project: &Project,
        cancel: &dyn Fn() -> bool,
    ) -> Result<Self> {
        let verified = pafstage::reopen(&prepared.record, project, cancel)?
            .context("Preparation is incomplete; prepare a new folder before building")?;
        let tools = PaTools {
            launcher_generator: Some(project.generator.clone().into()),
            installer: Some(project.installer.clone().into()),
            ..Default::default()
        };
        let requests = pafbuild::official_build_requests(&verified.package, &tools)?;
        let hashes = [
            digest(&requests[0].executable, cancel)?,
            digest(&requests[1].executable, cancel)?,
        ];
        let mut custom_code = Vec::new();
        for path in pafbuild::official_custom_code(&verified.package) {
            if path.try_exists()? {
                custom_code.push(path);
            }
        }
        Ok(Self {
            project: project.clone(),
            prepared: verified,
            requests,
            hashes,
            custom_code,
        })
    }
    /// Explicit action only. Logs and failed output are retained outside the package.
    /// Snapshot known official log paths; absence/staleness is reported separately.
    pub fn run(
        &self,
        allow_custom_code: bool,
        cancel: &dyn Fn() -> bool,
        report: &dyn Fn(&str),
    ) -> Result<Attempt> {
        if cancel() {
            return Err(anyhow!("Build cancelled before execution"));
        }
        let current = Self::create(&self.prepared, &self.project, cancel)?;
        if current.hashes != self.hashes
            || current.requests.iter().zip(&self.requests).any(|(a, b)| {
                a.executable != b.executable
                    || a.arguments != b.arguments
                    || a.working_directory != b.working_directory
            })
        {
            return Err(anyhow!(
                "Selected tools changed after review; review build commands again"
            ));
        }
        if !allow_custom_code && !current.custom_code.is_empty() {
            return Err(anyhow!(
                "Review custom NSIS code and explicitly permit it for this build"
            ));
        }
        let root = self
            .prepared
            .record
            .parent()
            .context("Missing preparation directory")?;
        let directory = root.join(format!("build-{}", crate::util::new_id()));
        fs::create_dir(&directory)
            .with_context(|| format!("Cannot create build results at {}", directory.display()))?;
        let attempt = || -> Result<Attempt> {
            let journal = directory.join("build-journal.txt");
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&journal)?;
            writeln!(file, "InstallScope build invocation journal. This is not a compiler log or official approval.")?;
            writeln!(
                file,
                "Package: {}\nCustom NSIS permitted for this attempt: {}",
                current.prepared.package.display(),
                allow_custom_code
            )?;
            for (i, request) in self.requests.iter().enumerate() {
                writeln!(file, "Stage {} executable: {}\nArguments (one item each): {:?}\nWorking directory: {:?}\nTimeout: {} seconds\nExecutable SHA256: {}", i+1, request.executable.display(), request.arguments, request.working_directory, request.timeout.as_secs(), self.hashes[i].iter().map(|b| format!("{b:02x}")).collect::<String>())?;
            }
            file.sync_all()?;
            let names = [
                "PortableApps.comLauncherGeneratorLog.txt",
                "PortableApps.comInstallerLog.txt",
            ];
            let paths: Vec<PathBuf> = self
                .requests
                .iter()
                .zip(names)
                .map(|(r, name)| r.executable.parent().unwrap().join("Data").join(name))
                .collect();
            let before: Vec<_> = paths.iter().map(|p| log_stamp(p).ok()).collect();
            let tools = PaTools {
                launcher_generator: Some(self.requests[0].executable.clone()),
                installer: Some(self.requests[1].executable.clone()),
                ..Default::default()
            };
            let journal_file = Mutex::new(file);
            let log_error = Mutex::new(None);
            let result = pafbuild::compile_official_with_cancel(
                &current.prepared.package,
                &self.project.app_id,
                &tools,
                allow_custom_code,
                &|message| {
                    report(message);
                    let mut file = journal_file.lock().unwrap();
                    if let Err(e) = writeln!(file, "{message}").and_then(|_| file.flush()) {
                        *log_error.lock().unwrap() = Some(e.to_string());
                    }
                },
                &|| cancel() || log_error.lock().unwrap().is_some(),
            )
            .map_err(|e| format!("{e:#}"));
            let mut warnings = Vec::new();
            let mut logs = Vec::new();
            if let Some(error) = log_error.into_inner().unwrap() {
                warnings.push(format!("Invocation journal write failed: {error}"));
            }
            for (i, path) in paths.iter().enumerate() {
                let capture = capture_log(path, &directory.join(names[i]), before[i].as_ref());
                match capture {
                    Ok(path) => logs.push(path),
                    Err(e) => warnings.push(format!(
                        "Compiler log not captured from {}: {e:#}",
                        path.display()
                    )),
                }
            }
            let (validation_report, validation_errors) = match write_validation_report(
                &directory,
                &current.prepared.package,
                &self.project,
                &result,
                &warnings,
                cancel,
            ) {
                Ok((path, errors)) => (Some(path), Some(errors)),
                Err(e) => {
                    warnings.push(format!("Validation report could not be completed: {e:#}"));
                    (None, None)
                }
            };
            let mut file = journal_file.into_inner().unwrap();
            if let Err(e) = writeln!(
                file,
                "Result: {result:?}\nLog capture warnings: {warnings:?}"
            )
            .and_then(|_| file.sync_all())
            {
                warnings.push(format!("Could not finish invocation journal: {e}"));
            }
            Ok(Attempt {
                directory: directory.clone(),
                journal,
                result,
                logs,
                validation_report,
                validation_errors,
                warnings,
            })
        };
        attempt().with_context(|| {
            format!(
                "Build attempt stopped; retained output: {}",
                directory.display()
            )
        })
    }
}
fn write_validation_report(
    directory: &Path,
    package: &Path,
    project: &Project,
    result: &Result<Compiled, String>,
    warnings: &[String],
    cancel: &dyn Fn() -> bool,
) -> Result<(PathBuf, usize)> {
    use crate::json::Value as V;
    let string = |s: &str| V::String(s.into());
    let mut errors: Vec<V> = portable_builder_kit::preflight(package, result.is_ok())
        .into_iter()
        .map(|f| {
            V::Object(vec![
                ("path".into(), string(&f.path.to_string_lossy())),
                ("message".into(), string(&f.message)),
                (
                    "classification".into(),
                    string("wizard static preflight; not complete format validation"),
                ),
            ])
        })
        .collect();
    if let Err(error) = result {
        errors.push(V::Object(vec![
            ("path".into(), string(&package.to_string_lossy())),
            ("message".into(), string(error)),
            ("classification".into(), string("build attempt")),
        ]));
    }
    let mut artifact = V::Null;
    if let Ok(Compiled {
        paf: Some(path), ..
    }) = result
    {
        match pafbuild::build_hash(path, cancel) {
            Ok(hash) => {
                artifact = V::Object(vec![
                    ("path".into(), string(&path.to_string_lossy())),
                    (
                        "sha256".into(),
                        string(&hash.iter().map(|b| format!("{b:02x}")).collect::<String>()),
                    ),
                ])
            }
            Err(e) => errors.push(V::Object(vec![
                ("path".into(), string(&path.to_string_lossy())),
                ("message".into(), string(&e.to_string())),
                ("classification".into(), string("artifact hash")),
            ])),
        }
    }
    let mut report_warnings: Vec<V> = warnings.iter().map(|s| string(s)).collect();
    for warning in ["Static preflight is partial: complete metadata, icon content, configuration semantics and host-write behavior are not fully validated.","Redistribution and component permissions are user declarations requiring human review, not legal conclusions.","Manual test records are user observations; this wizard did not execute those tests."] {report_warnings.push(string(warning));}
    let error_count = errors.len();
    let value=V::Object(vec![
        ("schema".into(),V::UInt(1)),("app_id".into(),string(&project.app_id)),
        ("build_succeeded".into(),V::Bool(result.is_ok())),("automated_checks_passed".into(),V::Bool(errors.is_empty())),
        ("validation_complete".into(),V::Bool(false)),("portability_verified".into(),V::Bool(false)),("official_approval".into(),V::Bool(false)),
        ("format_reference".into(),string(crate::pafproject::FORMAT_SOURCE)),
        ("artifact".into(),artifact),("blocking_errors".into(),V::Array(errors)),("warnings".into(),V::Array(report_warnings)),
        ("manual_tests_reported_by_user".into(),V::Array(project.tests.iter().map(|s|string(s)).collect())),
        ("project_snapshot_note".into(),string("Save a new project snapshot to retain edited answers, permission declarations and manual observations. Review private paths before sharing reports.")),
    ]);
    let path = directory.join("validation-report.json");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    file.write_all(crate::json::to_json_string_pretty(&value).as_bytes())?;
    file.sync_all()?;
    Ok((path, error_count))
}
fn log_stamp(path: &Path) -> Result<(std::time::SystemTime, [u8; 32])> {
    let m = fs::symlink_metadata(path)?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 {
            return Err(anyhow!("Reparse log refused"));
        }
    }
    if !m.is_file() || m.file_type().is_symlink() || m.len() > 16 * 1024 * 1024 {
        return Err(anyhow!(
            "Log is not a regular file within the capture limit"
        ));
    }
    Ok((m.modified()?, digest(path, &|| false)?))
}

fn capture_log(
    path: &Path,
    destination: &Path,
    before: Option<&(std::time::SystemTime, [u8; 32])>,
) -> Result<PathBuf> {
    let stamp = log_stamp(path)?;
    if before == Some(&stamp) {
        return Err(anyhow!("No changed log was observed"));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 16 * 1024 * 1024 {
        return Err(anyhow!(
            "Log exceeds the 16 MiB capture limit; inspect the original log"
        ));
    }
    if crate::util::sha256(&bytes) != stamp.1 {
        return Err(anyhow!(
            "Log changed during capture; inspect the original log"
        ));
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)?;
    output.write_all(&bytes)?;
    output.sync_all()?;
    Ok(destination.to_path_buf())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_separates_failed_builds_and_manual_observations() {
        let temp = crate::util::tempdir().unwrap();
        let package = temp.path().join("missing package");
        let project = Project {
            app_id: "ExamplePortable".into(),
            tests: vec!["Launch and change settings".into()],
            ..Default::default()
        };
        let (path, errors) = write_validation_report(
            temp.path(),
            &package,
            &project,
            &Err("compiler failed".into()),
            &[],
            &|| false,
        )
        .unwrap();
        assert!(errors > 1);
        let value = crate::json::parse_bytes(&fs::read(path).unwrap()).unwrap();
        assert_eq!(value["build_succeeded"].as_bool(), Some(false));
        assert_eq!(value["automated_checks_passed"].as_bool(), Some(false));
        assert_eq!(value["validation_complete"].as_bool(), Some(false));
        assert_eq!(value["portability_verified"].as_bool(), Some(false));
        assert_eq!(value["official_approval"].as_bool(), Some(false));
        assert_eq!(
            value["manual_tests_reported_by_user"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert!(value["blocking_errors"].as_array().unwrap().len() > 1);
        assert!(write_validation_report(
            temp.path(),
            &package,
            &project,
            &Err("retry".into()),
            &[],
            &|| false
        )
        .is_err());
    }
    #[test]
    fn retains_only_new_logs_without_overwriting_previous_results() {
        let temp = crate::util::tempdir().unwrap();
        let source = temp.path().join("official-log.txt");
        let copy = temp.path().join("captured.txt");
        assert!(log_stamp(&source).is_err());
        fs::write(&source, b"old diagnostics").unwrap();
        let before = log_stamp(&source).unwrap();
        assert!(capture_log(&source, &copy, Some(&before)).is_err());
        assert!(!copy.exists());
        let expected = b"\xff\xfeN\0e\0w\0";
        fs::write(&source, expected).unwrap();
        capture_log(&source, &copy, Some(&before)).unwrap();
        assert_eq!(fs::read(&copy).unwrap(), expected);
        fs::write(&source, b"third result").unwrap();
        assert!(capture_log(&source, &copy, None).is_err());
        assert_eq!(fs::read(&copy).unwrap(), expected);
        let large = File::create(&source).unwrap();
        large.set_len(16 * 1024 * 1024 + 1).unwrap();
        assert!(capture_log(&source, &temp.path().join("large.txt"), None).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn refuses_linked_compiler_logs() {
        let temp = crate::util::tempdir().unwrap();
        let source = temp.path().join("actual.txt");
        let link = temp.path().join("link.txt");
        fs::write(&source, b"diagnostics").unwrap();
        std::os::unix::fs::symlink(&source, &link).unwrap();
        assert!(capture_log(&link, &temp.path().join("copy.txt"), None).is_err());
    }
}
