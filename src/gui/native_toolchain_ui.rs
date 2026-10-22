use super::{widgets, Job};
use setupscope_core::dependencies::native::{self, Action, Options, Review};
use std::path::{Path, PathBuf};
enum Value {
    Review(Box<Review>),
    Message(String),
}
pub struct NativeState {
    bootstrapper: String,
    layout: String,
    action: Action,
    arm64: bool,
    recommended: bool,
    cmake: bool,
    sdk: bool,
    language: String,
    elevated: bool,
    review: Option<Review>,
    acknowledged: bool,
    job: Option<Job<Result<Value, String>>>,
    launching: bool,
    status: String,
}
impl Default for NativeState {
    fn default() -> Self {
        Self {
            bootstrapper: String::new(),
            layout: String::new(),
            action: Action::Install,
            arm64: false,
            recommended: false,
            cmake: true,
            sdk: true,
            language: "en-US".into(),
            elevated: false,
            review: None,
            acknowledged: false,
            job: None,
            launching: false,
            status: String::new(),
        }
    }
}
impl Drop for NativeState {
    fn drop(&mut self) {
        if let Some(j) = &self.job {
            j.cancel();
        }
    }
}
impl NativeState {
    fn invalidate(&mut self) {
        self.review = None;
        self.acknowledged = false;
        self.status =
            "Inputs changed or review discarded; prepare a fresh native setup review.".into();
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce(&setupscope_core::snapshot::Progress) -> anyhow::Result<Value>
            + Send
            + 'static,
    ) {
        self.job = Some(Job::spawn(ctx, move |p| {
            work(p).map_err(|e| format!("{e:#}"))
        }));
        self.status.clear();
    }
    pub(super) fn ui(&mut self, ui: &mut egui::Ui, root: &Path) {
        if let Some(done) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            self.launching = false;
            match done {
                Ok(Value::Review(r)) => {
                    self.review = Some(*r);
                    self.acknowledged = false;
                    self.status="Microsoft publisher verified. Review the observed hash, channel and selected workload before launch.".into();
                }
                Ok(Value::Message(m)) => self.status = m,
                Err(e) => self.status = e,
            }
        }
        super::widgets::collapsing_header(ui,setupscope_core::i18n::t("MSVC / Windows SDK / CMake / Ninja — Vendor Setup")).show(ui,|ui|{
            ui.label(setupscope_core::i18n::t("Microsoft's interactive installer manages native workloads and multi-GiB offline layouts separately from pinned bundles. Nothing downloads or installs automatically."));
            ui.hyperlink_to(setupscope_core::i18n::t("Official Build Tools 2022 Downloads And Channel Details"),"https://learn.microsoft.com/en-us/visualstudio/releases/2022/release-history");
            ui.hyperlink_to(setupscope_core::i18n::t("Official Offline Layout Instructions"),"https://learn.microsoft.com/en-us/visualstudio/install/create-an-offline-installation-of-visual-studio");
            ui.add_enabled_ui(self.job.is_none(),|ui|{
                let mut changed=false;
                for(label,value)in[("Microsoft vs_buildtools.exe",&mut self.bootstrapper),("Offline Layout Folder",&mut self.layout)]{let id=ui.label(setupscope_core::i18n::t(label)).id;changed|=ui.add(egui::TextEdit::singleline(value).desired_width(ui.available_width()).char_limit(230)).labelled_by(id).on_hover_text(setupscope_core::i18n::t("Use an absolute local path. Select a bootstrapper downloaded from Microsoft's official source. Offline install requires the bootstrapper inside the layout root.")).changed();}
                ui.horizontal_wrapped(|ui|{
                    if ui.button(setupscope_core::i18n::t("Choose Bootstrapper…")).on_hover_text(setupscope_core::i18n::t("Select the downloaded vs_buildtools.exe; selecting does not launch it.")).clicked(){if let Some(p)=widgets::pick_file("Microsoft Build Tools bootstrapper","Executable",&["exe"]){self.bootstrapper=p.display().to_string();changed=true;}}
                    if ui.button(setupscope_core::i18n::t("Choose Layout…")).on_hover_text(setupscope_core::i18n::t("Select an existing offline layout or parent folder, then edit the path for a new layout. Updating an existing layout changes its contents; preserve your own backup.")).clicked(){if let Some(p)=widgets::pick_folder("Microsoft offline layout"){self.layout=p.display().to_string();changed=true;}}
                });
                let id=ui.label(setupscope_core::i18n::t("Vendor Operation")).id;egui::ComboBox::from_id_salt("native_action").selected_text(setupscope_core::i18n::t(self.action.label())).show_ui(ui,|ui|{for action in Action::ALL{changed|=ui.selectable_value(&mut self.action,action,setupscope_core::i18n::t(action.label())).changed();}}).response.labelled_by(id).on_hover_text(setupscope_core::i18n::t("Install changes Windows. Layout downloads files; verify checks vendor layout integrity; offline install requests --noWeb. Each requires a new review."));
                ui.label(setupscope_core::i18n::t("MSVC x64/x86 selected. Optional component selection:"));
                ui.horizontal_wrapped(|ui|{
                    changed|=ui.checkbox(&mut self.sdk,setupscope_core::i18n::t("Windows SDK 26100")).on_hover_text(setupscope_core::i18n::t("Adds the explicit Windows 11 SDK 10.0.26100 component. Workload dependencies may still add SDK components when unchecked.")).changed();
                    changed|=ui.checkbox(&mut self.cmake,setupscope_core::i18n::t("CMake / Ninja")).on_hover_text(setupscope_core::i18n::t("Adds Microsoft's C++ CMake tools component, including the vendor-managed build tools. Distinct from SetupScope's pinned standalone CMake installer.")).changed();
                    changed|=ui.checkbox(&mut self.arm64,setupscope_core::i18n::t("ARM64 Target Tools")).on_hover_text(setupscope_core::i18n::t("Adds cross-compilation components; this does not change the host architecture or certify every ARM64 workflow.")).changed();
                    changed|=ui.checkbox(&mut self.recommended,setupscope_core::i18n::t("Recommended Components")).on_hover_text(setupscope_core::i18n::t("Allows additional vendor-recommended components and more disk usage; unchecked limits this request, but required workload dependencies remain.")).changed();
                });
                ui.horizontal_wrapped(|ui|{let id=ui.label(setupscope_core::i18n::t("Layout Language")).id;egui::ComboBox::from_id_salt("native_language").selected_text(&self.language).show_ui(ui,|ui|{for l in["en-US","de-DE","es-ES","fr-FR","ja-JP","zh-CN"]{changed|=ui.selectable_value(&mut self.language,l.into(),l).changed();}}).response.labelled_by(id).on_hover_text(setupscope_core::i18n::t("Language packages requested when creating a layout. Does not change Windows language."));changed|=ui.checkbox(&mut self.elevated,setupscope_core::i18n::t("Request Administrator Prompt")).on_hover_text(setupscope_core::i18n::t("Uses Windows runas after review. You handle UAC yourself; no credentials are stored. Vendor setup can independently request elevation.")).changed();});
                if changed{self.invalidate();}
                if ui.button(setupscope_core::i18n::t("Verify Publisher And Prepare Native Setup Review")).on_hover_text(setupscope_core::i18n::t("Hash the local bootstrapper and require a valid cached Microsoft signature. Does not download, launch or accept vendor terms.")).clicked(){self.invalidate();let options=Options{bootstrapper:PathBuf::from(&self.bootstrapper),layout:PathBuf::from(&self.layout),action:self.action,arm64:self.arm64,recommended:self.recommended,cmake:self.cmake,sdk:self.sdk,language:self.language.clone(),elevated:self.elevated};self.start(ui.ctx(),move|p|native::review(options,p).map(|r|Value::Review(Box::new(r))));}
                if let Some(review)=&self.review{
                    egui::ScrollArea::vertical().id_salt("native_setup_review").max_height(180.).show(ui,|ui|{ui.add(egui::Label::new(review.text()).wrap().selectable(true));});
                    ui.checkbox(&mut self.acknowledged,setupscope_core::i18n::t("I Reviewed The Command, Vendor Terms, Disk Usage And Recovery")).on_hover_text(setupscope_core::i18n::t("Authorizes this one vendor operation. Review the channel and components in Microsoft's visible installer; changes cannot be automatically undone."));
                    ui.horizontal_wrapped(|ui|{
                        if ui.add_enabled(self.acknowledged,egui::Button::new(setupscope_core::i18n::t("Launch Reviewed Microsoft Setup"))).on_hover_text(setupscope_core::i18n::t("Rechecks the held file hash and Microsoft signature, records a receipt, then opens the interactive vendor UI.")).clicked(){let review=self.review.take().expect("review exists");self.acknowledged=false;self.launching=true;let root=root.to_path_buf();self.start(ui.ctx(),move|p|native::launch(&root,review,true,p).map(Value::Message));}
                        if ui.button(setupscope_core::i18n::t("Discard Native Review")).on_hover_text(setupscope_core::i18n::t("Release the held bootstrapper without launching.")).clicked(){self.invalidate();}
                    });
                }
            });
            if let Some(job)=&self.job{ui.spinner();ui.label(job.progress.phase.lock().unwrap().clone());if self.launching{ui.label(setupscope_core::i18n::t("Complete or cancel Microsoft's UI. Closing SetupScope does not cancel vendor work."));}else if ui.button(setupscope_core::i18n::t("Cancel Publisher Verification")).on_hover_text(setupscope_core::i18n::t("Stop hashing at the next checkpoint. No vendor process has launched.")).clicked(){job.cancel();}ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));}
            if !self.status.is_empty(){ui.add(egui::Label::new(setupscope_core::i18n::t(&self.status)).wrap().selectable(true));}
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edits_clear_native_consent_without_launch() {
        let mut s = NativeState::default();
        s.acknowledged = true;
        s.invalidate();
        assert!(!s.acknowledged && s.review.is_none() && s.job.is_none() && !s.launching);
    }
}
