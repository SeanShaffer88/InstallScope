//! PortableApps Forge page: capture an installation (in Windows Sandbox or
//! on this PC) and turn it into a PortableApps.com package.

use super::app::App;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use setupsweep_core::i18n::{t, tf};
use setupsweep_core::model::{human_size, InstallLog};
use setupsweep_core::pafbuild::{self, AppDetails, Compiled, InstallerKind, PaTools, Package};
use setupsweep_core::platform;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct BuilderState {
    pub(super) wizard: super::pafwizard::Wizard,
    show_capture: bool,
    pub installer: String,
    kind: Option<InstallerKind>,
    pub details: AppDetails,
    silent: bool,
    include_defaults: bool,
    use_sandbox: bool,
    sandbox: Option<bool>,
    out_dir: String,
    tools: Option<PaTools>,
    tools_job: Option<Job<PaTools>>,
    allow_custom_code: bool,
    /// What the capture is doing, as shown to the user.
    stage: Arc<Mutex<String>>,
    /// Set when the user says the installation is finished (this-PC mode).
    host_done: Arc<AtomicBool>,
    waiting_for_user: Arc<AtomicBool>,
    capture_job: Option<Job<Result<(InstallLog, Package), String>>>,
    /// Sandbox capture: the work folder whose status.json is watched.
    work_dir: Option<PathBuf>,
    collect_job: Option<Job<Result<(PathBuf, Package), String>>>,
    last_poll: Option<std::time::Instant>,
    /// The finished package (its folder) and the capture's install log.
    package: Option<(PathBuf, Package)>,
    log: Option<InstallLog>,
    on_this_pc: bool,
    compile_job: Option<Job<Result<Compiled, String>>>,
    compiled: Option<Compiled>,
    error: Option<String>,
    initialized: bool,
}

impl BuilderState {
    fn busy(&self) -> bool {
        self.capture_job.is_some()
            || self.work_dir.is_some()
            || self.collect_job.is_some()
            || self.compile_job.is_some()
    }
}

impl App {
    fn builder_init(&mut self, ctx: &egui::Context) {
        let b = &mut self.builder;
        if b.initialized {
            return;
        }
        b.initialized = true;
        let available = pafbuild::sandbox_available();
        b.sandbox = Some(available);
        b.use_sandbox = available;
        b.silent = false;
        b.details.packager = self.settings.packager_name.clone();
        b.out_dir = if self.settings.portable_out_dir.is_empty() {
            std::env::var("USERPROFILE")
                .map(|p| format!("{p}\\Documents\\Portable Apps"))
                .unwrap_or_default()
        } else {
            self.settings.portable_out_dir.clone()
        };
        self.refresh_tools(ctx);
    }

    fn refresh_tools(&mut self, ctx: &egui::Context) {
        let dir = self.settings.portableapps_dir.clone();
        self.builder.tools_job = Some(Job::spawn(ctx, move |_| pafbuild::find_tools(&dir)));
    }

