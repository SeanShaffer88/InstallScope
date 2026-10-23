use super::{a11y, widgets, Job};
use setupscope_core::dependencies::{self, InstallReview, Receipt};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

enum Outcome {
    Message(String),
    Review(InstallReview),
    Receipts(Vec<Receipt>),
}
pub struct DependencyState {
    native: super::native_toolchain_ui::NativeState,
    arch: String,
    preset: usize,
    selected: BTreeSet<String>,
    root: String,
    bundle: String,
    elevated: bool,
    license_reviewed: bool,
    reviewed: bool,
    review: Option<InstallReview>,
    job: Option<Job<Result<Outcome, String>>>,
    installing: bool,
    status: String,
    status_error: bool,
    receipts: Vec<Receipt>,
}
impl Default for DependencyState {
    fn default() -> Self {
        Self {
            native: Default::default(),
            arch: if cfg!(target_arch = "aarch64") {
                "arm64"
            } else {
                "x64"
            }
            .into(),
            preset: 0,
            selected: BTreeSet::new(),
            root: String::new(),
            bundle: String::new(),
            elevated: false,
            license_reviewed: false,
            reviewed: false,
            review: None,
            job: None,
            installing: false,
            status: String::new(),
            status_error: false,
            receipts: vec![],
        }
    }
}
impl Drop for DependencyState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}
impl DependencyState {
    fn invalidate(&mut self) {
        self.review = None;
        self.reviewed = false;
        self.license_reviewed = false;
    }
    fn start(
        &mut self,
        ctx: &egui::Context,
        work: impl FnOnce(&setupscope_core::snapshot::Progress) -> anyhow::Result<Outcome>
            + Send
            + 'static,
    ) {
        self.job = Some(Job::spawn(ctx, move |p| {
            work(p).map_err(|e| format!("{e:#}"))
        }));
        self.status.clear();
        self.status_error = false;
    }
    fn preset(&mut self, packages: &[dependencies::Package]) {
        self.selected.clear();
        let name = match self.preset {
            0 => ".NET SDK",
            1 => ".NET Desktop Runtime",
            2 => "CMake",
            _ => return,
        };
        for p in packages {
            if p.arch == self.arch && p.name == name {
                self.selected.insert(p.id.clone());
            }
        }
    }
    pub fn ui(&mut self, ui: &mut egui::Ui, default_root: &Path) {
        if self.root.is_empty() {
            self.root = default_root.display().to_string();
            if let Ok(c) = dependencies::catalog() {
                self.preset(&c.packages);
            }
        }
        if let Some(done) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            self.installing = false;
            match done {
                Ok(Outcome::Message(s)) => { self.status = s; self.status_error = false; }
                Ok(Outcome::Review(r)) => {
                    self.review = Some(r);
                    self.reviewed = false;
                    self.status =
                        "Review prepared from verified local installer. Nothing launched.".into();
                    self.status_error = false;
                }
                Ok(Outcome::Receipts(r)) => {
                    self.receipts = r;
                    self.status="Loaded local installation receipts. Unknown/pending outcomes require vendor inspection before retry.".into();
                    self.status_error = false;
                }
                Err(e) => { self.status = format!("Stopped: {e}"); self.status_error = true; }
            };
        }
        let catalog = match dependencies::catalog() {
            Ok(c) => c,
            Err(e) => {
                ui.label(setupscope_core::i18n::t(&format!("Catalog unavailable: {e:#}")));
                return;
            }
        };
        widgets::scope_note(ui,"Select > Download / Offline Bundle > Verify > Review One Installer","Only a compiled, pinned catalog is accepted. This view makes explicit network and disk changes; diagnostics elsewhere remain read-only. No automatic catalog refresh, installer launch or repair. Full installers are cached; project packages, workloads, MSVC and Windows SDK layouts are not included. Integrity is not proof of vendor safety. Signatures use cached Windows trust without online revocation checks. Cancel is checked between bounded operations; network calls can take up to 30 seconds to return. Closing SetupScope cannot undo or cancel a vendor installation.");
        ui.small(setupscope_core::i18n::t(&format!(
            "Pinned catalog verified {} · .NET 10 / CMake · x64 and ARM64",
            catalog.verified
        )));
        let busy = self.job.is_some();
        ui.add_enabled_ui(!busy,|ui|self.native.ui(ui, &PathBuf::from(&self.root)));
        ui.add_enabled_ui(!busy,|ui|{
            let mut changed=false;
            ui.horizontal_wrapped(|ui|{
                let architecture_label=ui.label(setupscope_core::i18n::t("Architecture"));
                egui::ComboBox::from_id_salt("dependency_arch").selected_text(&self.arch).show_ui(ui,|ui|{for a in ["x64","arm64"]{changed|=ui.selectable_value(&mut self.arch,a.into(),a).on_hover_text(setupscope_core::i18n::t("Select installer architecture. ARM64 is blocked on x64 hosts; x64 on ARM64 may use emulation. This does not certify compatibility.")).changed();}}).response.labelled_by(architecture_label.id);
            });
            ui.horizontal_wrapped(|ui|{
                let preset_label=ui.label(setupscope_core::i18n::t("Bundle Preset"));
                egui::ComboBox::from_id_salt("dependency_preset").selected_text(setupscope_core::i18n::t([".NET Development","Desktop Runtime Only","CMake Configuration","Custom"][self.preset])).show_ui(ui,|ui|{for (i,label) in [".NET Development","Desktop Runtime Only","CMake Configuration","Custom"].into_iter().enumerate(){changed|=ui.selectable_value(&mut self.preset,i,setupscope_core::i18n::t(label)).changed();}}).response.labelled_by(preset_label.id);
            });
            if changed{self.preset(&catalog.packages);self.invalidate();}
            let arch=self.arch.clone();
            for p in catalog.packages.iter().filter(|p|p.arch==arch){
                let mut chosen=self.selected.contains(&p.id);
                if a11y::explain(ui.checkbox(&mut chosen,format!("{} {} · {:.1} MiB",p.name,p.version,p.bytes as f64/1048576.)),&format!("{}\nOfficial download: {}\n{}: {}\nExpected publisher: {}",p.notes,p.url,p.algorithm,p.digest,p.publisher)).changed(){if chosen{self.selected.insert(p.id.clone());}else{self.selected.remove(&p.id);}self.preset=3;self.invalidate();}
            }
            let total:u64=catalog.packages.iter().filter(|p|self.selected.contains(&p.id)).map(|p|p.bytes).sum();
            ui.small(setupscope_core::i18n::t(&format!("{} selected · {:.1} MiB payload · allow additional staging/export space",self.selected.len(),total as f64/1048576.)));
            super::widgets::collapsing_header(ui,setupscope_core::i18n::t("Locations And Package Details")).show(ui,|ui|{
                let library_label=ui.label(setupscope_core::i18n::t("Local Library Root (cache and receipts)"));
                if ui.add(egui::TextEdit::singleline(&mut self.root).desired_width(ui.available_width()).char_limit(230)).labelled_by(library_label.id).on_hover_text(setupscope_core::i18n::t("Use an absolute local folder. Cache is dependencies/downloads; receipts are dependencies/receipts. Existing files are never silently replaced. Network/reparse paths are blocked. Changing this clears install review.")).changed(){self.invalidate();}
                if ui.button(setupscope_core::i18n::t("Choose Library Folder…")).clicked(){if let Some(p)=widgets::pick_folder("Choose local dependency library"){self.root=p.display().to_string();self.invalidate();}}
                for p in catalog.packages.iter().filter(|p|self.selected.contains(&p.id)) {super::widgets::collapsing_header(ui,format!("{} {} / {}",p.name,p.version,p.arch)).show(ui,|ui|{ui.label(&p.notes);ui.hyperlink_to(setupscope_core::i18n::t("Official Metadata"),&p.source);ui.hyperlink_to(setupscope_core::i18n::t("Vendor License / Terms"),&p.license_url);ui.hyperlink_to(setupscope_core::i18n::t("Official Installer URL"),&p.url);ui.add(egui::Label::new(setupscope_core::i18n::t(&format!("{}: {}\nPublisher: {}",p.algorithm,p.digest,p.publisher))).wrap().selectable(true));});}
            });
            let ids:Vec<_>=self.selected.iter().cloned().collect();let root=PathBuf::from(&self.root);let enabled=!ids.is_empty();
            ui.horizontal_wrapped(|ui|{
                if a11y::explain(widgets::action_button(ui,enabled,"Download Selected"),"Explicit HTTPS request to official vendor hosts and approved CDN redirects. OS TLS is validated; no cookies, automatic authentication or executable commands. Size/hash are checked before publication, then publisher checked. No installation.").clicked(){self.invalidate();let root=root.clone();let ids=ids.clone();self.start(ui.ctx(),move|p|dependencies::download(&root,&ids,p).map(Outcome::Message));}
                if a11y::explain(ui.add_enabled(enabled,egui::Button::new(setupscope_core::i18n::t("Verify Cached Files"))),"Read cached installers in the background. Check exact pinned size, SHA-256/SHA-512 and expected Authenticode publisher. Invalid or missing files block installation; nothing is downloaded or repaired.").clicked(){self.invalidate();let root=root.clone();let ids=ids.clone();self.start(ui.ctx(),move|p|dependencies::verify(&root,&ids,p).map(Outcome::Message));}
            });
            super::widgets::collapsing_header(ui,setupscope_core::i18n::t("Offline Bundle — Export / Import")).show(ui,|ui|{
                ui.label(setupscope_core::i18n::t("Folder transfer; no archive extraction. Export requires a new folder; bundle.json is written last. An incomplete folder without that manifest cannot import. Import uses compiled catalog identities and rejects foreign URLs/hashes/commands. Partial failure can leave complete cached installers or an incomplete export folder; inspect before retry."));
                let bundle_label=ui.label(setupscope_core::i18n::t("Bundle Folder (new for export; existing for import)"));
                if ui.add(egui::TextEdit::singleline(&mut self.bundle).desired_width(ui.available_width()).char_limit(230)).labelled_by(bundle_label.id).changed(){self.license_reviewed=false;}
                ui.horizontal_wrapped(|ui|{
                    if ui.button(setupscope_core::i18n::t("Choose Bundle Parent…")).on_hover_text(setupscope_core::i18n::t("Select a parent; SetupScope appends a unique new folder name for export.")).clicked(){if let Some(p)=widgets::pick_folder("Choose new bundle parent"){self.bundle=p.join(format!("SetupScope-offline-{}",setupscope_core::util::new_id())).display().to_string();self.license_reviewed=false;}}
                    if ui.button(setupscope_core::i18n::t("Choose Existing Bundle…")).clicked(){if let Some(p)=widgets::pick_folder("Choose existing offline bundle"){self.bundle=p.display().to_string();self.license_reviewed=false;}}
                });
                a11y::explain(ui.checkbox(&mut self.license_reviewed,setupscope_core::i18n::t("I reviewed selected vendors' licensing and redistribution terms")),"Review the vendor links under package details. A project's open-source license alone does not establish every installer's redistribution rights. This applies to this exact selection and destination.");
                ui.horizontal_wrapped(|ui|{
                    if ui.add_enabled(enabled&&self.license_reviewed&&!self.bundle.is_empty(),egui::Button::new(setupscope_core::i18n::t("Export New Offline Bundle"))).clicked(){let dest=PathBuf::from(&self.bundle);let root=root.clone();let ids=ids.clone();self.invalidate();self.start(ui.ctx(),move|p|dependencies::export_bundle(&root,&ids,&dest,true,p).map(Outcome::Message));}
                    if ui.add_enabled(!self.bundle.is_empty(),egui::Button::new(setupscope_core::i18n::t("Import And Verify Bundle"))).on_hover_text(setupscope_core::i18n::t("Imports only known catalog packages. Every selected bundle entry is checked before copying; existing cached files are preserved and verified. Nothing installs.")).clicked(){let src=PathBuf::from(&self.bundle);let root=root.clone();self.invalidate();self.start(ui.ctx(),move|p|dependencies::import_bundle(&root,&src,p).map(Outcome::Message));}
                });
            });
            super::widgets::collapsing_header(ui,setupscope_core::i18n::t("Reviewed Installation — One Package At A Time")).show(ui,|ui|{
                ui.label(setupscope_core::i18n::t("Select exactly one package. Installation can change this PC and cannot be automatically undone. Vendor UI controls installation options and licensing; use Installed Apps/vendor uninstaller for removal. Cancel or close the vendor UI there, not by closing SetupScope."));
                if a11y::explain(ui.checkbox(&mut self.elevated,setupscope_core::i18n::t("Request Administrator Prompt")),"Explicitly requests Windows runas/UAC. Otherwise normal vendor activation is used; the vendor can still request elevation itself. Never bypass UAC or approve an unknown publisher.").changed(){self.review=None;self.reviewed=false;}
                if ui.add_enabled(ids.len()==1,egui::Button::new(setupscope_core::i18n::t("Prepare Exact Install Review"))).on_hover_text(setupscope_core::i18n::t("Verify and hold the cached installer and parent paths; show the exact fixed vendor command, pinned digest, publisher, privilege request and recovery limits. Nothing launches.")).clicked(){self.invalidate();let root=root.clone();let id=ids[0].clone();let elevated=self.elevated;self.start(ui.ctx(),move|p|dependencies::review_install(&root,&id,elevated,p).map(Outcome::Review));}
                if let Some(review)=&self.review {
                    ui.small(setupscope_core::i18n::t("Original Review: Commands, Paths And Vendor Evidence Are Preserved"));ui.add(egui::Label::new(review.text()).wrap().selectable(true));}
                if self.review.is_some() && ui.button(setupscope_core::i18n::t("Discard Review And Release File")).clicked(){self.review=None;self.reviewed=false;}
                if self.review.is_some(){a11y::explain(ui.checkbox(&mut self.reviewed,setupscope_core::i18n::t("I reviewed this exact installer, command, licensing and system effects")),"Changing the package, library or privilege request clears review. The installer is rehashed and its signature rechecked immediately before launch. A durable receipt must be created before launching.");}
                if widgets::action_button(ui,self.review.is_some()&&self.reviewed,"Launch Reviewed Vendor Installer").clicked(){if let Some(review)=self.review.take(){self.reviewed=false;self.installing=true;let root=root.clone();self.start(ui.ctx(),move|p|dependencies::install(&root,review,true,p).map(Outcome::Message));}}
            });
            super::widgets::collapsing_header(ui,setupscope_core::i18n::t("Installation Receipts")).show(ui,|ui|{
                if ui.button(setupscope_core::i18n::t("Refresh Local Receipts")).on_hover_text(setupscope_core::i18n::t("Read at most 20 recent receipts from a bounded local directory. No vendor query or installer execution. Prepared/pending records are not proof of installed state.")).clicked(){let root=root.clone();self.start(ui.ctx(),move|_|dependencies::recent_receipts(&root).map(Outcome::Receipts));}
                for r in &self.receipts{super::widgets::collapsing_header(ui,format!("{} {} / {} — {}",r.package,r.version,r.arch,r.state)).show(ui,|ui|{ui.add(egui::Label::new(setupscope_core::i18n::t(&format!("{}\nPID {} · Exit {:?}\n{}\n{}\nDigest {}\nElevation requested {}",r.started,r.pid,r.exit_code,r.path,r.command,r.digest,r.elevated))).wrap().selectable(true));});}
            });
        });
        if let Some(job) = &self.job {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
            ui.horizontal_wrapped(|ui| {
                ui.spinner();
                ui.label(job.progress.phase.lock().unwrap().clone());
            });
            ui.small(setupscope_core::i18n::t(&format!(
                "{:.1} MiB processed",
                job.progress.count() as f64 / 1048576.
            )));
            if self.installing {
                ui.colored_label(super::theme::WARN,"Vendor installation may be active. Finish/cancel its UI; closing SetupScope cannot cancel or undo it. A pending receipt remains if tracking is interrupted.");
            } else if ui.button(setupscope_core::i18n::t("Cancel At Next Checkpoint")).clicked() {
                job.cancel();
            }
        }
        if !self.status.is_empty() {
            ui.separator();
            if self.status_error {
                ui.add(egui::Label::new(egui::RichText::new(format!("✖ {}", setupscope_core::i18n::t(&self.status))).color(super::theme::BAD)).wrap().selectable(true));
            } else {
                ui.add(egui::Label::new(setupscope_core::i18n::t(&self.status)).wrap().selectable(true));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_preset_avoids_redundant_runtime_and_other_architecture() {
        let c = dependencies::catalog().unwrap();
        let mut s = DependencyState::default();
        s.arch = "x64".into();
        s.preset(&c.packages);
        assert_eq!(s.selected.len(), 1);
        let id = s.selected.iter().next().unwrap();
        assert!(id.starts_with("dotnet-") && id.ends_with("-x64"));
        s.preset = 1;
        s.preset(&c.packages);
        assert!(s.selected.iter().next().unwrap().starts_with("runtime-"));
    }
    #[test]
    fn changed_inputs_clear_every_consent() {
        let mut s = DependencyState::default();
        s.reviewed = true;
        s.license_reviewed = true;
        s.invalidate();
        assert!(!s.reviewed && !s.license_reviewed && s.review.is_none());
    }
}
