//! Developer workspace: read-only diagnostics and separately reviewed workflows.
use super::{a11y, app::App, theme, Job};
use egui::RichText;
use setupscope_core::{
    devcheck::{self, Profile, Report, Status, Target},
    i18n::t,
};
use std::time::Duration;

#[derive(Default)]
pub struct DevCheckState {
    dependencies: super::dependency_ui::DependencyState,
    build_log: super::build_log_ui::BuildLogState,
    build_run: super::build_run_ui::RunState,
    container: super::container_readiness_ui::ContainerState,
    bundle: super::diagnostic_bundle_ui::BundleState,
    doctor: super::projectdoctor_ui::ProjectDoctorState,
    inspector: super::devinspect_ui::InspectorState,
    environment: super::envprofile_ui::EnvironmentState,
    profile: Profile,
    target: Target,
    command: String,
    minimum_version: String,
    maximum_version: String,
    job: Option<Job<Result<Report, String>>>,
    report: Option<Report>,
    report_inputs: Option<(Profile, Target, String, String, String)>,
    error: Option<String>,
}

impl DevCheckState {
    fn inputs(&self) -> (Profile, Target, String, String, String) {
        (self.profile, self.target, self.command.clone(), self.minimum_version.clone(), self.maximum_version.clone())
    }
    pub(super) fn accept_project_evidence(&mut self, report: &setupscope_core::projectdoctor::Report) -> bool {
        if self.job.is_some() || self.doctor.busy() {return false;}
        self.profile=report.tooling.profile; self.target=report.target;
        self.command=report.tooling.version_command.clone();
        self.minimum_version.clear(); self.maximum_version.clear();
        self.report_inputs=Some(self.inputs());
        self.report=Some(report.tooling.clone()); self.error=None;
        self.doctor.accept_evidence(report.clone()); true
    }
    pub(super) fn accept_log_evidence(&mut self, report: &setupscope_core::build_log::Report) -> bool { self.build_log.accept_evidence(report) }
    pub(super) fn accept_build_project(&mut self, path: &std::path::Path) -> bool { self.build_run.accept_project(path) }
}

impl Drop for DevCheckState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}

fn controls(ui: &mut egui::Ui, state: &mut DevCheckState) -> bool {
    let mut start = false;
    ui.add_enabled_ui(state.job.is_none(), |ui| {
        ui.horizontal_wrapped(|ui| {
            let label = ui.label(t("Toolchain profile"));
            let combo = egui::ComboBox::from_id_salt("devcheck_profile").selected_text(t(state.profile.label())).show_ui(ui, |ui| {
                for profile in Profile::ALL { a11y::explain(ui.selectable_value(&mut state.profile, profile, t(profile.label())), &t("Choose which prerequisite file groups to inspect; no tools are executed.")); }
            });
            a11y::explain(combo.response.labelled_by(label.id), &t("Select the build environment to check. This does not read or run a project's build scripts."));
        });
        ui.horizontal_wrapped(|ui| {
            let label = ui.label(t("Windows target"));
            let combo = egui::ComboBox::from_id_salt("devcheck_target").selected_text(state.target.label()).show_ui(ui, |ui| {
                for target in [Target::X64, Target::Arm64] { a11y::explain(ui.selectable_value(&mut state.target, target, target.label()), &t("Choose the Windows architecture for MSVC, SDK and Rust target-directory checks.")); }
            });
            a11y::explain(combo.response.labelled_by(label.id), &t("Architecture affects file-layout checks only. Executable architecture and runtime compatibility remain unverified."));
        });
        ui.horizontal_wrapped(|ui| {
            let label = ui.label(t("Additional command (optional)"));
            a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.command).hint_text(t("e.g. node or python.exe")).desired_width(180.0).char_limit(64)).labelled_by(label.id), &t("Inspect one additional command name, such as javac or gcc.exe. Do not enter arguments or paths. The command is never executed. Version requirements apply to this name, or to the profile's primary command if blank."));
        });
        ui.horizontal_wrapped(|ui| {
            let label = ui.label(t("Minimum version (inclusive)"));
            a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.minimum_version).hint_text(setupscope_core::i18n::t("No minimum")).desired_width(95.0).char_limit(23)).labelled_by(label.id), &t("Optional numeric minimum: 1–4 components, each 0–65535. Embedded product metadata must be at least this value. No semantic-version operators or prerelease tags."));
            let label = ui.label(t("Maximum version (exclusive)"));
            a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.maximum_version).hint_text(setupscope_core::i18n::t("No maximum")).desired_width(95.0).char_limit(23)).labelled_by(label.id), &t("Optional numeric upper limit: the embedded version must be strictly below this value. Blank leaves the upper limit open. This is a numeric metadata comparison, not a project compatibility guarantee."));
            start = a11y::explain(super::widgets::action_button(ui,true,&t("Inspect build readiness")), &t("Read the current process PATH and local prerequisite metadata in the background. No commands, project scripts, builds, downloads or environment changes are performed (F5).")).clicked();
        });
    });
    start
}

