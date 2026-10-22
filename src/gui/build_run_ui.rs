use super::{a11y, widgets, Job};
use setupscope_core::build_run::{self, Outcome, Request, Review};
use std::path::{Path, PathBuf};
enum ResultValue {
    Review(Review),
    Run(Outcome),
}
pub struct RunState {
    project: String,
    executable: String,
    arguments: String,
    logs: String,
    timeout: u32,
    preset: usize,
    review: Option<Review>,
    acknowledged: bool,
    job: Option<Job<Result<ResultValue, String>>>,
    status: String,
    last_log: Option<PathBuf>,
}
impl Default for RunState {
    fn default() -> Self {
        Self {
            project: String::new(),
            executable: String::new(),
            arguments: "build\n--locked\n--offline".into(),
            logs: String::new(),
            timeout: 600,
            preset: 0,
            review: None,
            acknowledged: false,
            job: None,
            status: String::new(),
            last_log: None,
        }
    }
}
impl Drop for RunState {
    fn drop(&mut self) {
        if let Some(j) = &self.job {
            j.cancel();
        }
    }
}
impl RunState {
    pub(super) fn accept_project(&mut self, path: &Path) -> bool {
        if self.job.is_some() {
            return false;
        }
        self.project = path.display().to_string();
        self.invalidate();
        true
    }
    fn invalidate(&mut self) {
        self.review = None;
        self.acknowledged = false;
        self.status =
            "Inputs changed or review discarded; prepare a fresh execution review.".into();
    }
    fn request(&self) -> Request {
        Request {
            project: self.project.clone().into(),
            executable: self.executable.clone().into(),
            arguments: self.arguments.lines().map(str::to_owned).collect(),
            logs: self.logs.clone().into(),
            timeout_seconds: self.timeout,
        }
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce(&setupscope_core::snapshot::Progress) -> anyhow::Result<ResultValue>
            + Send
            + 'static,
    ) {
        self.job = Some(Job::spawn(ctx, move |p| {
            work(p).map_err(|e| format!("{e:#}"))
        }));
        self.status.clear();
    }
    pub(super) fn ui(&mut self, ui: &mut egui::Ui, root: &Path) -> Option<PathBuf> {
        if self.logs.is_empty() {
            self.logs = root.display().to_string();
        }
        if let Some(done) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match done {
                Ok(ResultValue::Review(r)) => {
                    self.review = Some(r);
                    self.acknowledged = false;
                    self.status = "Review ready. Nothing executed.".into();
                }
                Ok(ResultValue::Run(r)) => {
                    self.status = r.summary;
                    self.last_log = Some(r.log);
                }
                Err(e) => self.status = e,
            }
        }
        let mut selected_log = None;
        super::widgets::collapsing_header(ui,setupscope_core::i18n::t("Reviewed Build / Test — Executes Project Code")).show(ui,|ui|{
            ui.label(setupscope_core::i18n::t("Explicit execution, separate from read-only diagnostics. Changes cannot be automatically undone. Inherits your account, environment and network access."));
            ui.add_enabled_ui(self.job.is_none(),|ui|{
                let mut changed=false;
                for (label,value) in [("Working Project Folder",&mut self.project),("Tool Executable (.exe)",&mut self.executable),("Existing Log Folder",&mut self.logs)] {
                    let id=ui.label(setupscope_core::i18n::t(label)).id;changed|=ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()).char_limit(230)).labelled_by(id).on_hover_text(setupscope_core::i18n::t("Absolute local path. Network, reparse and parent-traversal paths are blocked. No PATH executable lookup.")).changed();
                }
                ui.horizontal_wrapped(|ui|{
                    if ui.button(setupscope_core::i18n::t("Choose Project…")).on_hover_text(setupscope_core::i18n::t("Select the working folder; does not run anything.")).clicked(){if let Some(p)=widgets::pick_folder("Build working folder"){self.project=p.display().to_string();changed=true;}}
                    if ui.button(setupscope_core::i18n::t("Choose Tool…")).on_hover_text(setupscope_core::i18n::t("Select the trusted compiler/build tool .exe yourself. Scripts and shell command parsing are not supported.")).clicked(){if let Some(p)=widgets::pick_file("Choose trusted build tool","Executable",&["exe"]){self.executable=p.display().to_string();changed=true;}}
                    if ui.button(setupscope_core::i18n::t("Choose Logs…")).on_hover_text(setupscope_core::i18n::t("Existing local folder for a uniquely named log; never overwrites a prior log.")).clicked(){if let Some(p)=widgets::pick_folder("Build log folder"){self.logs=p.display().to_string();changed=true;}}
                });
                let presets=["Rust Build (Offline)","Rust Test (Offline)",".NET Build (No Restore)",".NET Test (No Restore)","CMake Build", "CTest", "MSBuild", "CMake Configure", "Custom"];
                let id=ui.label(setupscope_core::i18n::t("Argument Preset")).id;
                let mut preset_changed=false;egui::ComboBox::from_id_salt("run_preset").selected_text(setupscope_core::i18n::t(presets[self.preset])).show_ui(ui,|ui|{for(i,name)in presets.iter().enumerate(){preset_changed|=ui.selectable_value(&mut self.preset,i,setupscope_core::i18n::t(name)).changed();}}).response.labelled_by(id).on_hover_text(setupscope_core::i18n::t("Prefills literal arguments only. Select the corresponding executable separately. Offline/no-restore flags do not restrict network access by project code."));
                if preset_changed {self.arguments=match self.preset{0=>"build\n--locked\n--offline",1=>"test\n--locked\n--offline",2=>"build\n--no-restore",3=>"test\n--no-restore",4=>"--build\nbuild\n--config\nDebug",5=>"--test-dir\nbuild\n--output-on-failure",6=>"/p:Configuration=Debug",7=>"-S\n.\n-B\nbuild",_=>""}.into();changed=true;}
                let id=ui.label(setupscope_core::i18n::t("Arguments — One Literal Argument Per Line")).id;
                changed|=ui.add(egui::TextEdit::multiline(&mut self.arguments).desired_rows(3).desired_width(ui.available_width()).char_limit(16000)).labelled_by(id).on_hover_text(setupscope_core::i18n::t("Do not add shell quotes. Spaces stay within the argument. Blank lines represent an empty argument. No pipes, substitutions or shell operators are interpreted by SetupScope; the chosen executable interprets its own arguments.")).changed();
                ui.horizontal(|ui|{let id=ui.label(setupscope_core::i18n::t("Timeout (Seconds)")).id;changed|=ui.add(egui::DragValue::new(&mut self.timeout).range(1..=7200)).labelled_by(id).on_hover_text(setupscope_core::i18n::t("At the deadline, stop the contained process tree. Does not undo changes. Maximum two hours.")).changed();});
                if changed{self.invalidate();}
                if a11y::explain(ui.button(setupscope_core::i18n::t("Prepare Execution Review")),"Validate local paths and hold the selected executable. Nothing executes until separate acknowledgment and Run.").clicked(){self.invalidate();let request=self.request();self.start(ui.ctx(),move|_|build_run::review(request).map(ResultValue::Review));}
                if let Some(review)=&self.review {
                    ui.small(setupscope_core::i18n::t("Original Review: Commands, Paths And Vendor Evidence Are Preserved"));
                    egui::ScrollArea::vertical().id_salt("run_review").max_height(160.).show(ui,|ui|{ui.add(egui::Label::new(review.text()).wrap().selectable(true));});
                    ui.checkbox(&mut self.acknowledged,setupscope_core::i18n::t("I Reviewed The Code, Tool, Arguments And Side Effects")).on_hover_text(setupscope_core::i18n::t("Authorizes only this reviewed run. Any input change clears approval. This is not a sandbox."));
                    ui.horizontal_wrapped(|ui|{
                        if ui.add_enabled(self.acknowledged,egui::Button::new(setupscope_core::i18n::t("Run Reviewed Command"))).on_hover_text(setupscope_core::i18n::t("Executes now with your current privileges. Writes a local log and may modify project files or access the network.")).clicked(){let review=self.review.take().expect("review shown");self.acknowledged=false;self.start(ui.ctx(),move|p|build_run::run(review,true,p).map(ResultValue::Run));}
                        if ui.button(setupscope_core::i18n::t("Discard Review")).on_hover_text(setupscope_core::i18n::t("Release the held executable without running.")).clicked(){self.invalidate();}
                    });
                }
            });
            if let Some(job)=&self.job{ui.spinner();if ui.button(setupscope_core::i18n::t("Cancel Pending / Running Work")).on_hover_text(setupscope_core::i18n::t("Cancels preparation or stops the contained build process tree. Already completed side effects remain.")).clicked(){job.cancel();}ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));}
            if !self.status.is_empty(){ui.add(egui::Label::new(setupscope_core::i18n::t(&self.status)).wrap().selectable(true));}
            if let Some(log)=&self.last_log{if ui.button(setupscope_core::i18n::t("Use Captured Log For Triage")).on_hover_text(setupscope_core::i18n::t("Select the last captured log in saved-log triage. Inspection remains a separate explicit action. Logs can contain secrets.")).clicked(){selected_log=Some(log.clone());}}
        });
        selected_log
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn project_handoff_clears_consent_without_starting_work() {
        let mut s = RunState::default();
        s.acknowledged = true;
        assert!(s.accept_project(Path::new("C:/Dev/Example")));
        assert!(!s.acknowledged && s.review.is_none() && s.job.is_none());
    }
    #[test]
    fn default_render_is_collapsed_and_does_not_execute() {
        let ctx = egui::Context::default();
        let mut s = RunState::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                s.ui(ui, Path::new("C:/Data"));
            });
        });
        assert!(s.job.is_none() && s.review.is_none() && !s.acknowledged);
    }
}
