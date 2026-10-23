//! Explicit Project Readiness orchestration; shared diagnostics keep their own controls.
use super::{
    a11y,
    app::{App, Tab},
    theme, widgets, Job,
};
use setupscope_core::{
    devcheck::{Profile, Target},
    project_readiness::{self, Destination, Selection},
};
use std::{path::PathBuf, time::Duration};

#[derive(Default)]
pub struct ReadinessState {
    project: String,
    log: String,
    target: Target,
    fallback: Profile,
    job: Option<Job<Result<project_readiness::Report, String>>>,
    report: Option<project_readiness::Report>,
    error: Option<String>,
}
impl Drop for ReadinessState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}
impl ReadinessState {
    pub(super) fn current_project_evidence(&self) -> Option<&setupscope_core::projectdoctor::Report> {
        self.current().then(|| &self.report.as_ref().expect("current evidence").project)
    }
    pub(super) fn poll(&mut self) {
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
    }
    pub(super) fn board_status(&self) -> &'static str {
        if self.job.is_some() {
            "Inspection Running"
        } else if self.error.is_some() {
            "Inspection Failed — Previous Evidence Retained"
        } else if self.report.is_none() {
            "Not Inspected — No Automatic Scans"
        } else if !self.current() {
            "Inputs Changed Or Evidence Cancelled — Inspect Again"
        } else if self.report.as_ref().is_some_and(|r| r.incomplete()) {
            "Incomplete Evidence — Review Required"
        } else {
            "Evidence Collected — Actual Build Unverified"
        }
    }
    fn selection(&self) -> Selection {
        Selection {
            project: PathBuf::from(self.project.trim()),
            target: self.target,
            fallback_profile: self.fallback,
            log: (!self.log.trim().is_empty()).then(|| PathBuf::from(self.log.trim())),
        }
    }
    fn current(&self) -> bool {
        self.job.is_none()
            && self.error.is_none()
            && self.report.as_ref().is_some_and(|r| {
                r.selection == self.selection()
                    && !r.project.cancelled
                    && !r.project.tooling.cancelled
                    && !r.log_skipped
                    && !r.log.as_ref().is_some_and(|l| l.cancelled)
            })
    }
}

#[derive(Default)]
struct Actions {
    browse: bool,
    log: bool,
    inspect: bool,
}
fn controls(ui: &mut egui::Ui, state: &mut ReadinessState) -> Actions {
    let mut actions = Actions::default();
    ui.add_enabled_ui(state.job.is_none(), |ui| {
        let label=ui.label(setupscope_core::i18n::t("Project Folder"));
        a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.project).char_limit(4096).desired_width(ui.available_width().min(550.0)).hint_text(setupscope_core::i18n::t("C:\\Dev\\ExampleProject"))).labelled_by(label.id), "Select an existing absolute local folder. Known manifests and bounded ancestor declarations are inspected; no recursive source scan or project code execution.");
        ui.horizontal_wrapped(|ui| {
            actions.browse=a11y::explain(ui.button(setupscope_core::i18n::t("Choose Project Folder…")),"Choosing a folder changes the input only. A separate inspection action starts the checks.").clicked();
            let label=ui.label(setupscope_core::i18n::t("Target"));
            egui::ComboBox::from_id_salt("readiness_target").width(64.).selected_text(state.target.label()).show_ui(ui, |ui| {
                ui.selectable_value(&mut state.target,Target::X64,"x64"); ui.selectable_value(&mut state.target,Target::Arm64,"arm64");
            }).response.labelled_by(label.id).on_hover_text(setupscope_core::i18n::t("Requested build architecture, not a claim about host architecture or runtime compatibility."));
        });
        ui.horizontal_wrapped(|ui| {
            let label=ui.label(setupscope_core::i18n::t("Fallback Profile"));
            egui::ComboBox::from_id_salt("readiness_fallback").selected_text(setupscope_core::i18n::t(state.fallback.label())).show_ui(ui, |ui| {
                for profile in Profile::ALL { ui.selectable_value(&mut state.fallback,profile,setupscope_core::i18n::t(profile.label())); }
            }).response.labelled_by(label.id).on_hover_text(setupscope_core::i18n::t("Project Doctor selects a profile from recognized declarations; this fallback is used when none selects a supported stack. Mixed projects may require additional separate profile scans."));
        });
        let label=ui.label(setupscope_core::i18n::t("Saved Build Log (Optional)"));
        a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.log).char_limit(4096).desired_width(ui.available_width().min(550.0)).hint_text(setupscope_core::i18n::t("Leave empty for setup checks only"))).labelled_by(label.id),"One explicitly chosen saved file, at most 2 MiB. UTF-8 or BOM UTF-16; 20,000 lines and 8 KiB per line. No folder search, log discovery or raw-log export.");
        ui.horizontal_wrapped(|ui| {
            actions.log=a11y::explain(ui.button(setupscope_core::i18n::t("Choose Saved Log…")),"Attach a saved log for troubleshooting; no command is run and ownership of the log is not inferred.").clicked();
            if a11y::explain(ui.add_enabled(!state.log.is_empty(),egui::Button::new(setupscope_core::i18n::t("Clear Log"))), "Detach the log from the next inspection. Does not delete or modify the log file; existing evidence is retained and labeled stale.").clicked() { state.log.clear(); }
            actions.inspect=a11y::explain(super::widgets::action_button(ui,!state.project.trim().is_empty(),"Inspect Project Readiness"),"Explicitly read bounded project declarations, prerequisite/PATH/version metadata and the optional saved log in one background workflow. No install, build, script, repair, environment change or upload occurs.").clicked();
        });
    });
    actions
}

