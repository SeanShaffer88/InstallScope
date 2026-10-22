//! Composes existing bounded, read-only diagnostics. Never executes a build or repair.
use crate::{build_log, devcheck, projectdoctor, snapshot::Progress};
use anyhow::{ensure, Result};
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selection {
    pub project: PathBuf,
    pub target: devcheck::Target,
    pub fallback_profile: devcheck::Profile,
    pub log: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Doctor,
    BuildReadiness,
    BuildLog,
    Environment,
    Ports,
    Storage,
    Dependencies,
}

pub struct NextStep {
    pub destination: Destination,
    pub title: &'static str,
    pub guidance: &'static str,
}

pub struct Report {
    pub selection: Selection,
    pub project: projectdoctor::Report,
    pub log: Option<build_log::Report>,
    pub log_error: Option<String>,
    pub log_skipped: bool,
    pub finished_at: String,
    pub steps: Vec<NextStep>,
}

impl Report {
    pub fn incomplete(&self) -> bool {
        self.project.partial
            || self.project.cancelled
            || self.project.tooling.cancelled
            || self.project.tooling.path_partial
            || self.log_error.is_some()
            || self.log_skipped
            || self.log.as_ref().is_some_and(|r| r.partial || r.cancelled)
    }
    pub fn summary(&self) -> String {
        let mut text = format!(
            "Project Readiness — evidence, not build certification\nProject: {}\nTarget: {}\nRequested fallback profile: {}\nObserved prerequisite profile: {}\nProject observation: {}\nWorkflow finished: {}\nIncomplete evidence: {}\nDeclarations: {} · Nonmatching/unknown/review declarations: {}\nNo builds, scripts, repairs, installs, environment changes or uploads performed.\n",
            self.selection.project.display(), self.selection.target.label(), self.selection.fallback_profile.label(),
            self.project.tooling.profile.label(), self.project.observed_at, self.finished_at, self.incomplete(),
            self.project.findings.len(), self.project.findings.iter().filter(|f| f.state != projectdoctor::State::Candidate).count()
        );
        if let Some(path) = &self.selection.log {
            text.push_str(&format!("Selected saved log: {}\n", path.display()));
            if let Some(log) = &self.log {
                text.push_str(&format!(
                    "Log observed: {} · Recognized hint groups: {} · Partial: {} · Cancelled: {}\n",
                    log.observed,
                    log.findings.len(),
                    log.partial,
                    log.cancelled
                ));
            }
            if self.log_error.is_some() {
                text.push_str("Log inspection failed; project evidence is retained. Review the local error.\n");
            }
            if self.log_skipped {
                text.push_str("Log inspection skipped after cancellation.\n");
            }
            text.push_str("The saved log is user-associated, not proven to belong to this project, target or observation time. Hints are not confirmed causes. Raw log text is excluded.\n");
        }
        for step in &self.steps {
            text.push_str(&format!("\n{}\n{}\n", step.title, step.guidance));
        }
        text
    }
}

/// Uses Project Doctor's existing stack selection and its prerequisite/PATH report.
/// Log failure does not erase usable project evidence. No project recursion is added.
pub fn inspect(selection: &Selection, progress: &Progress) -> Result<Report> {
    ensure!(
        !progress.cancelled(),
        "Project Readiness cancelled before inspection"
    );
    if let Some(log) = &selection.log {
        ensure!(log.is_absolute(), "Select an absolute saved-log path");
    }
    let mut request = projectdoctor::Request::current(selection.project.clone(), selection.target)?;
    request.tools.profile = selection.fallback_profile;
    let project = projectdoctor::inspect(&request, progress)?;
    let mut report = Report {
        selection: selection.clone(),
        project,
        log: None,
        log_error: None,
        log_skipped: false,
        finished_at: String::new(),
        steps: vec![],
    };
    if let Some(path) = &selection.log {
        if progress.cancelled() {
            report.log_skipped = true;
        } else {
            progress.set_phase("Inspecting selected saved build log");
            match build_log::inspect(path, progress) {
                Ok(log) => report.log = Some(log),
                Err(error) => report.log_error = Some(error.to_string()),
            }
        }
    }
    report.finished_at = crate::util::now_string();
    report.steps = next_steps(&report);
    Ok(report)
}

