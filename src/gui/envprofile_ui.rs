//! Explicit profile authoring, child launch and reviewed user-only transactions.
use super::{
    a11y, theme,
    widgets::{pick_file, pick_folder, save_file},
    Job,
};
use setupscope_core::{
    envprofile::{self, Change, ProcessPlan, Profile, State, Transaction, Variable},
    i18n::t,
};
use std::{
    path::{Path, PathBuf},
    process::Child,
    time::Duration,
};

#[derive(Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    value: String,
    remove: bool,
}
enum Update {
    Imported(Profile),
    Process(ProcessPlan),
    Prepared(Transaction),
    Receipt(Transaction),
    Listed(Vec<String>),
    Launched(Child),
    Saved,
}
pub struct EnvironmentState {
    name: String,
    project: String,
    paths: String,
    variables: Vec<Entry>,
    process: Option<ProcessPlan>,
    executable: String,
    arguments: String,
    new_console: bool,
    launch_reviewed: bool,
    child: Option<Child>,
    child_status: String,
    user_plan: Option<Transaction>,
    apply_reviewed: bool,
    receipt: Option<Transaction>,
    undo_reviewed: bool,
    open_advanced: Option<bool>,
    ids: Vec<String>,
    selected: Option<String>,
    job: Option<Job<Result<Update, String>>>,
    message: String,
    error: Option<String>,
}
impl Default for EnvironmentState {
    fn default() -> Self {
        Self {
            name: "Development profile".into(),
            project: String::new(),
            paths: String::new(),
            variables: vec![],
            process: None,
            executable: String::new(),
            arguments: String::new(),
            new_console: true,
            launch_reviewed: false,
            child: None,
            child_status: String::new(),
            user_plan: None,
            apply_reviewed: false,
            receipt: None,
            undo_reviewed: false,
            open_advanced: None,
            ids: vec![],
            selected: None,
            job: None,
            message: String::new(),
            error: None,
        }
    }
}
impl EnvironmentState {
    fn profile(&self) -> anyhow::Result<Profile> {
        let profile = Profile {
            version: 1,
            name: self.name.trim().into(),
            project: self.project.trim().into(),
            path_prepend: self
                .paths
                .lines()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect(),
            variables: self
                .variables
                .iter()
                .map(|e| Variable {
                    name: e.name.clone(),
                    value: if e.remove {
                        None
                    } else {
                        Some(e.value.trim().into())
                    },
                })
                .collect(),
        };
        profile.validate()?;
        Ok(profile)
    }
    fn invalidate(&mut self) {
        self.process = None;
        self.user_plan = None;
        self.apply_reviewed = false;
        self.launch_reviewed = false;
    }
    fn load_profile(&mut self, profile: Profile) {
        self.name = profile.name;
        self.project = profile.project;
        self.paths = profile.path_prepend.join("\n");
        self.variables = profile
            .variables
            .into_iter()
            .map(|v| Entry {
                name: v.name,
                value: v.value.clone().unwrap_or_default(),
                remove: v.value.is_none(),
            })
            .collect();
        self.invalidate();
    }
    fn spawn(
        &mut self,
        ui: &egui::Ui,
        operation: impl FnOnce() -> anyhow::Result<Update> + Send + 'static,
    ) {
        self.error = None;
        self.message.clear();
        self.job = Some(Job::spawn(ui.ctx(), move |_| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(operation)).map_err(|_|"Environment operation failed unexpectedly. Preserve local receipts and reload before recovery.".into()).and_then(|r|r.map_err(|e|format!("{e:#}")))
        }));
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, data: &Path) {
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Err(error) => self.error = Some(error),
                Ok(Update::Imported(profile)) => {
                    self.load_profile(profile);
                    self.message = "Profile imported for review; nothing applied or launched.".into();
                }
                Ok(Update::Process(plan)) => {
                    self.process = Some(plan);
                    self.launch_reviewed = false;
                }
                Ok(Update::Prepared(plan)) => {
                    self.user_plan = Some(plan);
                    self.apply_reviewed = false;
                    self.open_advanced = Some(true);
                }
                Ok(Update::Receipt(receipt)) => {
                    self.selected = Some(receipt.id.clone());
                    self.receipt = Some(receipt);
                    self.undo_reviewed = false;
                    self.open_advanced = Some(true);
                }
                Ok(Update::Listed(ids)) => self.ids = ids,
                Ok(Update::Saved) => self.message = "Profile saved to a new file. Only explicitly entered overrides were exported.".into(),
                Ok(Update::Launched(child)) => {
                    self.child_status=format!("Started process {}. Close it normally when finished; this is not a sandbox.",child.id());
                    self.child = Some(child);
                    self.launch_reviewed = false;
                }
            }
        }
        if let Some(child) = &mut self.child {
            match child.try_wait() {
                Ok(Some(status)) => {
                    self.child_status = format!("Launched process exited: {status}");
                    self.child = None;
                }
                Err(error) => {
                    self.child_status =
                        format!("Child status unavailable: {error}. No termination requested.");
                    self.child = None;
                }
                Ok(None) => {
                    ui.ctx().request_repaint_after(Duration::from_millis(250));
                }
            }
        }
        super::widgets::scope_note(ui, "Preview project environments first. Launch is not a sandbox; persistent edits require separate review.", "Project environment profiles — process-local by default. Author explicit developer paths; imported files never apply or launch automatically. Profiles do not export the inherited environment or read .env files. Do not enter credentials. Changing PATH, PYTHONPATH or SDK directories can cause the selected program to load code from those locations. Launch uses SetupScope's privileges, is not a security sandbox, and has no elevation fallback.");
        
        let action = controls(ui, self);
        if action.browse {
            if let Some(path) = pick_folder(&t("Choose project working directory")) {
                self.project = path.display().to_string();
                self.invalidate();
            }
        } else if action.executable {
            if let Some(path) = pick_file(
                &t("Choose an executable to launch explicitly"),
                "Windows executable",
                &["exe"],
            ) {
                self.executable = path.display().to_string();
                self.launch_reviewed = false;
            }
        } else if action.import {
            if let Some(path) = pick_file(
                &t("Import developer path profile (review only)"),
                "JSON profile",
                &["json"],
            ) {
                self.spawn(ui, move || envprofile::import(&path).map(Update::Imported));
            }
        } else if action.export {
            if let Ok(profile) = self.profile() {
                if let Some(path) = save_file(
                    &t("Save a new environment profile (contains local paths)"),
                    "developer-environment.json",
                    "JSON profile",
                    "json",
                ) {
                    self.spawn(ui, move || {
                        envprofile::export(&path, &profile)?;
                        Ok(Update::Saved)
                    });
                }
            }
        } else if action.preview {
            if let Ok(profile) = self.profile() {
                self.spawn(ui, move || {
                    envprofile::preview_process(&profile).map(Update::Process)
                });
            }
        } else if action.launch {
            if let Some(plan) = self.process.clone() {
                let exe = PathBuf::from(self.executable.trim());
                let args = self.arguments.clone();
                let new_console = self.new_console;
                self.launch_reviewed = false;
                self.spawn(ui, move || {
                    envprofile::launch(&plan, &exe, &args, new_console).map(Update::Launched)
                });
            }
        } else if action.user_preview {
            if let Ok(profile) = self.profile() {
                self.spawn(ui, move || prepare_user(profile));
            }
        } else if action.apply {
            if let Some(plan) = self.user_plan.take() {
                let data = data.to_owned();
                self.selected = Some(plan.id.clone());
                self.apply_reviewed = false;
                self.spawn(ui, move || apply_user(data, plan));
            }
        } else if action.list {
            let data = data.to_owned();
            self.spawn(ui, move || {
                envprofile::list_transactions(&data).map(Update::Listed)
            });
        } else if action.open {
            if let Some(id) = self.selected.clone() {
                let data = data.to_owned();
                self.spawn(ui, move || {
                    envprofile::load_transaction(&data, &id).map(Update::Receipt)
                });
            }
        } else if action.undo {
            if let Some(receipt) = self.receipt.clone() {
                let data = data.to_owned();
                self.undo_reviewed = false;
                self.spawn(ui, move || undo_user(data, receipt));
            }
        }
        if self.job.is_some() {
            ui.spinner();
            ui.label(t("Working locally. Persistent apply/undo finishes its journaled operation; there is no halfway cancellation. If interrupted, reload the retained receipt before recovery."));
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        }
        if !self.message.is_empty() {
            ui.label(t(&self.message));
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::WARN, error);
            ui.label(t("For an uncertain persistent save, Refresh receipts and open the transaction before retrying or undoing. Earlier backups are retained; do not delete pending receipts."));
        }
        if !self.child_status.is_empty() {
            ui.label(t(&self.child_status));
        }
    }
}
fn prepare_user(profile: Profile) -> anyhow::Result<Update> {
    #[cfg(windows)]
    {
        envprofile::prepare_user(&profile, &envprofile::UserEnvironment).map(Update::Prepared)
    }
    #[cfg(not(windows))]
    {
        let _ = profile;
        anyhow::bail!("User environment editing requires Windows")
    }
}
fn apply_user(data: PathBuf, plan: Transaction) -> anyhow::Result<Update> {
    #[cfg(windows)]
    {
        envprofile::apply_user(&data, &plan, &envprofile::UserEnvironment).map(Update::Receipt)
    }
    #[cfg(not(windows))]
    {
        let _ = (data, plan);
        anyhow::bail!("User environment editing requires Windows")
    }
}
fn undo_user(data: PathBuf, receipt: Transaction) -> anyhow::Result<Update> {
    #[cfg(windows)]
    {
        envprofile::undo_user(&data, &receipt, &envprofile::UserEnvironment).map(Update::Receipt)
    }
    #[cfg(not(windows))]
    {
        let _ = (data, receipt);
        anyhow::bail!("User environment editing requires Windows")
    }
}
fn changes_ui(ui: &mut egui::Ui, changes: &[Change], salt: &str) {
    egui::ScrollArea::vertical().id_salt(salt).max_height(240.0).show(ui,|ui|{
        if changes.is_empty(){ui.label(t("No selected values change. Unrelated inherited variables are not displayed or exported."));}
        for change in changes{ui.separator();ui.strong(&change.name);ui.label(setupscope_core::i18n::t(&format!("Before (type {}): {}",change.before.as_ref().map(|v|v.kind.to_string()).unwrap_or_else(||"absent".into()),envprofile::display_value(&change.before))));ui.label(setupscope_core::i18n::t(&format!("After (type {}): {}",change.after.as_ref().map(|v|v.kind.to_string()).unwrap_or_else(||"absent".into()),envprofile::display_value(&change.after))));}
    });
}
#[derive(Default)]
struct Actions {
    browse: bool,
    executable: bool,
    import: bool,
    export: bool,
    preview: bool,
    launch: bool,
    user_preview: bool,
    apply: bool,
    list: bool,
    open: bool,
    undo: bool,
}
fn controls(ui: &mut egui::Ui, state: &mut EnvironmentState) -> Actions {
    let mut action = Actions::default();
    ui.add_enabled_ui(state.job.is_none(),|ui|{
        let before=(state.name.clone(),state.project.clone(),state.paths.clone(),state.variables.clone());
        let label=ui.label(t("Profile name"));a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.name).char_limit(160)).labelled_by(label.id),&t("A local label for this project's developer paths. Profiles have a versioned JSON format and require explicit import/export."));
        ui.horizontal_wrapped(|ui|{let label=ui.label(t("Project working directory"));a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.project).char_limit(4096)).labelled_by(label.id),&t("Absolute Windows directory used only when launching a selected executable. No project files or activation scripts are read automatically."));action.browse=a11y::explain(ui.button(t("Choose project directory…")),&t("Choose the working folder. Choosing never launches a process or changes an environment.")).clicked();});
        let label=ui.label(t("PATH directories to prepend (one absolute path per line)"));a11y::explain(ui.add(egui::TextEdit::multiline(&mut state.paths).desired_width(ui.available_width()).desired_rows(3).char_limit(64*4096)).labelled_by(label.id),&t("Up to 64 explicit absolute directories. Existing PATH suffix is preserved; an already matching prefix is not duplicated. No variable expansion, URLs, quotes or empty path components are accepted in new entries."));
        let mut removed=None;
        for (i,entry) in state.variables.iter_mut().enumerate(){ui.push_id(("environment_variable",i),|ui|{
            ui.horizontal_wrapped(|ui|{
                let combo=egui::ComboBox::from_id_salt("name").selected_text(&entry.name).show_ui(ui,|ui|{for key in envprofile::VARIABLES{a11y::explain(ui.selectable_value(&mut entry.name,(*key).into(),*key),&t("A supported developer path variable. Secret-bearing and arbitrary variable names are intentionally excluded."));}}).response;
                a11y::explain(combo,&t("Choose a supported developer path variable. VIRTUAL_ENV alone does not activate a virtual environment or select its Python executable."));
                a11y::explain(ui.checkbox(&mut entry.remove,t("Remove variable")),&t("Explicitly omit this selected variable from the child environment, or delete its user-level value after separate persistent review. Machine-level values are never changed."));
                if a11y::explain(ui.button(t("Remove row")),&t("Remove this override from the profile editor. Does not change the system.")).clicked(){removed=Some(i);}
            });
            if !entry.remove{let label=ui.label(t("Explicit developer path value"));a11y::explain(ui.add(egui::TextEdit::singleline(&mut entry.value).char_limit(16000)).labelled_by(label.id),&t("Enter an absolute path. PYTHONPATH, CMAKE_PREFIX_PATH and GOPATH may contain semicolon-separated paths. No credentials, URLs or percent-variable expansion are accepted."));}
        });}
        if let Some(i)=removed{state.variables.remove(i);}
        if a11y::explain(ui.add_enabled(state.variables.len()<16,egui::Button::new(t("Add developer variable"))),&t("Add an explicit path override. Up to 16 rows; duplicate names are rejected. Does not read inherited variables.")).clicked(){state.variables.push(Entry{name:"CARGO_HOME".into(),value:String::new(),remove:false});}
        if before!=(state.name.clone(),state.project.clone(),state.paths.clone(),state.variables.clone()){state.invalidate();}
        let valid=match state.profile(){Ok(_)=>true,Err(error)=>{ui.colored_label(theme::MUTED,setupscope_core::i18n::t(&format!("Profile needs attention: {error}")));false}};
        ui.horizontal_wrapped(|ui|{
            action.import=a11y::explain(ui.button(t("Import profile for review…")),&t("Read a bounded JSON profile, validate only known developer path overrides and populate this editor. Never applies or launches. Arbitrary .env files are not imported.")).clicked();
            action.export=a11y::explain(ui.add_enabled(valid,egui::Button::new(t("Export new profile…"))),&t("Save only explicitly entered overrides and the project path to a new JSON file. Contains local paths; not a secret detector or redacted bundle. Existing files are never overwritten.")).clicked();
            action.preview=a11y::explain(super::widgets::action_button(ui,valid,&t("Preview process-local environment")),&t("Compare selected overrides with SetupScope's inherited environment. Unrelated values are not captured or exported. No command or environment change occurs.")).clicked();
        });
        if let Some(plan)=&state.process{
            ui.separator();ui.strong(t("Default: launch a child with this environment only"));changes_ui(ui,&plan.changes,"environment_process_diff");
            ui.label(t("The inherited environment can differ from a terminal or IDE. SDK directories do not prove installed/working tools. This does not run activation scripts or set up compiler workloads."));
            let previous=(state.executable.clone(),state.arguments.clone(),state.new_console);
            ui.horizontal_wrapped(|ui|{let label=ui.label(t("Exact executable (.exe)"));a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.executable).char_limit(32768)).labelled_by(label.id),&t("Absolute executable path, passed directly without PATH search or a shell. Scripts require selecting their interpreter explicitly. No elevation fallback occurs."));action.executable=a11y::explain(ui.button(t("Choose executable…")),&t("Choose a Windows executable. Choosing alone never runs it.")).clicked();});
            let label=ui.label(t("Arguments (reviewed Windows command-line syntax)"));a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.arguments).char_limit(8192)).labelled_by(label.id),&t("Exact raw arguments passed to the selected executable. Its own parser applies. Selecting cmd.exe or another interpreter can execute commands; review the complete arguments yourself."));
            a11y::explain(ui.checkbox(&mut state.new_console,t("Open a console for command-line programs")),&t("Start with a new console when checked. When unchecked, console creation is suppressed; output is not captured and interactive console input is unavailable. GUI executables manage their own windows."));
            if previous!=(state.executable.clone(),state.arguments.clone(),state.new_console){state.launch_reviewed=false;}
            ui.add(egui::Label::new(setupscope_core::i18n::t(&format!("Launch: {} {}\nWorking directory: {}",state.executable,state.arguments,plan.profile.project))).selectable(true));
            a11y::explain(ui.checkbox(&mut state.launch_reviewed,t("I reviewed this executable, arguments, directory, environment and privileges")),&t("Required before launching. The child may execute project code with SetupScope's permissions. This is not a sandbox. Existing processes and saved Windows environment are unchanged. Closing SetupScope does not terminate the child."));
            action.launch=a11y::explain(ui.add_enabled(state.launch_reviewed&&!state.executable.trim().is_empty()&&state.child.is_none(),egui::Button::new(t("Launch with process-local profile"))),&t("Launch the exact executable once with selected overrides and the remaining inherited environment. No global SetEnvironmentVariable call or persistent write occurs. Tracks direct child exit only; no descendant termination or output capture.")).clicked();
        }
        let advanced=super::widgets::collapsing_header(ui,t("Persistent Current-User Changes and Recovery")).id_salt("environment_persistent").open(state.open_advanced.take()).show(ui,|ui|{
            ui.label(t("Only HKCU Environment is edited. Machine values are never changed. User PATH is separate from the combined process PATH; prepending user PATH may not precede machine PATH. Existing apps may retain their old environment."));
            action.user_preview=a11y::explain(ui.add_enabled(valid,egui::Button::new(t("Preview saved user-level changes"))),&t("Read only the selected current-user values and prepare exact before/after bytes and types in memory. Does not save or apply. Unreadable or unsupported values block preview.")).clicked();
            if let Some(plan)=&state.user_plan{
                ui.label(setupscope_core::i18n::t(&format!("Prepared transaction: {}",plan.id)));changes_ui(ui,&plan.changes,"environment_user_preview");
                a11y::explain(ui.checkbox(&mut state.apply_reviewed,t("I reviewed these persistent user changes and the local prior-state backup")),&t("Required before applying. Exact selected prior values, types and absence are journaled under SetupScopeData/environment-profiles. These private receipts can contain sensitive old values. Edits affect future processes and are not an atomic multi-variable transaction."));
                action.apply=a11y::explain(ui.add_enabled(state.apply_reviewed,egui::Button::new(t("Apply reviewed current-user changes"))),&t("Persist this exact preview to HKCU only, after writing a recovery receipt. Reject stale values, serialize SetupScope writers and verify each write. Other applications can still race the registry; on interruption inspect the retained receipt and review undo.")).clicked();
            }
            ui.separator();
            ui.horizontal_wrapped(|ui|{
                action.list=a11y::explain(ui.button(t("Refresh environment receipts")),&t("List up to 256 local transaction folders, including interrupted operations. Does not apply or undo anything.")).clicked();
                let combo=egui::ComboBox::from_id_salt("environment_receipt").selected_text(state.selected.as_deref().unwrap_or("Choose receipt")).show_ui(ui,|ui|{for id in &state.ids{a11y::explain(ui.selectable_value(&mut state.selected,Some(id.clone()),id),&t("Choose the receipt to open and review. Selection never changes Windows."));}}).response;
                a11y::explain(combo,&t("Saved current-user transaction IDs. Open a receipt to review its exact original and intended values before undoing."));
                action.open=a11y::explain(ui.add_enabled(state.selected.is_some(),egui::Button::new(t("Open environment receipt"))),&t("Read saved values and operation state. Pending Applying or Undoing can mean an interruption; review before recovery. No automatic mutation occurs.")).clicked();
            });
            if let Some(receipt)=&state.receipt{
                ui.label(setupscope_core::i18n::t(&format!("Loaded receipt: {} · {} · {:?}",receipt.id,receipt.profile_name,receipt.state)));changes_ui(ui,&receipt.changes,"environment_receipt_values");
                if !receipt.notice.is_empty(){ui.label(&receipt.notice);}
                if receipt.state!=State::Undone{
                    a11y::explain(ui.checkbox(&mut state.undo_reviewed,t("I reviewed this receipt and want its exact prior user values restored")),&t("Required for undo/recovery. Each current value must match its saved before or after state. Any third-party value blocks undo before restoration starts; no machine values are touched."));
                    action.undo=a11y::explain(ui.add_enabled(state.undo_reviewed,egui::Button::new(t("Restore reviewed prior user values"))),&t("Journal Undoing, restore exact prior bytes/types/absence and verify. If interrupted, retain and reopen the receipt. Changes since this review or conflicting current values are rejected; third-party registry writers cannot be locked out atomically.")).clicked();
                }
            }
        });
        a11y::explain(advanced.header_response,&t("Optional persistent edits, separate from the default child-only environment. Every apply and undo requires a fresh explicit review and retains a local recovery receipt."));
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, Vec2};
    fn fixture() -> EnvironmentState {
        EnvironmentState {
            project: "C:\\fixture".into(),
            paths: "C:\\SDK\\bin".into(),
            executable: "C:\\Tool.exe".into(),
            ..Default::default()
        }
    }
    fn frame(
        ctx: &egui::Context,
        state: &mut EnvironmentState,
        events: Vec<Event>,
        advanced: bool,
    ) -> (egui::FullOutput, Actions) {
        let mut actions = Actions::default();
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1400.0, 3000.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if advanced {
                        state.open_advanced = Some(true);
                    }
                    actions = controls(ui, state);
                });
            },
        );
        (output, actions)
    }
    fn transaction() -> Transaction {
        Transaction {
            version: 1,
            id: "fixture".into(),
            identity: "0".repeat(64),
            created: "fixture".into(),
            profile_name: "fixture".into(),
            changes: vec![Change {
                name: "CARGO_HOME".into(),
                before: None,
                after: Some(setupscope_core::model::RegValue::string(
                    "CARGO_HOME",
                    "C:\\SDK",
                )),
            }],
            state: State::Prepared,
            notice: String::new(),
        }
    }
    #[test]
    fn launch_and_persistent_changes_require_separate_accessible_review() {
        for reviewed in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut state = fixture();
            state.process = Some(ProcessPlan {
                profile: state.profile().unwrap(),
                changes: vec![],
            });
            state.launch_reviewed = reviewed;
            state.user_plan = Some(transaction());
            state.apply_reviewed = reviewed;
            let mut receipt = transaction();
            receipt.state = State::Applied;
            state.receipt = Some(receipt);
            state.undo_reviewed = reviewed;
            frame(&ctx, &mut state, vec![], true);
            let (output, actions) = frame(&ctx, &mut state, vec![], true);
            assert!(!actions.launch && !actions.apply && !actions.undo);
            assert!(state.job.is_none());
            let update = output.platform_output.accesskit_update.unwrap();
            for description in [
                "Launch the exact executable once",
                "Persist this exact preview",
                "Journal Undoing",
            ] {
                let node = update
                    .nodes
                    .iter()
                    .find(|(_, node)| {
                        node.role() == egui::accesskit::Role::Button
                            && node.description().is_some_and(|d| d.contains(description))
                    })
                    .unwrap_or_else(|| panic!("Missing review control: {description}"));
                assert_eq!(node.1.is_disabled(), !reviewed);
            }
        }
    }
    #[test]
    fn controls_do_not_read_apply_launch_or_export_automatically() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = fixture();
        frame(&ctx, &mut state, vec![], false);
        let (output, actions) = frame(&ctx, &mut state, vec![], false);
        assert!(!actions.import && !actions.export && !actions.preview && !actions.user_preview);
        assert!(state.job.is_none());
        let update = output.platform_output.accesskit_update.unwrap();
        assert!(update.nodes.iter().any(|(_, node)| node
            .description()
            .is_some_and(|d| d.contains("Unrelated values are not captured or exported"))));
    }
    #[test]
    fn editing_a_profile_invalidates_both_previews_and_review_acknowledgements() {
        let ctx = egui::Context::default();
        let mut state = fixture();
        state.process = Some(ProcessPlan {
            profile: state.profile().unwrap(),
            changes: vec![],
        });
        state.user_plan = Some(transaction());
        state.launch_reviewed = true;
        state.apply_reviewed = true;
        frame(&ctx, &mut state, vec![], false);
        frame(&ctx, &mut state, vec![key(Key::Tab)], false);
        frame(&ctx, &mut state, vec![Event::Text("changed".into())], false);
        assert!(state.process.is_none() && state.user_plan.is_none());
        assert!(!state.launch_reviewed && !state.apply_reviewed);
        assert!(state.job.is_none());
    }
    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }
    #[test]
    fn keyboard_can_request_a_preview_without_executing_work_in_controls() {
        let ctx = egui::Context::default();
        let mut state = fixture();
        frame(&ctx, &mut state, vec![], false);
        for _ in 0..8 {
            frame(&ctx, &mut state, vec![key(Key::Tab)], false);
        }
        assert!(
            frame(&ctx, &mut state, vec![key(Key::Enter)], false)
                .1
                .preview
        );
        assert!(state.job.is_none());
    }
}