    pub fn poll_builder(&mut self, ctx: &egui::Context) {
        let b = &mut self.builder;
        if let Some(tools) = b.tools_job.as_ref().and_then(|j| j.poll()) {
            b.tools_job = None;
            b.tools = Some(tools);
        }
        if let Some(r) = b.capture_job.as_ref().and_then(|j| j.poll()) {
            b.capture_job = None;
            match r {
                Ok((log, pkg)) => {
                    let _ = self.store.save_log(&log);
                    self.builder.package = Some((PathBuf::from(&pkg.dir), pkg));
                    self.builder.log = Some(log);
                    self.reload_logs();
                    self.programs.needs_refresh = true;
                    self.after_capture(ctx);
                }
                Err(e) => self.builder.error = Some(e),
            }
            return;
        }
        // Sandbox: watch status.json about once a second.
        if let Some(work) = b.work_dir.clone() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
            if b.last_poll.is_some_and(|t| t.elapsed().as_millis() < 900) {
                return;
            }
            b.last_poll = Some(std::time::Instant::now());
            if let Some(st) = pafbuild::read_status(&work) {
                *b.stage.lock().unwrap() = st.phase.clone();
                if st.done {
                    b.work_dir = None;
                    match (st.error, st.package) {
                        (Some(e), _) => b.error = Some(e),
                        (None, Some(pkg)) => {
                            let out = PathBuf::from(&b.out_dir);
                            b.collect_job = Some(Job::spawn(ctx, move |_| {
                                pafbuild::collect_sandbox_package(&work, &pkg, &out)
                                    .map(|dir| {
                                        let mut pkg = pkg.clone();
                                        pkg.dir = dir.to_string_lossy().to_string();
                                        (dir, pkg)
                                    })
                                    .map_err(|e| e.to_string())
                            }));
                        }
                        (None, None) => b.error = Some(t("No package was made.")),
                    }
                }
            }
        }
        if let Some(r) = self.builder.collect_job.as_ref().and_then(|j| j.poll()) {
            self.builder.collect_job = None;
            match r {
                Ok(p) => {
                    self.builder.package = Some(p);
                    self.after_capture(ctx);
                }
                Err(e) => self.builder.error = Some(e),
            }
        }
        if let Some(r) = self.builder.compile_job.as_ref().and_then(|j| j.poll()) {
            self.builder.compile_job = None;
            match r {
                Ok(c) => {
                    if let Some(p) = &c.paf {
                        self.set_status(tf("Made {file}", &[("file", p.display().to_string())]));
                    }
                    self.builder.compiled = Some(c);
                }
                Err(e) => self.builder.error = Some(e),
            }
            self.refresh_tools(ctx);
        }
    }

    /// Keep the prepared files for review before any build tool runs.
    fn after_capture(&mut self, _ctx: &egui::Context) {
        self.builder.allow_custom_code = false;
        self.set_status(t(
            "Capture complete. Review the tool commands, then compile.",
        ));
    }

    fn start_compile(&mut self, ctx: &egui::Context) {
        let Some((dir, pkg)) = self.builder.package.clone() else {
            return;
        };
        let tools = self.builder.tools.clone().unwrap_or_default();
        self.builder.error = None;
        self.builder.compiled = None;
        let stage = self.builder.stage.clone();
        let allow_custom_code = std::mem::take(&mut self.builder.allow_custom_code);
        self.builder.compile_job = Some(Job::spawn(ctx, move |progress| {
            let r = pafbuild::compile_official_with_cancel(
                &dir,
                &pkg.app_id,
                &tools,
                allow_custom_code,
                &|p| *stage.lock().unwrap() = p.to_string(),
                &|| progress.cancelled(),
            )
            .map_err(|e| e.to_string());
            r
        }));
    }

    fn start_capture(&mut self, ctx: &egui::Context) {
        self.settings.packager_name = self.builder.details.packager.clone();
        self.settings.portable_out_dir = self.builder.out_dir.clone();
        self.save_settings();
        let b = &mut self.builder;
        b.error = None;
        b.package = None;
        b.log = None;
        b.compiled = None;
        b.host_done.store(false, Ordering::SeqCst);
        b.waiting_for_user.store(false, Ordering::SeqCst);
        let job = pafbuild::Job {
            details: b.details.clone(),
            installer: b.installer.clone(),
            silent: b.silent,
            out_dir: b.out_dir.clone(),
            include_defaults: b.include_defaults,
            learn_seconds: 30,
            sandbox: false,
            settings: self.settings.clone(),
            preview_only: false,
        };
        if b.use_sandbox {
            let work = self
                .store
                .root
                .join("builder")
                .join(setupsweep_core::util::new_id());
            *b.stage.lock().unwrap() = t("Starting Windows Sandbox");
            match pafbuild::start_sandbox(&job, &work) {
                Ok(()) => {
                    b.on_this_pc = false;
                    b.work_dir = Some(work);
                    b.last_poll = None;
                }
                Err(e) => b.error = Some(e.to_string()),
            }
            return;
        }
        b.on_this_pc = true;
        let reg = self.reg.clone();
        let stage = b.stage.clone();
        let done = b.host_done.clone();
        let waiting = b.waiting_for_user.clone();
        let ctx2 = ctx.clone();
        b.capture_job = Some(Job::spawn(ctx, move |pr| {
            let report = |p: &str| {
                *stage.lock().unwrap() = p.to_string();
                ctx2.request_repaint();
            };
            let ask = || {
                waiting.store(true, Ordering::SeqCst);
                ctx2.request_repaint();
                while !done.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }
                waiting.store(false, Ordering::SeqCst);
            };
            pafbuild::capture(&job, reg.as_ref(), &report, &ask, pr).map_err(|e| e.to_string())
        }));
    }

    pub fn ui_builder(&mut self, ctx: &egui::Context) {
        self.builder_init(ctx);
        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        // The page's explanation line (see a11y::explain):
                        // the focused control's explanation, for keyboard
                        // and touch users who never see a hover tooltip.
                        super::a11y::focus_hint(ui, theme::MUTED);
                        if let Some(e) = self.builder.error.clone() {
                            theme::warning_banner(ui, e);
                            ui.add_space(6.0);
                        }
                        if !self.builder.busy() && !self.builder.wizard.operation_active() {
                            ui.horizontal(|ui| {
                                super::a11y::explain(ui.selectable_value(&mut self.builder.show_capture, false, "Guided PAF project"), "Plan and save a project from existing application files, with source-linked configuration previews.");
                                super::a11y::explain(ui.selectable_value(&mut self.builder.show_capture, true, "Installation capture"), "Use the existing installation capture workflow. This can install software on this PC or in Windows Sandbox.");
                            });
                        }
                        if !self.builder.show_capture && !self.builder.busy() {
                            self.builder.wizard.ui(ui);
                        } else if self.builder.busy() {
                            self.ui_builder_running(ui);
                        } else if self.builder.package.is_some() {
                            self.ui_builder_result(ui);
                        } else {
                            self.ui_builder_setup(ui);
                        }
                    });
            });
    }

    fn ui_builder_setup(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("1. Installer")).strong());
            ui.horizontal(|ui| {
                super::a11y::explain(
                    ui.add(
                        egui::TextEdit::singleline(&mut self.builder.installer)
                            .hint_text(t("The program's setup file (.exe or .msi)"))
                            .desired_width(420.0),
                    ),
                    &t("The program's setup file (.exe or .msi): type its path or use Browse. Its kind is detected and shown under this box."),
                );
                if super::a11y::explain(
                    ui.button(t("Browse…")),
                    &t("Choose the program's setup file (.exe or .msi)."),
                )
                .clicked()
                {
                    if let Some(p) =
                        pick_file(&t("Choose the installer"), "Installer", &["exe", "msi"])
                    {
                        self.builder.installer = p.to_string_lossy().to_string();
                        self.builder.kind = None;
                        if self.builder.details.name.is_empty() {
                            let stem = p
                                .file_stem()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_default();
                            // "Foo-1.2.3-setup-x64" → "Foo"
                            let name: String = stem
                                .split(['-', '_', ' '])
                                .take_while(|w| {
                                    !w.chars().next().is_some_and(|c| c.is_ascii_digit())
                                        && !w.eq_ignore_ascii_case("setup")
                                        && !w.eq_ignore_ascii_case("installer")
                                        && !w.starts_with('v')
                                })
                                .collect::<Vec<_>>()
                                .join(" ");
                            self.builder.details.name = if name.is_empty() { stem } else { name };
                        }
                    }
                }
            });
            if !self.builder.installer.is_empty() && self.builder.kind.is_none() {
                let p = PathBuf::from(&self.builder.installer);
                if p.is_file() {
                    self.builder.kind = Some(pafbuild::detect_installer(&p));
                }
            }
            if let Some(k) = self.builder.kind {
                small_muted(ui, tf("Installer type: {kind}", &[("kind", t(k.label()))]));
                ui.add_enabled_ui(k.can_be_silent(), |ui| {
                    let silent_why = if k.can_be_silent() {
                        t("Installs without showing the installer's questions, using its default choices.")
                    } else {
                        t("Unavailable: this kind of installer can't be run silently, so it is installed with its normal questions.")
                    };
                    super::a11y::explain(
                        ui.checkbox(
                            &mut self.builder.silent,
                            t("Install silently (no questions; uses the default choices)"),
                        ),
                        &silent_why,
                    );
                });
                if !k.can_be_silent() {
                    self.builder.silent = false;
                }
            }
        });
        ui.add_space(8.0);
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("2. About the program")).strong());
            let d = &mut self.builder.details;
            egui::Grid::new("pa_details")
                .num_columns(2)
                .spacing([12.0, 6.0])
                .show(ui, |ui| {
                    let name_label = ui.label(t("Name"));
                    super::a11y::explain(
                        ui.add(
                            egui::TextEdit::singleline(&mut d.name)
                                .hint_text("Notepad++")
                                .desired_width(320.0),
                        )
                        .labelled_by(name_label.id),
                        &t("The program's name, used for the package's name and folder."),
                    );
                    ui.end_row();
                    let version_label = ui.label(t("Version"));
                    super::a11y::explain(
                        ui.add(
                            egui::TextEdit::singleline(&mut d.version)
                                .hint_text(t("Read from the installation when empty"))
                                .desired_width(320.0),
                        )
                        .labelled_by(version_label.id),
                        &t("The program's version, shown in the package. Left empty, it is read from the installation during the capture."),
                    );
                    ui.end_row();
                    let publisher_label = ui.label(t("Publisher"));
                    super::a11y::explain(
                        ui.add(
                            egui::TextEdit::singleline(&mut d.publisher)
                                .hint_text(t("Read from the installation when empty"))
                                .desired_width(320.0),
                        )
                        .labelled_by(publisher_label.id),
                        &t("The program's publisher, shown in the package. Left empty, it is read from the installation during the capture."),
                    );
                    ui.end_row();
                    let packager_label = ui.label(t("Packaged by"));
                    super::a11y::explain(
                        ui.add(
                            egui::TextEdit::singleline(&mut d.packager)
                                .hint_text(t("Your name (optional)"))
                                .desired_width(320.0),
                        )
                        .labelled_by(packager_label.id),
                        &t("Your name, recorded in the package as who packaged it. Optional, and remembered for next time."),
                    );
                    ui.end_row();
                    let category_label = ui.label(t("Category"));
                    let combo = egui::ComboBox::from_id_salt("pa_cat")
                        .selected_text(t(&d.category))
                        .show_ui(ui, |ui| {
                            for c in pafbuild::CATEGORIES {
                                ui.selectable_value(&mut d.category, c.to_string(), t(c));
                            }
                        });
                    super::a11y::explain(
                        combo.response.labelled_by(category_label.id),
                        &t("The category the package is listed under."),
                    );
                    ui.end_row();
                    let description_label = ui.label(t("Description"));
                    super::a11y::explain(
                        ui.add(
                            egui::TextEdit::singleline(&mut d.description)
                                .char_limit(512)
                                .desired_width(420.0),
                        )
                        .labelled_by(description_label.id),
                        &t("A short description of the program, written into the package (up to 512 characters)."),
                    );
                    ui.end_row();
                    let homepage_label = ui.label(t("Homepage"));
                    super::a11y::explain(
                        ui.add(
                            egui::TextEdit::singleline(&mut d.homepage)
                                .hint_text("https://")
                                .desired_width(420.0),
                        )
                        .labelled_by(homepage_label.id),
                        &t("The program's website, written into the package so its users can visit it."),
                    );
                    ui.end_row();
                    ui.label(t("License"));
                    ui.horizontal_wrapped(|ui| {
                        super::a11y::explain(
                            ui.checkbox(&mut d.freeware, t("Free")),
                            &t("Tick if the program costs nothing to use; recorded in the package's licence information."),
                        );
                        super::a11y::explain(
                            ui.checkbox(&mut d.open_source, t("Open source")),
                            &t("Tick if the program's source code is openly available; recorded in the package's licence information."),
                        );
                        super::a11y::explain(
                            ui.checkbox(&mut d.commercial_use, t("Business use allowed")),
                            &t("Tick if the program's licence allows it to be used in a business; recorded in the package's licence information."),
                        );
                        super::a11y::explain(
                            ui.checkbox(&mut d.shareable, t("May be shared")),
                            &t("Tick if the program's licence allows giving copies to other people; recorded in the package's licence information."),
                        );
                    });
                    ui.end_row();
                });
            if !d.name.trim().is_empty() {
                small_muted(
                    ui,
                    tf(
                        "Package: {name} ({id})",
                        &[("name", d.display_name()), ("id", d.app_id())],
                    ),
                );
            }
        });
        ui.add_space(8.0);
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("3. Where to install it")).strong());
            let available = self.builder.sandbox == Some(true);
            ui.add_enabled_ui(available, |ui| {
                let sandbox_why = if available {
                    t("The installer runs inside Windows Sandbox, a throwaway copy of Windows, and the package is collected from there. This PC's files and registry are never touched.")
                } else {
                    t("Unavailable: Windows Sandbox isn't turned on (see below).")
                };
                super::a11y::explain(
                    ui.radio_value(
                        &mut self.builder.use_sandbox,
                        true,
                        t("In Windows Sandbox (recommended: this PC is never touched)"),
                    ),
                    &sandbox_why,
                );
            });
            if !available {
                small_muted(ui, t("Windows Sandbox isn't turned on. It's part of Windows 11 Pro, Enterprise and Education: turn on \"Windows Sandbox\" in Windows Features and restart."));
                self.builder.use_sandbox = false;
            }
            super::a11y::explain(
                ui.radio_value(&mut self.builder.use_sandbox, false, t("On this PC (SetupSweep records the installation so you can remove it precisely afterwards)")),
                &t("The installer runs on this PC while SetupSweep records the installation, so the program can be removed precisely afterwards. Starting needs Administrator rights."),
            );
            ui.add_space(4.0);
            super::a11y::explain(
                ui.checkbox(
                    &mut self.builder.include_defaults,
                    t("Use the settings from the first run as the package's default settings"),
                ),
                &t("Keeps the settings the program has after its first run in the capture as the settings the package starts with."),
            );
            small_muted(ui, t("SetupSweep first watches Windows for 30 seconds to learn what changes on its own, so it can leave that out."));
        });
        ui.add_space(8.0);
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("4. Package")).strong());
            ui.horizontal(|ui| {
                let save_label = ui.label(t("Save to"));
                super::a11y::explain(
                    ui.add(egui::TextEdit::singleline(&mut self.builder.out_dir).desired_width(380.0))
                        .labelled_by(save_label.id),
                    &t("The folder the finished package is saved into, as its own folder with the launcher and installer."),
                );
                if super::a11y::explain(
                    ui.button(t("Browse…")),
                    &t("Choose the folder to save the package into."),
                )
                .clicked()
                {
                    if let Some(p) = pick_folder(&t("Choose where to save the package")) {
                        self.builder.out_dir = p.to_string_lossy().to_string();
                    }
                }
            });
            self.ui_builder_tools(ui, &ctx);
        });
        ui.add_space(10.0);
        let ok = PathBuf::from(&self.builder.installer).is_file()
            && !self.builder.details.name.trim().is_empty()
            && !self.builder.out_dir.trim().is_empty();
        let start_why = if !ok {
            t("Unavailable: choose an installer file above, give the program a name, and choose where to save the package first.")
        } else if !self.builder.use_sandbox && !self.elevated {
            t("Unavailable: installing on this PC needs Administrator rights. Run SetupSweep as Administrator, or choose Windows Sandbox above.")
        } else if self.builder.use_sandbox {
            t("Starts the capture: the installer runs in Windows Sandbox, SetupSweep records what it installs, and the package files are prepared in the Save to folder. Review them before compiling with the official tools.")
        } else {
            t("Starts the capture: the installer runs on this PC while SetupSweep records it, and the package files are prepared in the Save to folder. Review them before compiling with the official tools.")
        };
        ui.add_enabled_ui(ok && (self.builder.use_sandbox || self.elevated), |ui| {
            if super::a11y::explain(
                theme::primary_button(ui, format!("📦 {}", t("Start"))),
                &start_why,
            )
            .clicked()
            {
                self.start_capture(&ctx);
            }
        });
        if !self.builder.use_sandbox && !self.elevated {
            small_muted(ui, t("Installing on this PC needs Administrator rights."));
        }
    }

    fn ui_builder_tools(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let tools = self.builder.tools.clone();
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("✔").color(theme::OK));
            ui.label(t(
                "PortableApps.com Launcher Generator: official tool required",
            ));
            small_muted(
                ui,
                if tools
                    .as_ref()
                    .and_then(|t| t.launcher_generator.as_ref())
                    .is_some()
                {
                    t("found")
                } else {
                    t("install the official Launcher and choose its PortableApps folder below")
                },
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(if tools.as_ref().and_then(|t|t.installer.as_ref()).is_some(){"✔"}else{"!"}).color(theme::MUTED));
            match tools.as_ref().and_then(|t| t.installer.clone()) {
                Some(p) => {
                    ui.label(t("Installer: the official PortableApps.com Installer"));
                    small_muted(ui, p.display().to_string());
                }
                None => {
                    ui.label(t("Official PortableApps.com Installer: not found"));
                    small_muted(ui, t("Install the official PortableApps.com Installer and choose its PortableApps folder below. A missing official tool stops compilation; no replacement installer is selected."));
                }
            }
        });
        ui.horizontal(|ui| {
            let folder_label = ui.label(t("PortableApps folder"));
            let mut dir = self.settings.portableapps_dir.clone();
            let r = super::a11y::explain(
                ui.add(
                    egui::TextEdit::singleline(&mut dir)
                        .hint_text(t("Found automatically on any drive"))
                        .desired_width(300.0),
                )
                .labelled_by(folder_label.id),
                &t("The folder your PortableApps platform is in; the official PortableApps.com Installer is looked for there. Found automatically on any drive when this is empty."),
            );
            let mut changed = r.lost_focus() && dir != self.settings.portableapps_dir;
            if super::a11y::explain(
                ui.button(t("Browse…")),
                &t("Choose your PortableApps folder."),
            )
            .clicked()
            {
                if let Some(p) = pick_folder(&t("Choose your PortableApps folder")) {
                    dir = p.to_string_lossy().to_string();
                    changed = true;
                }
            }
            self.settings.portableapps_dir = dir;
            if changed {
                self.save_settings();
                self.refresh_tools(ctx);
            }
        });
    }

    fn ui_builder_running(&mut self, ui: &mut egui::Ui) {
        let stage = self.builder.stage.lock().unwrap().clone();
        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                spinner(ui);
                ui.label(RichText::new(t(&stage)).strong());
            });
            if let Some(job) = &self.builder.compile_job {
                let stopping = job.progress.cancelled();
                small_muted(ui, t("Stopping a build keeps the captured package files. An incomplete executable may remain; rebuild before using it."));
                if super::a11y::explain(
                    ui.add_enabled(!stopping, egui::Button::new(if stopping { t("Stopping build…") } else { t("Cancel build") })),
                    &t("Stops the compiler and its child processes on Windows. Captured application files are kept. File preparation may take a moment to stop; rebuild before using any output."),
                ).clicked() { job.cancel(); }
            }
            if let Some(j) = &self.builder.capture_job {
                let p = j.progress.phase();
                if !p.is_empty() && p != "Done" {
                    small_muted(ui, p);
                }
            }
            if self.builder.work_dir.is_some() {
                ui.add_space(4.0);
                small_muted(ui, t("Follow the steps in the Windows Sandbox window. When the program is installed, start it once, set it up, close it and click OK in the sandbox. The sandbox closes by itself when the package is ready."));
                if super::a11y::explain(
                    ui.button(t("Stop waiting")),
                    &t("Stops waiting for the sandbox. No package is made; close the Windows Sandbox window to throw the capture away."),
                )
                .clicked()
                {
                    self.builder.work_dir = None;
                    self.builder.error = Some(t(
                        "Stopped. Close the Windows Sandbox window to discard it.",
                    ));
                }
            }
            if self.builder.waiting_for_user.load(Ordering::SeqCst) {
                ui.add_space(6.0);
                ui.label(t("When the installation has finished, start the program once, set it up the way you want it to start, and close it."));
                if super::a11y::explain(
                    theme::primary_button(ui, t("Done: take the second snapshot")),
                    &t("Tells SetupSweep the installation is finished and the program is set up the way you want, so it takes the second snapshot and builds the package."),
                )
                .clicked()
                {
                    self.builder.host_done.store(true, Ordering::SeqCst);
                }
            }
        });
    }

    fn ui_builder_result(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let Some((dir, pkg)) = self.builder.package.clone() else {
            return;
        };
        theme::card(ui, |ui| {
            ui.label(
                RichText::new(if self.builder.compiled.is_some() {
                    t("Build completed; portability testing is still required")
                } else {
                    t("Captured files are ready for review")
                })
                .strong()
                .color(theme::OK),
            );
            field(ui, "Folder", &dir.display().to_string());
            field(ui, "Program", &pkg.main_exe);
            field(
                ui,
                "Program files",
                &format!("{} ({})", pkg.files, human_size(pkg.bytes)),
            );
            if let Some(c) = &self.builder.compiled {
                if let Some(l) = &c.launcher {
                    field(ui, "Launcher", &l.display().to_string());
                }
                if let Some(p) = &c.paf {
                    field(ui, "Installer", &p.display().to_string());
                }
            }
            for w in &pkg.warnings {
                small_muted(ui, format!("⚠ {w}"));
            }
            ui.add_space(6.0);
            ui.label(RichText::new(t("Portability check")).strong());
            for i in &pkg.portability.issues {
                use setupsweep_core::portability::Severity;
                let (icon, color) = match i.severity {
                    Severity::Problem => ("✖", theme::BAD),
                    Severity::Warning => ("⚠", theme::WARN),
                    Severity::Info => ("ℹ", theme::MUTED),
                };
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(icon).color(color));
                    ui.label(RichText::new(t(&i.title)).color(color).strong());
                });
                small_muted(ui, t(&i.detail));
            }
            ui.add_space(6.0);
            self.ui_builder_tools(ui, &ctx);
            ui.label(RichText::new(t("Build command review")).strong());
            small_muted(ui, t("Review the official tool windows. This capture workflow does not retain compiler-log copies; the guided PAF workflow does."));
            if let Some(tools) = &self.builder.tools {
                for (label, tool) in [
                    ("Launcher Generator", &tools.launcher_generator),
                    ("Installer", &tools.installer),
                ] {
                    if let Some(tool) = tool {
                        field(ui, label, &tool.display().to_string());
                        field(ui, "Argument (package folder)", &dir.display().to_string());
                        if let Some(parent) = tool.parent() {
                            field(ui, "Working directory", &parent.display().to_string());
                        }
                    }
                }
            }
            field(
                ui,
                "Launcher output",
                &dir.join(format!("{}.exe", pkg.app_id))
                    .display()
                    .to_string(),
            );
            small_muted(ui,format!("Installer output is checked in {} and {} for one new/updated {}_*.paf.exe. Official tool windows may appear. No shell or automatic elevation is used.",dir.parent().unwrap_or(&dir).display(),dir.display(),pkg.app_id));
            if pafbuild::official_custom_code(&dir)
                .iter()
                .any(|p| p.exists())
            {
                theme::warning_banner(ui,t("Custom NSIS code is present. It can run commands during compilation or installation. Review the source before allowing this build."));
                for path in pafbuild::official_custom_code(&dir)
                    .iter()
                    .filter(|p| p.exists())
                {
                    field(ui, "Custom code", &path.display().to_string());
                }
                super::a11y::explain(ui.checkbox(&mut self.builder.allow_custom_code,t("Allow the reviewed custom code for this build")),&t("Explicitly allow custom NSIS source to run as part of this build. This permission resets after each attempt. Presence checks are not a complete audit of scripts or includes."));
            }
            ui.horizontal_wrapped(|ui| {
                if super::a11y::explain(
                    ui.button(t("Open folder")),
                    &t("Opens the prepared package folder in File Explorer."),
                )
                .clicked()
                {
                    platform::shell_open(&dir.to_string_lossy());
                }
                if let Some(compactor) = self
                    .builder
                    .tools
                    .as_ref()
                    .and_then(|tools| tools.app_compactor.as_ref())
                {
                    if super::a11y::explain(
                        ui.button(t("Open AppCompactor")),
                        &t("Launches AppCompactor; select this package there and test it after compression."),
                    )
                    .clicked()
                    {
                        platform::shell_open(&compactor.to_string_lossy());
                    }
                }
                let launcher_ini = dir.join("App").join("AppInfo").join("Launcher").join(format!("{}.ini", pkg.app_id));
                if super::a11y::explain(
                    ui.button(t("Edit Launcher settings")),
                    &t("Opens the package's Launcher settings file so you can edit it. Afterwards, use Compile to rebuild the launcher and installer with your changes."),
                )
                .clicked()
                {
                    platform::shell_open(&launcher_ini.to_string_lossy());
                }
                if super::a11y::explain(
                    ui.add(egui::Button::new(if self.builder.compiled.is_some() { t("Compile again") } else { t("Compile") })),
                    &t("Run the official Launcher Generator and Installer shown above, passing the package folder as an argument. Review their messages and test the resulting package. Custom code needs explicit permission for each attempt."),
                )
                .clicked()
                {
                    self.start_compile(&ctx);
                }
                if let Some(log) = self.builder.log.clone() {
                    if self.builder.on_this_pc && super::a11y::explain(
                        ui.button(tf("Remove {name} from this PC", &[("name", log.name.clone())])),
                        &t("Opens the removal review for the program captured on this PC, using its recorded installation. Nothing is removed until you confirm there."),
                    )
                    .clicked() {
                        self.uninstall_from_log(&log);
                    }
                }
                if super::a11y::explain(
                    ui.button(t("Build another")),
                    &t("Clears this result and goes back to the beginning to build another package. The finished package stays in its folder."),
                )
                .clicked()
                {
                    self.builder.package = None;
                    self.builder.compiled = None;
                    self.builder.log = None;
                    self.builder.error = None;
                    self.builder.details = AppDetails { packager: self.builder.details.packager.clone(), ..Default::default() };
                    self.builder.installer.clear();
                    self.builder.kind = None;
                }
            });
        });
        ui.add_space(8.0);
        let launcher_hdr = egui::CollapsingHeader::new(t("Launcher settings"))
            .default_open(true)
            .show(ui, |ui| {
                let mut text = pkg.launcher_ini.clone();
                super::a11y::explain(
                    ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .desired_width(f32::INFINITY)
                            .interactive(true),
                    ),
                    &t("The Launcher settings generated for this package, shown for reading. To change them, use Edit Launcher settings above, then Compile."),
                );
            });
        super::a11y::explain(
            launcher_hdr.header_response,
            &t("Show or hide the Launcher settings generated for this package."),
        );
    }
}
