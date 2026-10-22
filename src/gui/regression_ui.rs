//! Guided evidence capture; vendor operations are always performed externally.
use super::{a11y, theme, Job};
use setupscope_core::{
    i18n::t,
    platform::Registry,
    regression::{self, CaptureRequest, Comparison, Outcome, Scope, Step, Workspace},
    settings::Settings,
};
use std::{path::Path, sync::Arc, time::Duration};

enum Update {
    Prepared(Scope),
    Listed(Vec<String>),
    Opened(Workspace),
    Compared(Comparison),
}
#[derive(Default)]
pub struct RegressionState {
    name: String,
    scope: Option<Scope>,
    reviewed: bool,
    ids: Vec<String>,
    selected: Option<String>,
    workspace: Option<Workspace>,
    step: Option<Step>,
    outcome: Outcome,
    exit_code: String,
    note: String,
    capture_reviewed: bool,
    from: usize,
    to: usize,
    comparison: Option<Comparison>,
    page: usize,
    single_row: bool,
    job: Option<Job<Result<Update, String>>>,
    error: Option<String>,
}
impl Drop for RegressionState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}
#[derive(Default)]
struct Actions {
    prepare: bool,
    create: bool,
    list: bool,
    open: bool,
    capture: bool,
    compare: bool,
}
fn scope_ui(ui: &mut egui::Ui, scope: &Scope) {
    ui.label(t(
        "Frozen scope (change Settings and prepare a new workspace to change it):",
    ));
    for root in &scope.file_roots {
        ui.add(
            egui::Label::new(format!(
                "{} [{}]",
                root.path,
                if root.recursive {
                    "recursive"
                } else {
                    "direct children"
                }
            ))
            .selectable(true),
        );
    }
    for root in &scope.registry_roots {
        ui.add(egui::Label::new(root).selectable(true));
    }
    let excluded = egui::CollapsingHeader::new(t("Exact exclusions"))
        .id_salt("regression_exclusions")
        .show(ui, |ui| {
            ui.label(t("File exclusions:"));
            for path in &scope.file_exclusions {
                ui.add(egui::Label::new(path).selectable(true));
            }
            ui.label(t("Registry exclusions:"));
            for path in &scope.registry_exclusions {
                ui.add(egui::Label::new(path).selectable(true));
            }
        });
    a11y::explain(excluded.header_response,&t("Review excluded locations and wildcard patterns. Excluded changes cannot appear in the report. The entire SetupScope data folder is excluded to prevent evidence capturing itself."));
}
fn controls(ui: &mut egui::Ui, state: &mut RegressionState) -> Actions {
    let mut action = Actions::default();
    ui.add_enabled_ui(state.job.is_none(),|ui|{
        ui.horizontal_wrapped(|ui|{
            let label=ui.label(t("New fixture name"));
            a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.name).char_limit(160).desired_width(220.0)).labelled_by(label.id),&t("Name a reproducible installer test, such as MyApp per-user upgrade. Creates separate evidence; never changes existing workspaces."));
            action.prepare=a11y::explain(ui.button(t("Prepare scope from Settings")),&t("Expand current snapshot roots and exclusions, validate data paths and show the exact scope before any evidence is saved. No scan or vendor operation starts.")).clicked();
        });
        if let Some(scope)=&state.scope {
            ui.push_id("prepared_scope",|ui|scope_ui(ui,scope));
            a11y::explain(ui.checkbox(&mut state.reviewed,t("I reviewed this scope and will use an appropriate isolated test environment")),&t("Required before creating this workspace. Capture records file metadata and registry values in the selected scope; evidence may contain sensitive local data. Run vendor operations externally after reviewing them."));
            action.create=a11y::explain(ui.add_enabled(state.reviewed&&!state.name.trim().is_empty(),egui::Button::new(t("Create local workspace"))),&t("Save a new versioned plan under SetupScopeData/regressions. Does not scan or run an installer. No existing evidence is replaced.")).clicked();
        }
        ui.separator();
        ui.horizontal_wrapped(|ui|{
            action.list=a11y::explain(ui.button(t("Refresh saved workspaces")),&t("List up to 256 local workspace IDs. This does not scan the system or run vendor operations.")).clicked();
            let response=egui::ComboBox::from_id_salt("regression_workspace").selected_text(state.selected.as_deref().unwrap_or("Choose saved workspace")).show_ui(ui,|ui|{
                for id in &state.ids{a11y::explain(ui.selectable_value(&mut state.selected,Some(id.clone()),id),&t("Choose an existing local evidence folder, then Open or reload workspace."));}
            }).response;
            a11y::explain(response,&t("Saved workspace IDs. Invalid or interrupted folders remain visible; opening them either recovers readable receipts or explains the failure."));
            action.open=a11y::explain(ui.add_enabled(state.selected.is_some(),egui::Button::new(t("Open or reload workspace"))),&t("Read the selected saved plan and attempts. Reload after another instance changes it or after an interrupted capture. Does not overwrite evidence.")).clicked();
        });
        if let Some(workspace)=&state.workspace {
            ui.separator(); ui.strong(format!("{} · revision {}",workspace.name,workspace.revision));
            ui.label(format!("{} · {} · {}",workspace.computer,workspace.user,workspace.created));
            ui.add(egui::Label::new(format!("Workspace ID: {}",workspace.id)).selectable(true));
            let scope=egui::CollapsingHeader::new(t("Review saved capture scope")).id_salt("regression_saved_scope").show(ui,|ui|scope_ui(ui,&workspace.scope));
            a11y::explain(scope.header_response,&t("This plan uses its saved expanded scope. Later Settings changes do not change this run. Excluded or inaccessible locations cannot establish absence of leftovers."));
            for (index,attempt) in workspace.attempts.iter().enumerate(){
                ui.separator();
                let status=if attempt.evidence{"Saved evidence"}else if attempt.finished.is_empty(){"Interrupted or pending — no committed evidence"}else{"Capture failed — no committed evidence"};
                ui.label(format!("{}. {} · {} · {}",index+1,attempt.step.label(),attempt.outcome.label(),status));
                ui.label(format!("{} → {} · warnings: {}",attempt.started,if attempt.finished.is_empty(){"unknown"}else{&attempt.finished},attempt.warning_count));
                if let Some(code)=attempt.reported_exit_code{ui.label(format!("User-reported exit code: {code}"));}
                if !attempt.note.is_empty(){ui.label(&attempt.note);}
                if !attempt.error.is_empty(){ui.colored_label(theme::WARN,&attempt.error);}
            }
            let steps=workspace.allowed_steps();
            if !steps.contains(&state.step.unwrap_or(Step::Before)){state.step=steps.first().copied();state.capture_reviewed=false;}
            if steps.is_empty(){ui.label(t("All lifecycle checkpoints are saved. Review comparisons; this is not a certification of vendor behavior or a clean machine."));}
            else {
                let previous_details=(state.outcome,state.exit_code.clone(),state.note.clone());
                ui.separator();
                ui.label(t("Perform the reviewed vendor operation externally before capturing its checkpoint. SetupScope never installs, upgrades, uninstalls or reboots for this workflow."));
                let previous=state.step;
                let response=egui::ComboBox::from_id_salt("regression_step").selected_text(state.step.map(Step::label).unwrap_or("Choose checkpoint")).show_ui(ui,|ui|{
                    for step in steps{a11y::explain(ui.selectable_value(&mut state.step,Some(step),t(step.label())),&t("Choose the next chronological checkpoint. Upgrade is optional after installation; uninstall ends this run."));}
                }).response;
                a11y::explain(response,&t("Capture Before first, then After install, optionally After upgrade, then After uninstall. Failed or interrupted captures can be retried without deleting earlier attempts."));
                if previous!=state.step{state.capture_reviewed=false;state.outcome=Outcome::Unknown;state.exit_code.clear();}
                if state.step!=Some(Step::Before) {
                    let response=egui::ComboBox::from_id_salt("regression_outcome").selected_text(state.outcome.label()).show_ui(ui,|ui|{
                        for outcome in Outcome::VENDOR{a11y::explain(ui.selectable_value(&mut state.outcome,outcome,t(outcome.label())),&t("Record what you observed. SetupScope did not run the vendor operation and cannot verify its result."));}
                    }).response;
                    a11y::explain(response,&t("User-reported outcome, not a verified vendor result. Leave Unknown when uncertain. Failure, cancellation and reboot-required remain distinct."));
                    let label=ui.label(t("Reported exit code (optional)"));
                    a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.exit_code).char_limit(12).desired_width(120.0)).labelled_by(label.id),&t("Optional signed 32-bit decimal exit code you observed externally. Never inferred from snapshot changes. Leave blank if unavailable."));
                }
                let label=ui.label(t("Operation / fixture note"));
                a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.note).char_limit(1000)).labelled_by(label.id),&t("Record package/version, reviewed external command and relevant reboot or failure observations. Stored locally with evidence; do not enter passwords or tokens."));
                if previous_details!=(state.outcome,state.exit_code.clone(),state.note.clone()){state.capture_reviewed=false;}
                a11y::explain(ui.checkbox(&mut state.capture_reviewed,t("I reviewed this checkpoint, scope and sensitive-data implications")),&t("Required for each capture. Saves selected registry values and file metadata locally. Use an appropriate isolated environment; no vendor command runs. Cancellation preserves a failed/interrupted receipt and earlier evidence."));
                let code_valid=state.step==Some(Step::Before)||state.exit_code.trim().is_empty()||state.exit_code.trim().parse::<i32>().is_ok();
                if !code_valid{ui.colored_label(theme::WARN,t("Enter a signed 32-bit decimal exit code or leave it blank."));}
                action.capture=a11y::explain(ui.add_enabled(state.capture_reviewed&&code_valid,egui::Button::new(t("Capture and save checkpoint"))),&t("Read the frozen scope and save evidence plus a receipt. Limits: 50,000 cooperative items including registry values; 64 MiB payload accounting; 8 MiB per native registry value buffer; 64 MiB compressed and 128 MiB expanded per snapshot; 32 attempts. These are not a process memory/time cap. Incomplete evidence cannot prove a clean system.")).clicked();
            }
            if workspace.attempts.iter().filter(|a|a.evidence).count()>=2 {
                ui.separator(); ui.label(t("Read-only comparison (choose Before → After uninstall for remaining deltas):"));
                for (salt,label,selected) in [("regression_from","Earlier checkpoint",&mut state.from),("regression_to","Later checkpoint",&mut state.to)]{
                    let response=egui::ComboBox::from_id_salt(salt).selected_text(workspace.attempts.get(*selected).map(|a|format!("{}. {}",*selected+1,a.step.label())).unwrap_or_else(||label.into())).show_ui(ui,|ui|{
                        for (i,a) in workspace.attempts.iter().enumerate().filter(|(_,a)|a.evidence){a11y::explain(ui.selectable_value(selected,i,format!("{}. {}",i+1,a.step.label())),&t("Select committed evidence for this comparison. Failed or interrupted attempts cannot be selected."));}
                    }).response;
                    a11y::explain(response,&t(label));
                }
                action.compare=a11y::explain(ui.add_enabled(state.from<state.to&&workspace.attempts.get(state.from).is_some_and(|a|a.evidence)&&workspace.attempts.get(state.to).is_some_and(|a|a.evidence),egui::Button::new(t("Compare saved evidence"))),&t("Read and validate the two saved snapshots and show differences. Does not delete, restore or apply anything. Background changes are not proven to come from the installer.")).clicked();
            }
        }
    });
    action
}
impl RegressionState {
    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        data: &Path,
        settings: &Settings,
        reg: Arc<dyn Registry>,
    ) {
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Err(error) => self.error = Some(error),
                Ok(Update::Prepared(scope)) => {
                    self.scope = Some(scope);
                    self.reviewed = false;
                    self.error = None;
                }
                Ok(Update::Listed(ids)) => {
                    self.ids = ids;
                    self.error = None;
                }
                Ok(Update::Opened(workspace)) => {
                    self.selected = Some(workspace.id.clone());
                    self.step = workspace.allowed_steps().first().copied();
                    let saved: Vec<usize> = workspace
                        .attempts
                        .iter()
                        .enumerate()
                        .filter(|(_, a)| a.evidence)
                        .map(|(i, _)| i)
                        .collect();
                    self.from = saved.first().copied().unwrap_or(0);
                    self.to = saved.last().copied().unwrap_or(0);
                    self.workspace = Some(workspace);
                    self.capture_reviewed = false;
                    self.reviewed = false;
                    self.outcome = Outcome::Unknown;
                    self.exit_code.clear();
                    self.note.clear();
                    self.comparison = None;
                    self.error = None;
                }
                Ok(Update::Compared(comparison)) => {
                    self.comparison = Some(comparison);
                    self.page = 0;
                    self.error = None;
                }
            }
        }
        ui.label(t("Installer regression workspace — explicit local evidence, no vendor execution. Use an appropriate isolated Windows environment. Snapshots contain registry values and local paths; they are not redacted diagnostic bundles."));
        let action = controls(ui, self);
        let data = data.to_owned();
        if action.prepare {
            let settings = settings.clone();
            self.spawn(ui, move |_| {
                Scope::from_settings(&settings, &data).map(Update::Prepared)
            });
        } else if action.create {
            if let Some(scope) = self.scope.clone() {
                let name = self.name.clone();
                self.spawn(ui, move |_| {
                    regression::create(&data, &name, scope).map(Update::Opened)
                });
            }
        } else if action.list {
            self.spawn(ui, move |_| regression::list(&data).map(Update::Listed));
        } else if action.open {
            if let Some(id) = self.selected.clone() {
                self.spawn(ui, move |_| {
                    regression::load(&data, &id).map(Update::Opened)
                });
            }
        } else if action.capture {
            if let (Some(workspace), Some(step)) = (&self.workspace, self.step) {
                let id = workspace.id.clone();
                let request = CaptureRequest {
                    revision: workspace.revision,
                    step,
                    outcome: if step == Step::Before {
                        Outcome::NotApplicable
                    } else {
                        self.outcome
                    },
                    reported_exit_code: if step == Step::Before {
                        None
                    } else {
                        self.exit_code.trim().parse().ok()
                    },
                    note: self.note.clone(),
                };
                self.capture_reviewed = false;
                self.spawn(ui, move |progress| {
                    regression::capture(&data, &id, request, reg.as_ref(), progress)
                        .map(Update::Opened)
                });
            }
        } else if action.compare {
            if let Some(workspace) = &self.workspace {
                let (id, revision, from, to) =
                    (workspace.id.clone(), workspace.revision, self.from, self.to);
                self.spawn(ui, move |_| {
                    regression::compare(&data, &id, revision, from, to).map(Update::Compared)
                });
            }
        }
        if let Some(job) = &self.job {
            ui.spinner();
            ui.label(format!(
                "{} · {} items",
                job.progress.phase(),
                job.progress.count()
            ));
            if a11y::explain(ui.button(t("Cancel workspace operation")),&t("Capture stops cooperatively at read checkpoints; publication finishes safely. Short load/list/compare operations may finish before cancellation. Earlier evidence is preserved. Reload after any uncertain save result.")).clicked(){job.cancel();}
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::WARN, error);
            ui.label(t("Previous evidence is retained. Reload the workspace to inspect pending or completed receipts before retrying. Do not delete recovery files."));
        }
        if let Some(comparison) = &self.comparison {
            ui.separator();
            ui.label(t("Comparison results refer to the captured workspace revision, not current machine state. A zero delta is not proof of complete removal: exclusions, metadata-only file checks and unrelated activity limit attribution."));
            if a11y::explain(ui.checkbox(&mut self.single_row,t("One difference per page (for long paths)")),&t("Reduce each report page to one difference to review long file paths or registry names. Otherwise pages contain up to 50 rows. Each page remains bounded to 128 KiB and explicitly marks truncation.")).changed(){self.page=0;}
            let page_size = if self.single_row { 1 } else { 50 };
            let total = comparison.changes.total();
            let pages = total.max(1).div_ceil(page_size);
            self.page = self.page.min(pages - 1);
            ui.horizontal(|ui|{
                if a11y::explain(ui.add_enabled(self.page>0,egui::Button::new(t("Previous results page"))),&t("Review the preceding 50 differences without changing evidence.")).clicked(){self.page-=1;}
                ui.label(format!("{} / {}",self.page+1,pages));
                if a11y::explain(ui.add_enabled(self.page+1<pages,egui::Button::new(t("Next results page"))),&t("Review the next 50 differences. All rows are available through pages; raw registry contents are kept only in the saved snapshots.")).clicked(){self.page+=1;}
            });
            let report = report_page(comparison, self.page, page_size);
            if a11y::explain(ui.button(t("Copy this report page (contains local paths)")),&t("Explicitly copy this page's summary and up to 50 differences, bounded to 128 KiB, to the clipboard. Includes local paths and registry names; not redacted. No upload or file export occurs.")).clicked(){ui.ctx().copy_text(report.clone());}
            ui.add(egui::Label::new(report).selectable(true));
        }
    }
    fn spawn(
        &mut self,
        ui: &egui::Ui,
        operation: impl FnOnce(&setupscope_core::snapshot::Progress) -> anyhow::Result<Update>
            + Send
            + 'static,
    ) {
        self.error = None;
        self.job = Some(Job::spawn(ui.ctx(), move |progress| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(||operation(progress)))
                .map_err(|_|"Workspace operation failed unexpectedly; reload to inspect retained receipts before retrying".into())
                .and_then(|result|result.map_err(|e|format!("{e:#}")))
        }));
    }
}
fn report_page(comparison: &Comparison, page: usize, page_size: usize) -> String {
    let changes = &comparison.changes;
    let stats = changes.stats();
    let mut output=format!("{} → {}\nObservation: {}\n{} differences. Files +{} / -{} / ~{}; directories +{}; registry keys +{} / -{}; values +{} / -{} / ~{}.\nAdded size (saturating ceiling): {} bytes\nMetadata-only files; no content hashes. Outcomes are user-reported. Not installer attribution or clean-machine proof.\n",comparison.from.label(),comparison.to.label(),if comparison.complete_observation{"No capture warnings or uncertain reported outcomes in this interval"}else{"Incomplete / warnings / uncertain reported outcomes"},changes.total(),stats.files_added,stats.files_removed,stats.files_modified,stats.dirs_added,stats.keys_added,stats.keys_removed,stats.values_added,stats.values_removed,stats.values_modified,stats.bytes_added);
    for warning in comparison.warnings.iter().take(10) {
        append_bounded(&mut output, &format!("Warning: {warning}\n"));
    }
    if comparison.warnings.len() > 10 {
        append_bounded(
            &mut output,
            &format!(
                "{} further capture warnings retained in snapshots.\n",
                comparison.warnings.len() - 10
            ),
        );
    }
    let page_size = page_size.clamp(1, 50);
    let start = page.saturating_mul(page_size);
    let mut index = 0;
    let mut row = |kind: &str, path: &str, name: Option<&str>| {
        if (start..start.saturating_add(page_size)).contains(&index) {
            append_bounded(
                &mut output,
                &format!(
                    "{}: {}{}\n",
                    kind,
                    path,
                    name.map(|n| format!(" [{n}]")).unwrap_or_default()
                ),
            );
        }
        index += 1;
    };
    for f in &changes.files_added {
        row("Added file/directory", &f.path, None);
    }
    for f in &changes.files_removed {
        row("Removed file/directory", &f.path, None);
    }
    for f in &changes.files_modified {
        row("Modified file metadata", &f.after.path, None);
    }
    for k in &changes.keys_added {
        row("Added key (including its values)", &k.path, None);
    }
    for k in &changes.keys_removed {
        row("Removed key (including its values)", &k.path, None);
    }
    for v in &changes.values_added {
        row("Added value", &v.key, Some(&v.value.name));
    }
    for v in &changes.values_removed {
        row("Removed value", &v.key, Some(&v.value.name));
    }
    for v in &changes.values_modified {
        row("Modified value", &v.key, Some(&v.after.name));
    }
    output
}
fn append_bounded(output: &mut String, text: &str) {
    const LIMIT: usize = 128 * 1024;
    const NOTICE: &str = "\n[Report page truncated at 128 KiB; original evidence is retained.]\n";
    if output.ends_with(NOTICE) {
        return;
    }
    let room = LIMIT.saturating_sub(output.len() + NOTICE.len());
    if text.len() <= room {
        output.push_str(text);
    } else {
        let mut end = room.min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        output.push_str(&text[..end]);
        output.push_str(NOTICE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, Vec2};
    fn frame(
        ctx: &egui::Context,
        state: &mut RegressionState,
        events: Vec<Event>,
    ) -> (egui::FullOutput, Actions) {
        let mut actions = Actions::default();
        let out = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 900.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    actions = controls(ui, state);
                });
            },
        );
        (out, actions)
    }
    fn scope() -> Scope {
        Scope {
            registry_roots: vec![],
            file_roots: vec![],
            registry_exclusions: vec![],
            file_exclusions: vec![],
            excluded_data: "C:\\fixture-data".into(),
        }
    }
    #[test]
    fn review_gate_is_explicit_and_accessible_without_starting_work() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = RegressionState::default();
        state.name = "fixture".into();
        state.scope = Some(scope());
        frame(&ctx, &mut state, vec![]);
        let (output, actions) = frame(&ctx, &mut state, vec![]);
        assert!(!actions.create && !actions.capture);
        assert!(state.job.is_none());
        let nodes = output.platform_output.accesskit_update.unwrap();
        assert!(nodes.nodes.iter().any(|(_, node)| node
            .description()
            .is_some_and(|d| d.contains("Required before creating"))));
        assert!(nodes.nodes.iter().any(|(_, node)| node
            .description()
            .is_some_and(|d| d.contains("No existing evidence is replaced"))));
    }
    #[test]
    fn keyboard_can_prepare_scope_without_launching_operation_from_controls() {
        let ctx = egui::Context::default();
        let mut state = RegressionState::default();
        let key = |key| Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        frame(&ctx, &mut state, vec![]);
        frame(&ctx, &mut state, vec![key(Key::Tab)]);
        frame(&ctx, &mut state, vec![key(Key::Tab)]);
        assert!(frame(&ctx, &mut state, vec![key(Key::Enter)]).1.prepare);
        assert!(state.job.is_none());
        assert!(state.workspace.is_none());
    }
    #[test]
    fn paged_report_covers_rows_and_bounds_hostile_unicode_paths() {
        use setupscope_core::model::{ChangeSet, FileEntry};
        let changes = ChangeSet {
            files_added: (0..51)
                .map(|i| FileEntry {
                    path: format!("row-{i:03}"),
                    is_dir: false,
                    size: 1,
                    mtime: 0,
                })
                .collect(),
            ..Default::default()
        };
        let comparison = Comparison {
            changes,
            warnings: vec![],
            complete_observation: true,
            from: Step::Before,
            to: Step::Uninstalled,
        };
        let first = report_page(&comparison, 0, 50);
        let second = report_page(&comparison, 1, 50);
        assert!(first.contains("row-049"));
        assert!(!first.contains("row-050"));
        assert!(second.contains("row-050"));
        assert!(!second.contains("row-000"));
        assert!(report_page(&comparison, 50, 1).contains("row-050"));
        let mut output = String::new();
        append_bounded(&mut output, &"🦀".repeat(100_000));
        assert!(output.len() <= 128 * 1024);
        assert!(output.contains("truncated"));
    }
    #[test]
    fn checkpoint_capture_requires_review_and_valid_reported_exit_code() {
        for (reviewed, code, disabled) in [
            (false, "", true),
            (true, "bad", true),
            (true, "3010", false),
        ] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut state = RegressionState::default();
            state.workspace = Some(Workspace {
                version: 1,
                id: "fixture".into(),
                name: "fixture".into(),
                computer: "fixture".into(),
                user: "fixture".into(),
                created: "fixture".into(),
                identity_marker: "0".repeat(64),
                scope: scope(),
                revision: 2,
                attempts: vec![regression::Attempt {
                    id: "baseline".into(),
                    step: Step::Before,
                    started: "fixture".into(),
                    finished: "fixture".into(),
                    outcome: Outcome::NotApplicable,
                    reported_exit_code: None,
                    note: String::new(),
                    evidence: true,
                    warning_count: 0,
                    error: String::new(),
                }],
            });
            state.step = Some(Step::Installed);
            state.capture_reviewed = reviewed;
            state.exit_code = code.into();
            frame(&ctx, &mut state, vec![]);
            let (output, actions) = frame(&ctx, &mut state, vec![]);
            assert!(!actions.capture);
            assert!(state.job.is_none());
            let update = output.platform_output.accesskit_update.unwrap();
            let button = update
                .nodes
                .iter()
                .find(|(_, node)| {
                    node.role() == egui::accesskit::Role::Button
                        && node
                            .description()
                            .is_some_and(|d| d.contains("Read the frozen scope"))
                })
                .unwrap();
            assert_eq!(button.1.is_disabled(), disabled);
        }
    }
}