fn next_steps(report: &Report) -> Vec<NextStep> {
    use Destination::*;
    let mut steps = vec![NextStep { destination: Doctor, title: "Review Project Requirements", guidance: "Open the project evidence to compare declarations, source files, candidate metadata and unsupported requirements. Matching metadata is not proof that the project builds." },
        NextStep { destination: BuildReadiness, title: "Review Toolchain And PATH Evidence", guidance: "Open the same observation in the Toolchain / PATH / Versions view. Check the selected architecture, candidate order and version evidence. Mixed projects can require additional profiles; this process may differ from your IDE or terminal." }];
    if report.project.tooling.path_partial
        || report
            .project
            .tooling
            .entries
            .iter()
            .any(|e| e.status != devcheck::Status::Found)
    {
        steps.push(NextStep {destination: Dependencies, title: "Review Missing Developer Dependencies", guidance: "Open Developer Dependencies for pinned .NET/CMake installers or separately reviewed Microsoft MSVC/Windows SDK/CMake/Ninja setup. Choose the components your project requires. A missing observation can reflect a custom location; installing is never automatic and does not establish readiness."});
        steps.push(NextStep { destination: Environment, title: "Review The Project Environment", guidance: "Compare this process environment with the intended developer shell. Environment Profiles opens separately; no project profile is inferred or applied and persistent changes retain their own review." });
    }
    if report.selection.log.is_some() {
        steps.push(NextStep { destination: BuildLog, title: "Review Saved-Log Hints", guidance: "Inspect source line references in the original log. The selected file is not proven to belong to this project or run; pattern matches can be quoted, incidental or downstream symptoms." });
    }
    if let Some(log) = &report.log {
        let has = |id| {
            log.findings.iter().any(|f| {
                build_log::RULES
                    .get(f.rule)
                    .is_some_and(|rule| rule.id == id)
            })
        };
        if has("lnk1104") || has("access-denied") {
            steps.push(NextStep { destination: Ports, title: "Investigate Possible File Holders", guidance: "If the original error identifies a file, explicitly select it in Ports And Processes. A linker or access error does not establish a lock; do not automatically unlock files, terminate processes or elevate." });
        }
        if has("disk-full") {
            steps.push(NextStep { destination: Storage, title: "Review Disk-Space Hints", guidance: "Review the original log in Build Troubleshooting and inspect the relevant volume in Windows. A disk-space hint does not establish which volume filled up; SetupScope never deletes caches or project files." });
        }
    }
    steps
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("readiness-{}", crate::util::new_id()));
            fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn selection(&self) -> Selection {
            Selection {
                project: self.0.clone(),
                target: devcheck::Target::X64,
                fallback_profile: devcheck::Profile::Cmake,
                log: None,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn connects_evidence_and_hints_without_writing_or_copying_raw_log_text() {
        let f = Fixture::new();
        let log = f.0.join("example.log");
        let bytes = b"PRIVATE_SENTINEL\nerror LNK1104\nENOSPC\n";
        fs::write(&log, bytes).unwrap();
        fs::write(
            f.0.join("build.rs"),
            "Do not execute or interpret this file",
        )
        .unwrap();
        let mut selection = f.selection();
        selection.log = Some(log.clone());
        let report = inspect(&selection, &Progress::default()).unwrap();
        assert!(report
            .steps
            .iter()
            .any(|s| s.destination == Destination::Ports));
        assert!(report
            .steps
            .iter()
            .any(|s| s.destination == Destination::Storage));
        assert_eq!(fs::read(&log).unwrap(), bytes);
        assert_eq!(fs::read_dir(&f.0).unwrap().count(), 2);
        assert!(!report.summary().contains("PRIVATE_SENTINEL"));
        assert!(report.summary().contains("not proven to belong"));
        assert_eq!(report.project.tooling.profile, devcheck::Profile::Cmake);
    }
    #[test]
    fn failed_log_is_incomplete_and_preserves_project_evidence() {
        let f = Fixture::new();
        let mut selection = f.selection();
        selection.log = Some(f.0.join("missing.log"));
        let report = inspect(&selection, &Progress::default()).unwrap();
        assert!(report.log_error.is_some() && report.incomplete());
        assert_eq!(report.project.root, f.0);
        assert!(report.summary().contains("project evidence is retained"));
    }
    #[test]
    fn rejects_bad_inputs_and_respects_preexisting_cancellation() {
        let f = Fixture::new();
        let mut selection = f.selection();
        selection.log = Some("relative.log".into());
        assert!(inspect(&selection, &Progress::default()).is_err());
        selection.log = None;
        let progress = Progress::default();
        progress
            .cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(inspect(&selection, &progress).is_err());
        selection.project = f.0.join("missing");
        assert!(inspect(&selection, &Progress::default()).is_err());
    }
}
