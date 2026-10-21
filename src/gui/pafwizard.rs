//! Guided project planning and configuration preview. Package execution is gated
//! until the official-tool adapter has been verified against the current tools.
use super::{a11y, theme, widgets::*};
use egui::{RichText, Ui};
use installscope_core::pafproject::{Component, Finding, Project, FORMAT_SOURCE, STEPS};
use installscope_core::{
    pafassets,
    pafstage::{Prepared, StagePlan},
};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

type ProjectIo = Result<(PathBuf, Project), String>;

#[derive(Clone)]
enum PendingProject {
    New,
    Open(PathBuf),
}

#[derive(Default)]
pub struct Wizard {
    project: Project,
    saved: Project,
    path: Option<PathBuf>,
    pending: Option<PendingProject>,
    error: Option<String>,
    message: String,
    io_job: Option<super::Job<ProjectIo>>,
    confirm_close: bool,
    allow_close: bool,
    check_job: Option<super::Job<(Project, Vec<Finding>)>>,
    checked: Option<(Project, Vec<Finding>)>,
    icon_job: Option<super::Job<Result<(String, pafassets::IconAssets), String>>>,
    icon_preview: Option<(String, pafassets::IconAssets, egui::TextureHandle)>,
    review_job: Option<super::Job<Result<StagePlan, String>>>,
    plan: Option<StagePlan>,
    plan_texture: Option<egui::TextureHandle>,
    prepare_job: Option<super::Job<Result<Prepared, String>>>,
    prepared: Option<Prepared>,
    stage_status: Arc<Mutex<String>>,
}
fn text(ui: &mut Ui, label: &str, value: &mut String, explanation: &str) {
    let label = ui.label(label);
    a11y::explain(
        ui.add(
            egui::TextEdit::singleline(value)
                .char_limit(4096)
                .desired_width(420.0),
        )
        .labelled_by(label.id),
        explanation,
    );
}
fn choice(ui: &mut Ui, label: &str, value: &mut Option<bool>, explanation: &str) {
    ui.label(label);
    let explanation = format!("{label}: {explanation}");
    ui.horizontal(|ui| {
        for (v, name) in [
            (None, "Not reviewed"),
            (Some(true), "Yes"),
            (Some(false), "No"),
        ] {
            a11y::explain(ui.radio_value(value, v, name), &explanation);
        }
    });
}
fn folder(ui: &mut Ui, label: &str, value: &mut String, help: &str) {
    text(ui, label, value, help);
    if a11y::explain(ui.button(format!("Choose {label}…")), help).clicked() {
        if let Some(p) = pick_folder(label) {
            *value = p.to_string_lossy().into_owned();
        }
    }
}
fn file(ui: &mut Ui, label: &str, value: &mut String, extensions: &[&str], help: &str) {
    text(ui, label, value, help);
    if a11y::explain(ui.button(format!("Choose {label}…")), help).clicked() {
        if let Some(p) = pick_file(label, label, extensions) {
            *value = p.to_string_lossy().into_owned();
        }
    }
}
impl Wizard {
    pub fn ui(&mut self, ui: &mut Ui) {
        self.poll_io();
        self.poll_preparation();
        if let Some(result) = self.icon_job.as_ref().and_then(|j| j.poll()) {
            self.icon_job = None;
            match result {
                Ok((path, icon)) => self.set_icon(ui.ctx(), path, icon),
                Err(e) => self.error = Some(e),
            }
        }
        if let Some(result) = self.review_job.as_ref().and_then(|j| j.poll()) {
            self.review_job = None;
            match result {
                Ok(plan) => {
                    self.set_icon(ui.ctx(), plan.project().icon.clone(), plan.icon.clone());
                    self.plan_texture = self
                        .icon_preview
                        .as_ref()
                        .map(|(_, _, texture)| texture.clone());
                    self.plan = Some(plan);
                    self.error = None;
                }
                Err(e) => self.error = Some(e),
            }
        }
        if let Some(job) = &self.prepare_job {
            ui.heading("Preparing a new package folder");
            ui.spinner();
            ui.label(self.stage_status.lock().unwrap().as_str());
            ui.label("Original sources are untouched. Cancelling can leave a partial new folder for inspection; it is not a completed package.");
            if a11y::explain(ui.add_enabled(!job.progress.cancelled(),egui::Button::new("Cancel preparation")),"Stop between files or buffer operations. Partial new output is retained and the originals stay unchanged.").clicked(){job.cancel();}
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
            return;
        }
        if let Some(result) = self.check_job.as_ref().and_then(|j| j.poll()) {
            self.check_job = None;
            self.checked = Some(result);
        }
        ui.label(RichText::new("PortableApps.com package project").heading());
        if self.io_job.is_some() {
            ui.spinner();
            ui.label("Reading or saving project… Original source files are untouched.");
            return;
        }
        let ctx = ui.ctx().clone();
        ui.label("Plan and review a PortableApps.com package, convert its icon, and prepare files in a new workspace folder. Original sources are not edited or executed. Official build/log capture and installer-artwork customization are still pending in this wizard.");
        ui.horizontal_wrapped(|ui| {
            if a11y::explain(ui.button("New project"),"Start a new project with unanswered permissions. Unsaved answers require confirmation before replacement.").clicked() {
                if self.project!=self.saved {self.pending=Some(PendingProject::New);} else {self.reset();}
            }
            if a11y::explain(ui.button("Open project…"),"Open a saved InstallScope PAF project. Unsaved work requires confirmation before replacement.").clicked() {
                if let Some(p)=pick_file("Open package project","Project",&["json"]) {
                    if self.project != self.saved { self.pending=Some(PendingProject::Open(p)); } else { self.open(&ctx,p); }
                }
            }
            if a11y::explain(ui.button("Save snapshot…"),"Save all answers and the current step to a new JSON file. Existing files are never overwritten; use a new filename for each revision.").clicked() {
                if let Some(p)=save_file("Save project snapshot",&format!("portable-project-{}.json",installscope_core::util::new_id()),"Project","json") {
                    let project=self.project.clone();
                    self.io_job=Some(super::Job::spawn(&ctx,move |_| project.save_new(&p).map(|_|(p,project)).map_err(|e|format!("{e:#}"))));
                }
            }
            if self.project != self.saved { ui.label("Unsaved changes"); }
        });
        if let Some(action) = self.pending.clone() {
            theme::warning_banner(ui,"Starting or opening another project replaces the unsaved answers currently on this screen. Save a snapshot first if you want to keep them.");
            ui.horizontal(|ui| {
                if a11y::explain(ui.button("Discard unsaved answers and continue"),"Discard the current in-memory answers and start or open the requested project. Saved files are untouched.").clicked() { match action {PendingProject::New=>self.reset(),PendingProject::Open(path)=>self.open(&ctx,path)} self.pending=None; }
                if a11y::explain(ui.button("Keep working"),"Cancel opening the other project and retain all current answers.").clicked() {self.pending=None;}
            });
        }
        if self.io_job.is_some() {
            ui.ctx().request_repaint();
            return;
        }
        if let Some(e) = &self.error {
            theme::warning_banner(ui, e);
        }
        if !self.message.is_empty() {
            ui.label(&self.message);
        }
        if let Some(path) = &self.path {
            small_muted(
                ui,
                format!("Last opened/saved snapshot: {}", path.display()),
            );
        }
        ui.separator();
        ui.label(
            RichText::new(format!(
                "Step {} of {}: {}",
                self.project.step + 1,
                STEPS.len(),
                STEPS[self.project.step]
            ))
            .strong(),
        );
        let step = self.project.step;
        theme::card(ui, |ui| {
            let p = &mut self.project;
            match step {
                0 => {
                    ui.label("Start with application files you have permission to package, an original source folder, a separate working folder, and the installed official Launcher Generator and Installer. Wrapping an executable cannot establish portability.");
                    folder(ui,"Workspace",&mut p.workspace,"Choose an existing working folder separate from the original files. This planning stage does not create or change folders.");
                    ui.label("Save a snapshot at any step. Open it later to resume. Projects can contain local paths and notes; review them before sharing.");
                }
                1 => {
                    folder(ui,"Source folder",&mut p.source,"Choose the application files folder. Its contents will be mapped beneath the first subfolder in Program executable, shown in the review plan. Preparation copies files only after your explicit action; originals remain unchanged.");
                    ui.label("Planned layout: App holds application files and AppInfo metadata; Data holds user settings; Other holds supporting help and source material. Review file placement before any future copy operation.");
                    ui.hyperlink_to("Official directory layout", FORMAT_SOURCE);
                    ui.label("Archive import is not connected yet. Extract trusted archives into a separate folder and select that folder. Do not run extracted scripts to inspect them. Staging is limited to 100,000 entries and 64 GiB; links and special files are rejected.");
                }
                2 => {
                    text(ui,"Package display name",&mut p.name,"Enter the actual portable package name. The wizard does not append Portable or invent a publisher.");
                    text(ui,"AppID",&mut p.app_id,"Enter the intended package identifier. Review the official AppID naming rules for your publisher status; uniqueness is not checked automatically.");
                    text(ui,"Publisher",&mut p.publisher,"Identify the actual publisher and packager as applicable. Do not claim PortableApps.com authorship without authorization.");
                    text(ui,"Homepage",&mut p.homepage,"Enter the genuine homepage for this portable package. No URL is contacted.");
                    text(
                        ui,
                        "Description",
                        &mut p.description,
                        "Describe the application in up to 512 characters.",
                    );
                    let label = ui.label("Category");
                    let r = egui::ComboBox::from_id_salt("paf-project-category")
                        .selected_text(if p.category.is_empty() {
                            "Choose a category"
                        } else {
                            &p.category
                        })
                        .show_ui(ui, |ui| {
                            for c in installscope_core::pafbuild::CATEGORIES {
                                ui.selectable_value(&mut p.category, c.to_string(), c);
                            }
                        });
                    a11y::explain(r.response.labelled_by(label.id),"Select the official English category value, regardless of the application's language.");
                    text(ui,"Language",&mut p.language,"Enter the documented language value, or Multilingual when applicable. The complete locale list is not validated in this revision.");
                    text(ui,"Package version",&mut p.package_version,"Enter four numbers, such as 1.2.0.1. Choose release numbering explicitly; it is not inferred from the display version.");
                    text(
                        ui,
                        "Display version",
                        &mut p.display_version,
                        "Enter the human-readable release version, such as 1.2 Revision 1.",
                    );
                    ui.hyperlink_to("Official metadata requirements", FORMAT_SOURCE);
                }
                3 => {
                    ui.label("Where does the application store settings, data, logs and caches? Does it write to the registry, AppData, Documents, ProgramData or system locations? Does it need a runtime, service or administrator rights?");
                    let label = ui.label("Observed behavior and unresolved questions");
                    a11y::explain(ui.add(egui::TextEdit::multiline(&mut p.portability_notes).char_limit(65536).desired_rows(7).desired_width(f32::INFINITY)).labelled_by(label.id),"Record observations and uncertainties. These notes are not an automated portability assessment or a guarantee.");
                    ui.label("Record what must be tested on a clean account or isolated machine. Avoid adding cleanup rules until you understand which files belong to this application and how existing local settings will be restored.");
                }
                4 => {
                    text(ui,"Program executable relative to App",&mut p.program,"For App\\Example\\example.exe, enter Example\\example.exe. Your source folder must contain example.exe. Nested paths within Example are preserved. An application subfolder is required for staging; absolute paths and parent traversal are rejected.");
                    text(ui,"Command-line arguments (optional)",&mut p.arguments,"Use only arguments supported by the application. Review quoting against PAL documentation. No arguments are executed in this preview.");
                    ui.hyperlink_to(
                        "Official Launcher settings",
                        installscope_core::pafproject::LAUNCH_SOURCE,
                    );
                    ui.label("The draft includes only the executable and optional arguments. File moves, registry handling and custom code require separate review; notes are never converted into cleanup commands automatically.");
                }
                5 => {
                    file(ui,"App icon PNG",&mut p.icon,&["png"],"Choose a square static PNG from 256 to 4096 pixels, no larger than 16 MiB, that you have rights to use. Original artwork is not changed.");
                    if a11y::explain(ui.add_enabled(self.icon_job.is_none() && self.review_job.is_none(),egui::Button::new("Convert and preview icon")),"Decode and validate the selected PNG, then generate the documented icon variants in memory. No files are written yet.").clicked() {
                        let path=p.icon.clone();self.icon_job=Some(super::Job::spawn(&ctx,move |progress|pafassets::import(std::path::Path::new(&path),&||progress.cancelled()).map(|icon|(path,icon)).map_err(|e|format!("{e:#}"))));
                    }
                    if self.icon_job.is_some() {
                        ui.spinner();
                        ui.label("Converting icon…");
                    }
                    if let Some((path, icon, texture)) = &self.icon_preview {
                        if path == &p.icon {
                            ui.image((texture.id(), egui::vec2(128.0, 128.0)));
                            ui.label(format!("128×128 preview from {}×{} PNG. Output: PNG 16, 32, 75, 128, 256; ICO 16/32/48 at 8-bit and 32-bit, plus PNG 256. The 8-bit compatibility frames use a reduced palette and binary transparency.",icon.original_size,icon.original_size));
                        }
                    }
                    ui.label("App icon replacement is documented. Arbitrary artwork in the installer's left panel remains unverified for the official Installer; no unsupported setting is generated.");
                    ui.hyperlink_to("Official icon requirements", FORMAT_SOURCE);
                }
                6 => {
                    file(ui,"Official Installer",&mut p.installer,&["exe"],"Locate PortableApps.comInstaller.exe from the official distribution. No download or execution occurs here.");
                    file(ui,"Official Launcher Generator",&mut p.generator,&["exe"],"Locate PortableApps.comLauncherGenerator.exe. Selecting a filename does not establish provenance or compatibility.");
                    ui.label("Installer configuration: use documented defaults. No installer.ini is generated when no custom options have been selected. Existing advanced configuration must be reviewed before a future build.");
                    ui.hyperlink_to(
                        "Official Installer download and documentation",
                        installscope_core::pafbuild::INSTALLER_URL,
                    );
                }
                7 => {
                    choice(ui,"Have you verified permission to package these application files?",&mut p.rights,"Record your review; this is not a legal opinion. Unknown or denied permission blocks configuration preparation.");
                    choice(ui,"Open source?",&mut p.open_source,"Declare whether the application is fully open source under an applicable OSI-approved license. Do not infer this from price.");
                    choice(
                        ui,
                        "Free to use?",
                        &mut p.freeware,
                        "Declare whether the application is available at no cost.",
                    );
                    choice(ui,"Business use allowed?",&mut p.commercial_use,"Review the application's license before declaring business use is allowed.");
                    choice(ui,"May the package be shared?",&mut p.shareable,"Confirm redistribution rights separately from permission to run the application.");
                    ui.label("Include runtimes, plugins, fonts, artwork and other bundled files in the inventory. Notices and permission must be reviewed for every component.");
                    let mut remove = None;
                    for (i, c) in p.components.iter_mut().enumerate() {
                        ui.push_id(i,|ui| {ui.separator(); text(ui,"Component name",&mut c.name,"Identify the included component.");text(ui,"License",&mut c.license,"Record the license and applicable version; unresolved terms need human review.");text(ui,"Notice file / evidence",&mut c.notice,"Record where the license notice and permission evidence can be found.");choice(ui,"Redistribution reviewed and permitted?",&mut c.rights,"Confirm the rights for this component; do not infer permission from a download link."); if a11y::explain(ui.button("Remove inventory entry"),"Remove this component record from the current project. No application or notice files are removed.").clicked() {remove=Some(i);} });
                    }
                    if let Some(index) = remove {
                        p.components.remove(index);
                    }
                    if a11y::explain(ui.button("Add included component"),"Add a runtime, plugin, artwork or other included component to the rights inventory.").clicked() {p.components.push(Component::default());}
                }
                8 => {
                    ui.label("Configuration previews — drafts, not official review or a build result. No files are written by this preview.");
                    match p.drafts() {
                        Ok(files) => {
                            for (path, mut content) in files {
                                ui.label(RichText::new(path).strong());
                                ui.add(
                                    egui::TextEdit::multiline(&mut content)
                                        .code_editor()
                                        .interactive(false)
                                        .desired_width(f32::INFINITY),
                                );
                            }
                        }
                        Err(e) => {
                            ui.label(e.to_string());
                        }
                    }
                    ui.separator();
                    if a11y::explain(ui.add_enabled(self.review_job.is_none() && self.icon_job.is_none(),egui::Button::new("Review source mapping and icon")),"Read the selected application files and hash their contents, validate names/layout, convert the icon and create a proposed output path. No files are copied or executed. Large projects can take time.").clicked() {
                        let project=p.clone();let status=self.stage_status.clone();
                        self.review_job=Some(super::Job::spawn(&ctx,move |progress|StagePlan::create(&project,&||progress.cancelled(),&|s|*status.lock().unwrap()=s.to_string()).map_err(|e|format!("{e:#}"))));
                    }
                    if let Some(job) = &self.review_job {
                        ui.spinner();
                        ui.label(self.stage_status.lock().unwrap().as_str());
                        if a11y::explain(ui.button("Cancel review"),"Stop hashing/converting the source plan. No package files have been written.").clicked(){job.cancel();}
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(100));
                    }
                    if let Some(plan) = &self.plan {
                        if plan.project() == p {
                            ui.label(format!(
                                "{} application files, {} bytes. Proposed new output: {}",
                                plan.file_count(),
                                plan.total_bytes,
                                plan.output().display()
                            ));
                            ui.label("Review all file mappings and the configuration previews above. Every source entry is copied beneath App; empty Data and DefaultData folders, help.html, converted icons and a separate project snapshot are also created. No official Installer receipt or certification is generated.");
                            if let Some(texture) = &self.plan_texture {
                                ui.image((texture.id(), egui::vec2(128.0, 128.0)));
                                ui.label("Reviewed appicon_128.png preview from this exact plan");
                            }
                            for (name, bytes) in &plan.icon.files {
                                ui.label(format!(
                                    "Generated: App/AppInfo/{name} ({} bytes)",
                                    bytes.len()
                                ));
                            }
                            egui::ScrollArea::both()
                                .id_salt("paf-staging-mapping")
                                .max_height(220.0)
                                .show_rows(ui, 20.0, plan.entry_count(), |ui, range| {
                                    for i in range {
                                        if let Some((from, to, bytes)) = plan.mapping(i) {
                                            ui.add(
                                                egui::Label::new(format!(
                                                    "{} → {} ({bytes} bytes)",
                                                    from.display(),
                                                    to.display()
                                                ))
                                                .wrap_mode(egui::TextWrapMode::Extend),
                                            );
                                        }
                                    }
                                });
                            let already = self
                                .prepared
                                .as_ref()
                                .is_some_and(|x| x.package.parent() == Some(plan.output()));
                            if a11y::explain(ui.add_enabled(!already,egui::Button::new("Prepare reviewed files in the new folder")),"Create exactly the new output folder shown above. Copy the reviewed source contents, write draft configurations and icons, and save a project snapshot. No source overwrites, downloads or program execution occur. Partial output is retained on cancellation/failure.").clicked() {
                                let plan=plan.clone();let status=self.stage_status.clone();self.error=None;
                                self.prepare_job=Some(super::Job::spawn(&ctx,move |progress|plan.prepare(&||progress.cancelled(),&|s|*status.lock().unwrap()=s.to_string()).map_err(|e|format!("{e:#}"))));
                            }
                            if already {
                                ui.label("This plan was prepared. Create a new review plan to prepare another copy.");
                            }
                        } else {
                            ui.label("Answers changed after this plan. Create a new review plan before copying files.");
                        }
                    }
                    ui.label("Official build execution and compiler log capture are not connected to this wizard yet. There is no fallback installer.");
                }
                _ => {
                    ui.label("Manual test record. Tick only tests you actually performed on the built package; this wizard does not run them yet.");
                    for item in [
                        "Install into an isolated temporary folder",
                        "Launch and change settings",
                        "Close and relaunch: settings remain",
                        "Move to another path and drive letter",
                        "Upgrade: user Data survives",
                        "Verify host files and registry are restored",
                        "Test on a clean Windows account or machine",
                    ] {
                        let mut done = p.tests.iter().any(|s| s == item);
                        if a11y::explain(ui.checkbox(&mut done,item),"Record a manual observation only. This checkmark is not automatic verification or official certification.").changed() {
                            p.tests.retain(|s|s!=item);if done {p.tests.push(item.to_string());}
                        }
                    }
                    ui.label("Save a project snapshot to retain drafts, notes and test records. Prepared folders are not installable artifacts; build and testing remain separate. An installable package and validation report are not exported yet.");
                }
            }
        });
        if self.prepare_job.is_some() {
            ui.ctx().request_repaint();
            return;
        }
        ui.separator();
        if a11y::explain(ui.add_enabled(self.check_job.is_none(),egui::Button::new("Check folders and tools")),"Check whether the selected files and folders are accessible and the source and workspace overlap. This runs in the background and does not execute tools or validate their publisher.").clicked() {
            let project=self.project.clone();
            self.check_job=Some(super::Job::spawn(&ctx,move |_| {let result=project.inspect_paths();(project,result)}));
        }
        if self.check_job.is_some() {
            ui.spinner();
            ui.label("Checking paths… Network and removable drives may take time.");
        }
        if let Some((project, findings)) = &self.checked {
            if project == &self.project {
                if findings.is_empty() {
                    ui.label("Selected paths are accessible and source/workspace folders are separate. Tool authenticity and asset contents are unverified.");
                }
                for f in findings {
                    ui.label(format!("{}: {}", f.field, f.message));
                }
            } else {
                small_muted(ui,"Answers changed after the last path check. Check again before using these paths.");
            }
        }
        if let Some(prepared) = &self.prepared {
            ui.label(RichText::new("Prepared files — build and testing still required").strong());
            ui.label(format!(
                "{} application source files, {} source bytes: {}",
                prepared.files,
                prepared.bytes,
                prepared.package.display()
            ));
            ui.label(format!("Saved project: {}", prepared.record.display()));
            ui.label(format!("Preparation report: {}", prepared.report.display()));
            if a11y::explain(
                ui.button("Open prepared folder"),
                "Open the new staging folder for review. No launcher or installer is run.",
            )
            .clicked()
            {
                installscope_core::platform::shell_open(&prepared.package.to_string_lossy());
            }
        }
        let findings = self.project.findings();
        for f in findings.iter().filter(|f| f.step == step) {
            ui.label(
                RichText::new(format!(
                    "{} — {}: {}",
                    if f.blocking {
                        "Needs attention"
                    } else {
                        "Warning"
                    },
                    f.field,
                    f.message
                ))
                .color(if f.blocking {
                    theme::WARN
                } else {
                    theme::MUTED
                }),
            );
            small_muted(ui, f.class.label());
            if !f.source.is_empty() {
                ui.hyperlink_to("Requirement / reference", f.source);
            }
        }
        ui.horizontal(|ui| {
            if a11y::explain(ui.add_enabled(step>0,egui::Button::new("Back")),"Return to the previous step. Answers remain in memory; save a snapshot to keep them after closing.").clicked(){self.project.step-=1;}
            if a11y::explain(ui.add_enabled(step+1<STEPS.len(),egui::Button::new("Next")),"Continue reviewing the project. Unresolved findings remain visible and block configuration generation where applicable; navigation does not imply validation passed.").clicked(){self.project.step+=1;}
        });
        small_muted(ui,format!("{} unresolved checks across the project. Navigation is allowed so you can return with missing information.",findings.iter().filter(|f|f.blocking).count()));
    }
    fn reset(&mut self) {
        self.project = Project::default();
        self.saved = self.project.clone();
        self.path = None;
        if let Some(job) = &self.review_job {
            job.cancel();
        }
        self.review_job = None;
        if let Some(job) = &self.icon_job {
            job.cancel();
        }
        self.icon_job = None;
        self.plan = None;
        self.plan_texture = None;
        self.prepared = None;
        self.icon_preview = None;
        self.pending = None;
        self.checked = None;
        self.error = None;
        self.message = "New project. Permissions are unanswered.".into();
    }
    fn set_icon(&mut self, ctx: &egui::Context, path: String, icon: pafassets::IconAssets) {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [icon.preview.width as usize, icon.preview.height as usize],
            &icon.preview.pixels,
        );
        let texture = ctx.load_texture("paf-project-app-icon", image, egui::TextureOptions::LINEAR);
        self.icon_preview = Some((path, icon, texture));
    }
    fn poll_preparation(&mut self) {
        if let Some(result) = self.prepare_job.as_ref().and_then(|j| j.poll()) {
            self.prepare_job = None;
            match result {
                Ok(output) => {
                    self.path = Some(output.record.clone());
                    self.saved = self.project.clone();
                    self.message="Files prepared in a new folder. Original sources are intact; build and testing remain required.".into();
                    self.prepared = Some(output);
                    self.error = None;
                }
                Err(e) => self.error = Some(e),
            }
        }
    }
    fn poll_io(&mut self) {
        if let Some(result) = self.io_job.as_ref().and_then(|j| j.poll()) {
            self.io_job = None;
            match result {
                Ok((path, project)) => {
                    if self.project != project {
                        if let Some(job) = &self.review_job {
                            job.cancel();
                        }
                        self.review_job = None;
                        if let Some(job) = &self.icon_job {
                            job.cancel();
                        }
                        self.icon_job = None;
                        self.plan = None;
                        self.plan_texture = None;
                        self.prepared = None;
                        self.icon_preview = None;
                    }
                    self.saved = project.clone();
                    self.project = project;
                    self.path = Some(path);
                    self.error = None;
                    self.message =
                        "Project snapshot ready. No source files or tools were executed.".into();
                    self.checked = None;
                }
                Err(e) => self.error = Some(e),
            }
        }
    }
    pub fn guard_close(&mut self, ctx: &egui::Context) {
        self.poll_io();
        self.poll_preparation();
        if ctx.input(|i| i.viewport().close_requested())
            && !self.allow_close
            && (self.project != self.saved || self.io_job.is_some() || self.prepare_job.is_some())
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close = true;
        }
        if self.confirm_close {
            egui::Window::new("Unsaved package project").collapsible(false).resizable(false).show(ctx,|ui| {
                ui.label("The package project has unsaved answers. Keep working and save a snapshot, or close and discard the in-memory changes. Saved files are unaffected.");
                if self.io_job.is_some() || self.prepare_job.is_some() { ui.label("A project read/save or preparation is still finishing. Wait for it before closing; preparation can be cancelled from the wizard."); }
                if a11y::explain(ui.button("Keep working"),"Return to InstallScope and retain all answers. Use Save snapshot in Guided PAF project to save them.").clicked() {self.confirm_close=false;}
                if a11y::explain(ui.add_enabled(self.io_job.is_none() && self.prepare_job.is_none(),egui::Button::new("Close and discard unsaved answers")),"Close InstallScope and discard the unsaved project answers. Existing project snapshots and original application files remain unchanged.").clicked() {
                    self.allow_close=true;self.confirm_close=false;ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        }
    }
    fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        self.io_job = Some(super::Job::spawn(ctx, move |_| {
            Project::load(&path)
                .map(|p| (path, p))
                .map_err(|e| format!("{e:#}"))
        }));
    }
}