fn route(destination: Destination) -> Tab {
    match destination {
        Destination::Doctor => Tab::Doctor,
        Destination::BuildReadiness => Tab::Developer,
        Destination::BuildLog => Tab::BuildLog,
        Destination::Environment => Tab::Environment,
        Destination::Ports => Tab::Ports,
        Destination::Storage => Tab::BuildLog,
        Destination::Dependencies => Tab::Dependencies,
    }
}

impl App {
    pub fn ui_project_readiness(&mut self, ctx: &egui::Context) {
        self.readiness.poll();
        let mut open = None;
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            super::tool_window::scroll_body(ui,"project_readiness_workflow", |ui| {
                ui.label(setupscope_core::i18n::t("Connect project requirements, toolchain / PATH evidence and optional saved-log hints. Results are observations, not proof that a project builds."));
                let action=theme::card(ui,|ui|controls(ui,&mut self.readiness));
                if action.browse { if let Some(path)=widgets::pick_folder("Choose Project Folder") { self.readiness.project=path.display().to_string(); } }
                if action.log { if let Some(path)=widgets::pick_file("Choose Saved Build Log","Saved log", &["log","txt","out"]) { self.readiness.log=path.display().to_string(); } }
                if action.inspect {
                    let selection=self.readiness.selection(); self.readiness.error=None;
                    self.readiness.job=Some(Job::spawn(ctx,move |progress| {
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(||project_readiness::inspect(&selection,progress).map_err(|e|format!("{e:#}"))))
                            .unwrap_or_else(|_|Err("Inspection failed unexpectedly. Previous evidence retained.".into()))
                    }));
                }
                if let Some(job)=&self.readiness.job {
                    ui.horizontal_wrapped(|ui|{ ui.spinner(); ui.label(job.progress.phase());
                        if ui.button(setupscope_core::i18n::t("Cancel Inspection")).on_hover_text(setupscope_core::i18n::t("Stop at the next checkpoint. Operating-system reads must return first. Cancelled evidence cannot establish readiness.")).clicked(){job.cancel();}
                    });
                    ctx.request_repaint_after(Duration::from_millis(150));
                }
                if let Some(error)=&self.readiness.error { ui.colored_label(theme::BAD,error); ui.label(setupscope_core::i18n::t("Previous evidence is retained; it is not the result of this failed attempt.")); }
                let current=self.readiness.current();
                let Some(report)=&self.readiness.report else {ui.label(setupscope_core::i18n::t("Choose a project, optionally attach its saved log, then inspect. Nothing scans automatically."));return;};
                ui.separator();
                ui.add(egui::Label::new(setupscope_core::i18n::t(&format!("Observed Project: {} · {} · {}",report.project.root.display(),report.project.target.label(),report.project.observed_at))).selectable(true));
                if !current { theme::warning_banner(ui,"Previous / changed / cancelled evidence. Inspect again before opening linked diagnostics with this evidence."); }
                if report.incomplete() {theme::warning_banner(ui,"Incomplete Evidence — some inputs or observations are unavailable, unsupported, partial or cancelled. Review details; no readiness verdict is available.");}
                else {ui.label(setupscope_core::i18n::t("Evidence Collected — actual build success remains unverified."));}
                ui.label(setupscope_core::i18n::t(&format!("{} declaration findings · {} prerequisite checks · {} PATH entries · Profile: {}",report.project.findings.len(),report.project.tooling.checks.len(),report.project.tooling.entries.len(),report.project.tooling.profile.label())));
                if report.project.findings.is_empty(){ui.label(setupscope_core::i18n::t("No recognized requirement declaration was found; this does not mean there are no prerequisites."));}
                super::widgets::collapsing_header(ui,setupscope_core::i18n::t("Requirements And Prerequisites Needing Review")).show(ui,|ui|{
                    for finding in report.project.findings.iter().filter(|f|f.state!=setupscope_core::projectdoctor::State::Candidate).take(12){ui.label(setupscope_core::i18n::t(&format!("{} · {} · {}",finding.tool,setupscope_core::i18n::t(finding.state.label()),finding.declared)));}
                    for check in report.project.tooling.checks.iter().filter(|c|c.status!=setupscope_core::devcheck::Status::Found).take(12){ui.label(setupscope_core::i18n::t(&format!("{} · {}",check.name,setupscope_core::i18n::t(check.status.label()))));ui.small(setupscope_core::i18n::t(check.guidance));}
                    ui.small(setupscope_core::i18n::t("First 12 declarations/checks shown per group. Requirements and Toolchain / PATH contain the full bounded report. Matching metadata is not a build guarantee."));
                });
                if let Some(error)=&report.log_error { ui.colored_label(theme::WARN,setupscope_core::i18n::t(&format!("Saved-log inspection failed: {error}"))); }
                if let Some(log)=&report.log {
                    ui.label(setupscope_core::i18n::t(&format!("Saved Log: {} · {} hint groups · Observed {}",log.path.display(),log.findings.len(),log.observed)));
                    ui.label(setupscope_core::i18n::t("User-associated log: its project, target and run are unverified. Hints are not confirmed causes."));
                    for finding in &log.findings {if let Some(rule)=setupscope_core::build_log::RULES.get(finding.rule){ui.label(setupscope_core::i18n::t(&format!("{} · {} occurrence(s) · Lines {:?}",rule.title,finding.occurrences,finding.lines)));}}
                }
                ui.heading(setupscope_core::i18n::t("Guided Next Steps"));
                if ui.add_enabled(current,egui::Button::new(setupscope_core::i18n::t("Prepare Build / Test…"))).on_hover_text(setupscope_core::i18n::t("Pass only the project folder to Build Troubleshooting. Choose a tool and arguments, then review before anything executes.")).clicked(){open=Some(Destination::BuildLog);}
                ui.label(setupscope_core::i18n::t("Linked tools open with this observation where supported. Environment and file-owner tools still require their own explicit inputs and review."));
                for step in &report.steps {
                    theme::card(ui,|ui|{
                        ui.label(egui::RichText::new(setupscope_core::i18n::t(step.title)).strong()); ui.label(setupscope_core::i18n::t(step.guidance));
                        let linked=matches!(step.destination,Destination::Doctor|Destination::BuildReadiness|Destination::BuildLog);
                        let enabled=current && (step.destination!=Destination::BuildLog || report.log.is_some());
                        if a11y::explain(ui.add_enabled(enabled,egui::Button::new(if linked {"Open This Evidence"} else {"Open Tool"})),"Reuse this evidence without starting another scan. Running diagnostics are never replaced. Configuration and repair actions remain separate and explicitly reviewed.").clicked(){open=Some(step.destination);}
                    }); ui.add_space(8.0);
                }
                if ui.button(setupscope_core::i18n::t("Copy Readiness Summary (Contains Local Paths)")).on_hover_text(setupscope_core::i18n::t("Copies bounded counts, observation times and fixed guidance, including project/log paths. No raw log lines, environment values or manifest values are included. Review before sharing.")).clicked(){ctx.copy_text(report.summary());}
                super::widgets::collapsing_header(ui,setupscope_core::i18n::t("Scope And Freshness")).show(ui,|ui|{
                    ui.label(setupscope_core::i18n::t("Reports stay in memory for this session. Input changes invalidate linked handoff; editing files or changing the environment can also make unchanged inputs stale. Reinspect after changes."));
                    ui.label(setupscope_core::i18n::t("Project Doctor: 256 KiB per manifest, 2 MiB total and up to 32 ancestors. One inferred prerequisite profile is not a full mixed-stack certification. Saved logs retain their existing bounded triage rules. No builds, scripts, installs, repairs, persistent environment changes or uploads."));
                });
            });
        });
        if let Some(destination) = open {
            let report = self
                .readiness
                .report
                .as_ref()
                .expect("action requires report");
            let accepted = match destination {
                Destination::Doctor | Destination::BuildReadiness => {
                    self.devcheck.accept_project_evidence(&report.project)
                }
                Destination::BuildLog => {
                    let accepted=self.devcheck.accept_build_project(&report.project.root);
                    accepted && report.log.as_ref().map(|log|self.devcheck.accept_log_evidence(log)).unwrap_or(true)
                },
                _ => true,
            };
            if accepted {
                self.open_tool(ctx, route(destination));
            } else {
                self.set_error_status("A linked diagnostic is still running. Finish or cancel it explicitly before replacing its evidence.");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, Vec2};
    fn frame(
        ctx: &egui::Context,
        state: &mut ReadinessState,
        events: Vec<Event>,
    ) -> (egui::FullOutput, bool) {
        let mut inspect = false;
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 600.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    inspect = controls(ui, state).inspect;
                });
            },
        );
        (output, inspect)
    }
    #[test]
    fn explicit_accessible_controls_do_not_scan_on_render() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = ReadinessState::default();
        frame(&ctx, &mut state, vec![]);
        let (out, start) = frame(&ctx, &mut state, vec![]);
        assert!(!start && state.job.is_none());
        let update = out.platform_output.accesskit_update.unwrap();
        assert!(update.nodes.iter().any(|(_, n)| n
            .description()
            .is_some_and(|s| s.contains("No install, build, script"))));
        assert!(
            update
                .nodes
                .iter()
                .filter(|(_, n)| !n.labelled_by().is_empty())
                .count()
                >= 4
        );
    }
    #[test]
    fn keyboard_requests_inspection_without_starting_worker_from_controls() {
        let ctx = egui::Context::default();
        let mut state = ReadinessState::default();
        state.project = "C:/Dev/fixture".into();
        frame(&ctx, &mut state, vec![]);
        let key = |key| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        for _ in 0..7 {
            frame(&ctx, &mut state, vec![key(Key::Tab)]);
        }
        assert!(frame(&ctx, &mut state, vec![key(Key::Enter)]).1);
        assert!(state.job.is_none());
    }
    #[test]
    fn changed_inputs_and_failed_refresh_cannot_reuse_previous_observation() {
        let folder =
            std::env::temp_dir().join(format!("readiness-ui-{}", setupscope_core::util::new_id()));
        std::fs::create_dir(&folder).unwrap();
        let mut state = ReadinessState::default();
        state.project = folder.display().to_string();
        state.report =
            Some(project_readiness::inspect(&state.selection(), &Default::default()).unwrap());
        assert!(state.current());
        state.log = "C:\\saved.log".into();
        assert!(!state.current());
        state.log.clear();
        state.target = Target::Arm64;
        assert!(!state.current());
        state.target = Target::X64;
        state.error = Some("Refresh failed".into());
        assert!(!state.current());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
