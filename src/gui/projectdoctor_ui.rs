//! Project Doctor controls and reports; scanning is explicit and read-only.
use super::{a11y, theme, widgets::pick_folder, Job};
use setupscope_core::{
    devcheck::Target,
    i18n::t,
    projectdoctor::{self, Report, State},
};
use std::{path::PathBuf, time::Duration};
#[derive(Default)]
pub struct ProjectDoctorState {
    root: String,
    target: Target,
    job: Option<Job<Result<Report, String>>>,
    report: Option<Report>,
    error: Option<String>,
}
impl Drop for ProjectDoctorState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}
#[derive(Default)]
struct Actions {
    browse: bool,
    inspect: bool,
}
fn controls(ui: &mut egui::Ui, state: &mut ProjectDoctorState) -> Actions {
    let mut action = Actions::default();
    ui.add_enabled_ui(state.job.is_none(),|ui|{
        let label=ui.label(t("Project folder"));
        a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.root).desired_width(ui.available_width().min(360.0)).char_limit(4096).hint_text(setupscope_core::i18n::t("C:\\src\\MyProject"))).labelled_by(label.id),&t("Choose an absolute local project folder. Known manifest/version files are read here; .NET and Rust declarations may also be read from up to 32 parent folders. Linked/reparse paths are rejected. No project scripts execute."));
        ui.horizontal_wrapped(|ui|{
            if a11y::explain(ui.button(t("Choose project folder…")),&t("Choose the folder to inspect. Choosing alone never starts a scan.")).clicked(){action.browse=true;}
        });
        ui.horizontal_wrapped(|ui|{
            let label=ui.label(t("Target architecture"));
            let response=egui::ComboBox::from_id_salt("doctor_target").width(64.).selected_text(state.target.label()).show_ui(ui,|ui|{
                ui.selectable_value(&mut state.target,Target::X64,"x64");ui.selectable_value(&mut state.target,Target::Arm64,"arm64");
            }).response.labelled_by(label.id);
            a11y::explain(response,&t("Choose the desired build target. PE architecture evidence is shown separately; a host tool, emulator or cross-compiler can differ from that target."));
            action.inspect=a11y::explain(ui.add_enabled(!state.root.trim().is_empty(),egui::Button::new(t("Inspect project"))),&t("Read selected manifests, local tool metadata, SDK directory candidates and inherited PATH in a background job. No build, script, package-manager command, install, repair, download or environment change runs. Results are not saved automatically.")).clicked();
        });
    });
    action
}
impl ProjectDoctorState {
    pub(super) fn busy(&self) -> bool { self.job.is_some() }
    pub(super) fn accept_evidence(&mut self, report: Report) {
        self.root=report.root.display().to_string(); self.target=report.target;
        self.report=Some(report); self.error=None;
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Ok(report) => {
                    self.report = Some(report);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        super::widgets::scope_note(ui, "Inspect project declarations and local metadata. Actual build success remains unverified.", "Project declarations and local evidence—not a build-success guarantee. Select a folder, then Inspect project. Results stay in memory until replaced or the app closes. Limits: 256 KiB per UTF-8 manifest, 2 MiB total, 32 parent folders, 16 SDK roots and 256 entries per SDK/Rust store. Unsupported constraints, overrides, inaccessible locations and unverified dependencies stay explicit.");
        
        let action = controls(ui, self);
        if action.browse {
            if let Some(path) = pick_folder(&t("Choose a project folder for read-only inspection"))
            {
                self.root = path.display().to_string();
            }
        }
        if action.inspect {
            let root = PathBuf::from(self.root.trim());
            let target = self.target;
            self.error = None;
            self.job = Some(Job::spawn(ui.ctx(), move |progress| {
                let request =
                    projectdoctor::Request::current(root, target).map_err(|e| format!("{e:#}"))?;
                projectdoctor::inspect(&request, progress).map_err(|e| format!("{e:#}"))
            }));
        }
        if let Some(job) = &self.job {
            ui.spinner();
            ui.label(setupscope_core::i18n::t(&format!(
                "{}: {}",
                t("Inspection checkpoints"),
                job.progress.count()
            )));
            if a11y::explain(ui.button(t("Cancel project inspection")),&t("Stop at the next read/metadata checkpoint. An operating-system read may take time to return. Partial/cancelled results cannot establish readiness.")).clicked(){job.cancel();}
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::BAD, error);
            ui.label(t("Previous successful report is retained below. Correct the folder/permissions/format and inspect again."));
        }
        if let Some(report) = &self.report {
            if std::path::Path::new(self.root.trim()) != report.root || self.target != report.target {
                theme::warning_banner(ui, t("Inputs differ from the retained report. Inspect again; this evidence belongs to the folder and target shown below."));
            }
            if self.job.is_some() {
                ui.label(t("Previous report below; inspection is still running."));
            }
            ui.add(
                egui::Label::new(setupscope_core::i18n::t(&format!(
                    "{}: {} ({})",
                    t("Report folder"),
                    report.root.display(),
                    report.target.label()
                )))
                .selectable(true),
            );
            ui.label(setupscope_core::i18n::t(&format!(
                "{}: {}",
                t("Observed (local time)"),
                report.observed_at
            )));
            if report.partial || report.cancelled {
                ui.colored_label(
                    theme::WARN,
                    t("Partial or cancelled inspection—unknowns remain."),
                );
            }
            if a11y::explain(ui.button(t("Copy project report")),&t("Copy this report, including local paths and tool locations, to the clipboard. It is not a redacted diagnostic bundle; review sensitive details before sharing. No file is saved or uploaded.")).clicked(){ui.ctx().copy_text(report.text());}
            for (i, row) in report.findings.iter().enumerate() {
                super::widgets::collapsing_header(ui,format!(
                    "{} — {}: {}",
                    row.tool,
                    t(row.state.label()),
                    row.declared
                ))
                .id_salt(("doctor_finding", i))
                .default_open(row.state != State::Candidate)
                .show(ui, |ui| {
                    let color = match row.state {
                        State::Candidate => theme::OK,
                        State::Different | State::Missing => theme::WARN,
                        _ => theme::MUTED,
                    };
                    ui.colored_label(color, t(row.state.label()));
                    ui.add(egui::Label::new(row.source.display().to_string()).selectable(true));
                    ui.label(&row.detail);
                    for evidence in &row.evidence {
                        ui.add(egui::Label::new(evidence).selectable(true));
                    }
                });
            }
            if report.findings.is_empty() {
                ui.label(t("No supported requirement declaration was found. Dependencies and build readiness remain unverified."));
            }
            for note in &report.notes {
                ui.label(setupscope_core::i18n::t(note));
            }
            super::widgets::collapsing_header(ui,t("PATH candidates and prerequisite evidence")).id_salt("doctor_path_evidence").show(ui,|ui|{
                ui.label(setupscope_core::i18n::t(&format!("{}: {}",t("Primary prerequisite profile"),report.tooling.profile.label())));
                ui.label(t("Mixed projects may require additional profiles in Developer diagnostics. Observed PATH order is this process's environment, not guaranteed IDE/shell/version-manager selection."));
                for check in &report.tooling.checks {ui.label(setupscope_core::i18n::t(&format!("{} — {}",check.name,t(check.status.label()))));ui.label(setupscope_core::i18n::t(check.guidance));for evidence in &check.evidence{ui.label(evidence);}}
                for tool in &report.tooling.tools{if !tool.candidates.is_empty(){ui.label(&tool.name);for path in &tool.candidates{ui.add(egui::Label::new(path.display().to_string()).selectable(true));}for note in &tool.notes{ui.label(setupscope_core::i18n::t(note));}}}
                for note in &report.tooling.notes{ui.label(setupscope_core::i18n::t(note));}
            });
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, Vec2};
    fn frame(
        ctx: &egui::Context,
        state: &mut ProjectDoctorState,
        events: Vec<Event>,
    ) -> (egui::FullOutput, bool) {
        let mut start = false;
        let out = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 400.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    start = controls(ui, state).inspect;
                });
            },
        );
        (out, start)
    }
    #[test]
    fn controls_are_explicit_and_accessible() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = ProjectDoctorState::default();
        assert!(!frame(&ctx, &mut state, vec![]).1);
        let (out, start) = frame(&ctx, &mut state, vec![]);
        assert!(!start);
        assert!(state.job.is_none());
        let nodes = out.platform_output.accesskit_update.unwrap();
        assert!(nodes.nodes.iter().any(|(_, n)| n
            .description()
            .is_some_and(|s| s.contains("No project scripts execute"))));
        assert!(nodes.nodes.iter().any(|(_, n)| n
            .description()
            .is_some_and(|s| s.contains("No build, script"))));
    }
    #[test]
    fn keyboard_can_request_scan_without_starting_work_in_controls() {
        let ctx = egui::Context::default();
        let mut state = ProjectDoctorState {
            root: "C:\\src\\fixture".into(),
            target: Target::X64,
            job: None,
            report: None,
            error: None,
        };
        frame(&ctx, &mut state, vec![]);
        let key = |key| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        for _ in 0..4 {
            frame(&ctx, &mut state, vec![key(Key::Tab)]);
        }
        assert!(frame(&ctx, &mut state, vec![key(Key::Enter)]).1);
        assert!(state.job.is_none());
    }
}