fn status(ui: &mut egui::Ui, state: Status) {
    let color = match state {
        Status::Found => theme::OK,
        Status::Missing | Status::Review => theme::WARN,
        Status::Unknown => theme::MUTED,
    };
    ui.colored_label(color, t(state.label()));
}

impl App {
    pub fn start_devcheck(&mut self, ctx: &egui::Context) {
        if self.devcheck.job.is_some() {
            return;
        }
        let (profile, target, command) = (
            self.devcheck.profile,
            self.devcheck.target,
            self.devcheck.command.clone(),
        );
        let (minimum, maximum) = (
            self.devcheck.minimum_version.clone(),
            self.devcheck.maximum_version.clone(),
        );
        self.devcheck.error = None;
        self.devcheck.job = Some(Job::spawn(ctx, move |progress| {
            let mut request = devcheck::Request::current(profile, target, command)
                .map_err(|e| format!("{e:#}"))?;
            request.minimum_version = minimum;
            request.maximum_version = maximum;
            devcheck::inspect(&request, progress).map_err(|e| format!("{e:#}"))
        }));
    }

    pub fn ui_developer_tool(&mut self, ctx: &egui::Context, tool: super::app::Tab) {
        use super::app::Tab;
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            super::tool_window::scroll_body(ui,format!("developer_tool_body-{tool:?}"), |ui| {
                match tool {
                    Tab::BuildLog => {
                        if let Some(path)=self.devcheck.build_run.ui(ui, &self.store.root) { self.devcheck.build_log.select_path(path); }
                        self.devcheck.build_log.ui(ui);
                        ui.separator();
                        super::widgets::collapsing_header(ui,setupscope_core::i18n::t("Diagnostic Export — Preview And Save")).show(ui, |ui| self.devcheck.bundle.ui(ui, self.devcheck.report.as_ref()));
                    },
                    Tab::Containers => self.devcheck.container.ui(ui),
                    Tab::Dependencies => self.devcheck.dependencies.ui(ui, &self.store.root),
                    Tab::Bundle => self.devcheck.bundle.ui(ui, self.devcheck.report.as_ref()),
                    Tab::Doctor => self.devcheck.doctor.ui(ui),
                    Tab::Ports => self.devcheck.inspector.ui(ui),
                    Tab::Environment => self.devcheck.environment.ui(ui, &self.store.root),
                    _ => {},
                }
            });
        });
    }

    pub fn ui_devcheck(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.devcheck.job.as_ref().and_then(Job::poll) {
            self.devcheck.job = None;
            match result {
                Ok(report) => {
                    self.devcheck.report_inputs = Some(self.devcheck.inputs());
                    self.devcheck.report = Some(report);
                    self.devcheck.error = None;
                }
                Err(error) => self.devcheck.error = Some(error),
            }
        }
        let mut start = false;
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            super::tool_window::scroll_body(ui,"developer_diagnostics", |ui| {
                a11y::focus_hint(ui, theme::MUTED);
                super::widgets::scope_note(ui, "Read-only prerequisite, PATH and version evidence. No commands or environment changes.", "The diagnostic scan below is read-only: it executes no commands, changes no environment values and saves no results automatically. Environment Profiles has separate reviews for launching and persistent edits. Found means a file or directory was observed, not that a project will build. Numeric version checks use embedded Windows product metadata only. Binary architectures, SDK selection, project requirements and actual tool behavior remain unverified.");
                
                start = theme::card(ui, |ui| controls(ui, &mut self.devcheck));
                if let Some(job) = &self.devcheck.job {
                    ui.spinner(); ui.label(setupscope_core::i18n::t(&format!("{}: {}", t("Metadata checks"), job.progress.count())));
                    if a11y::explain(ui.button(t("Cancel inspection")), &t("Stop at the next metadata checkpoint. Partial results remain labeled; an operating-system call may take time to return.")).clicked() { job.cancel(); }
                    ctx.request_repaint_after(Duration::from_millis(150));
                }
                if let Some(error) = &self.devcheck.error { ui.colored_label(theme::BAD, error); }
                if self.devcheck.job.is_some() { ui.label(t("Previous report remains below while inspection runs.")); }
                let Some(report) = &self.devcheck.report else { ui.label(t("Choose a profile, then inspect. Nothing is scanned automatically.")); return; };
                if self.devcheck.report_inputs.as_ref() != Some(&self.devcheck.inputs()) {
                    theme::warning_banner(ui, t("Inputs changed since the displayed inspection. Inspect again; retained evidence belongs to the earlier profile, target, command and version range."));
                }
                ui.separator();
                ui.heading(format!("{} · {}", t(report.profile.label()), report.target.label()));
                if report.cancelled { ui.colored_label(theme::WARN, t("Cancelled — partial report; no readiness conclusion is available.")); }
                ui.label(t("This is SetupScope's inherited process environment. A terminal, IDE, activated virtual environment or Developer Command Prompt can have different settings. Restart SetupScope from the intended environment after changes."));
                ui.heading(t("Build prerequisite evidence"));
                for check in &report.checks {
                    ui.separator(); ui.label(RichText::new(t(&check.name)).strong()); status(ui, check.status);
                    for path in &check.evidence { ui.add(egui::Label::new(path).selectable(true)); }
                    ui.label(t(check.guidance));
                }
                ui.separator(); ui.heading(t("Read-only version compatibility"));
                ui.label(setupscope_core::i18n::t(&format!("{}: {} · {}: {} · {}: {}", t("Command"), report.version_command, t("Minimum (inclusive)"), report.version_range.minimum.map(|v| v.to_string()).unwrap_or_else(|| t("Open")), t("Maximum (exclusive)"), report.version_range.maximum_exclusive.map(|v| v.to_string()).unwrap_or_else(|| t("Open")))));
                ui.label(t("Each observed candidate is evaluated separately. There is no overall compatible verdict: a shell may select another candidate. Known proxies, launchers, SDK hosts, scripts, prerelease/special builds and unreadable metadata remain unknown. Unrecognized wrappers can report their own versions. This does not verify identity or evaluate project manifests or semantic-version ranges."));
                if report.versions.is_empty() { ui.label(t("No version candidate was observed for this command; compatibility is unknown. Enter another command name or inspect its PATH environment.")); }
                for candidate in &report.versions {
                    ui.separator(); ui.add(egui::Label::new(candidate.path.to_string_lossy()).selectable(true));
                    if let Some(evidence) = &candidate.evidence { ui.label(setupscope_core::i18n::t(&format!("{}: {}", t("Embedded product version"), evidence.version))); }
                    let color = match candidate.compatibility { setupscope_core::devversion::Compatibility::Matches => theme::OK, setupscope_core::devversion::Compatibility::Outside => theme::WARN, _ => theme::MUTED };
                    ui.colored_label(color, t(candidate.compatibility.label())); ui.label(t(&candidate.detail));
                }
                ui.separator(); ui.heading(t("PATH conflict inspector"));
                ui.label(t("Ordered PATH-only candidates: directory order first, then the displayed PATHEXT order. The first observed file is not a guarantee of what a shell runs: aliases, functions, current-directory lookup, application aliases, permissions and activation scripts can change resolution. Multiple locations do not prove different versions."));
                if report.path_partial { ui.colored_label(theme::WARN, t("Some PATH inputs were skipped or assumed. Treat candidate order as partial.")); }
                for tool in &report.tools {
                    let summary = format!("{} — {} {}", tool.name, tool.candidates.len(), t("candidate files"));
                    let header = super::widgets::collapsing_header(ui,summary).id_salt(("devcheck_tool", &tool.name)).default_open(tool.candidates.len() > 1 || !tool.notes.is_empty()).show(ui, |ui| {
                        if tool.candidates.is_empty() { ui.label(t("No regular candidate file observed in inspected PATH directories. This does not establish that the tool is absent from the PC.")); }
                        if tool.candidates.len() > 1 { ui.colored_label(theme::WARN, t("Multiple candidates: review ordering and intended environment; versions were not queried.")); }
                        for (i, path) in tool.candidates.iter().enumerate() { ui.add(egui::Label::new(setupscope_core::i18n::t(&format!("{}. {}", i + 1, path.display()))).selectable(true)); }
                        for note in &tool.notes { ui.label(setupscope_core::i18n::t(note)); }
                    });
                    a11y::explain(header.header_response, &t("Expand observed candidate paths and uncertainty notes. This never launches the command or opens a discovered file."));
                }
                let entries = super::widgets::collapsing_header(ui,t("PATH entries in inherited order")).show(ui, |ui| {
                    for entry in &report.entries {
                        ui.separator(); ui.add(egui::Label::new(setupscope_core::i18n::t(&format!("{}. {}", entry.order, entry.raw))).selectable(true)); status(ui, entry.status); ui.label(setupscope_core::i18n::t(&entry.detail));
                        if let Some(path) = &entry.path { ui.add(egui::Label::new(path.to_string_lossy()).selectable(true)); }
                    }
                });
                a11y::explain(entries.header_response, &t("Review duplicates, missing directories, relative paths and skipped entries. No edit or repair action is offered."));
                for note in &report.notes { ui.label(setupscope_core::i18n::t(note)); }
                ui.label(t("Limits: 256 PATH entries, 8 extensions, 64 candidates per command, 256 discovery operations and 64 entries per discovery directory. Network/device paths and observed links/reparse points are skipped. Concurrent filesystem changes can race metadata checks; this is not a security boundary."));
            });
        });
        if start {
            self.start_devcheck(ctx);
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, Vec2};

    fn frame(
        ctx: &egui::Context,
        state: &mut DevCheckState,
        events: Vec<Event>,
    ) -> (egui::FullOutput, bool) {
        let mut start = false;
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 400.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    start = controls(ui, state);
                });
            },
        );
        (output, start)
    }

    #[test]
    fn evidence_handoff_reuses_report_and_refuses_busy_replacement() {
        let ctx=egui::Context::default();
        let tooling=devcheck::inspect(&devcheck::Request{profile:Profile::Node,target:Target::Arm64,..Default::default()},&Default::default()).unwrap();
        let incoming=setupscope_core::projectdoctor::Report{
            root:"C:/Dev/Observed".into(),observed_at:"original observation".into(),target:Target::Arm64,
            findings:vec![],notes:vec![],files:vec![],tooling,partial:true,cancelled:false,
        };
        let mut state=DevCheckState::default();
        state.job=Some(Job::spawn(&ctx,|_|Err("test worker".into())));
        assert!(!state.accept_project_evidence(&incoming));
        assert!(state.report.is_none());assert_eq!(state.profile,Profile::RustMsvc);
        state.job=None;
        assert!(state.accept_project_evidence(&incoming));
        assert_eq!(state.profile,Profile::Node);assert_eq!(state.target,Target::Arm64);
        assert_eq!(state.report.as_ref().unwrap().checks.len(),incoming.tooling.checks.len());
        assert!(state.job.is_none(),"handoff must not start another scan");
    }
    #[test]
    fn controls_expose_accessible_guidance_without_starting_automatically() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = DevCheckState::default();
        assert!(!frame(&ctx, &mut state, vec![]).1);
        let (out, start) = frame(&ctx, &mut state, vec![]);
        assert!(!start);
        let update = out.platform_output.accesskit_update.unwrap();
        assert!(update.nodes.iter().any(|(_, n)| n
            .description()
            .is_some_and(|s| s.contains("command is never executed"))));
        assert!(update.nodes.iter().any(|(_, n)| n
            .description()
            .is_some_and(|s| s.contains("No commands, project scripts"))));
        assert!(
            update
                .nodes
                .iter()
                .filter(|(_, n)| !n.labelled_by().is_empty())
                .count()
                >= 5
        );
        assert!(state.job.is_none());
    }

    #[test]
    fn inspect_button_can_be_activated_by_keyboard() {
        let ctx = egui::Context::default();
        let mut state = DevCheckState::default();
        ctx.enable_accesskit();
        frame(&ctx, &mut state, vec![]);
        for _ in 0..6 {
            frame(
                &ctx,
                &mut state,
                vec![Event::Key {
                    key: Key::Tab,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Modifiers::NONE,
                }],
            );
        }
        let (_, start) = frame(
            &ctx,
            &mut state,
            vec![Event::Key {
                key: Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
        );
        assert!(start);
        assert!(
            state.job.is_none(),
            "the controls only request a scan; rendering never starts work itself"
        );
    }
}
