//! PortableApps Forge page: capture an installation (in Windows Sandbox or
//! on this PC) and turn it into a PortableApps.com package.

use super::app::App;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use setupscope_core::i18n::{t, tf};
use setupscope_core::model::{human_size, InstallLog};
use setupscope_core::pafbuild::{self, AppDetails, Compiled, InstallerKind, PaTools, Package};
use setupscope_core::platform;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

type CaptureInputs = (String, AppDetails, bool, bool, bool, String, String);

#[derive(Default)]
pub struct BuilderState {
    pub installer: String,
    kind: Option<InstallerKind>,
    pub details: AppDetails,
    silent: bool,
    include_defaults: bool,
    use_sandbox: bool,
    sandbox: Option<bool>,
    out_dir: String,
    tools: Option<PaTools>,
    /// The built-in PortableApps.com Launcher source is already on this PC.
    toolchain_ready: bool,
    tools_job: Option<Job<(PaTools, bool)>>,
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
    log_save_job: Option<Job<Result<(), String>>>,
    log_save_error: Option<String>,
    on_this_pc: bool,
    compile_job: Option<Job<Result<Compiled, String>>>,
    compiled: Option<Compiled>,
    error: Option<String>,
    initialized: bool,
    capture_reviewed: bool,
    review_inputs: Option<CaptureInputs>,
}

impl BuilderState {
    fn review_selection(&mut self, tools_dir: &str) {
        let selection = (self.installer.clone(), self.details.clone(), self.silent,
            self.include_defaults, self.use_sandbox, self.out_dir.clone(), tools_dir.to_owned());
        if self.review_inputs.as_ref() != Some(&selection) { self.capture_reviewed = false; }
        self.review_inputs = Some(selection);
    }
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
        let root = self.store.root.clone();
        self.builder.tools_job = Some(Job::spawn(ctx, move |_| {
            (
                pafbuild::find_tools(&dir),
                setupscope_core::patools::find(&root).is_some(),
            )
        }));
    }

    pub fn poll_builder(&mut self, ctx: &egui::Context) {
        let b = &mut self.builder;
        if let Some(result) = b.log_save_job.as_ref().and_then(|j| j.poll()) {
            b.log_save_job = None;
            b.log_save_error = result.err();
        }
        if let Some((tools, ready)) = b.tools_job.as_ref().and_then(|j| j.poll()) {
            b.tools_job = None;
            b.tools = Some(tools);
            b.toolchain_ready = ready;
        }
        if let Some(r) = b.capture_job.as_ref().and_then(|j| j.poll()) {
            b.capture_job = None;
            match r {
                Ok((log, pkg)) => {
                    self.builder.log_save_error = self.store.save_log(&log).err().map(|e| e.to_string());
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
                    self.builder.toolchain_ready = setupscope_core::patools::find(&self.store.root).is_some();
                    self.builder.compiled = Some(c);
                }
                Err(e) => self.builder.error = Some(e),
            }
        }
    }

    /// Make the launcher and installer straight away.
    fn after_capture(&mut self, ctx: &egui::Context) {
        self.start_compile(ctx);
    }

    fn start_compile(&mut self, ctx: &egui::Context) {
        let Some((dir, pkg)) = self.builder.package.clone() else {
            return;
        };
        let tools = self.builder.tools.clone().unwrap_or_default();
        self.builder.error = None;
        self.builder.compiled = None;
        let stage = self.builder.stage.clone();
        let root = self.store.root.clone();
        self.builder.compile_job = Some(Job::spawn(ctx, move |_| {
            let r = pafbuild::compile(&dir, &pkg.app_id, &tools, &root, &|p| {
                *stage.lock().unwrap() = p.to_string()
            })
            .map_err(|e| e.to_string());
            r
        }));
    }

    fn start_capture(&mut self, ctx: &egui::Context) {
        if self.builder.busy() || !self.builder.capture_reviewed { return; }
        self.builder.capture_reviewed = false;
        self.settings.packager_name = self.builder.details.packager.clone();
        self.settings.portable_out_dir = self.builder.out_dir.clone();
        self.save_settings();
        let b = &mut self.builder;
        b.error = None;
        b.log_save_error = None;
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
                .join(setupscope_core::util::new_id());
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
                        if let Some(error) = self.builder.log_save_error.clone() {
                            theme::warning_banner(ui, format!("The capture recovery log was not saved: {error}. In-memory evidence is retained; keep this session open and retry before relying on saved recovery."));
                            if super::a11y::explain(ui.add_enabled(self.builder.log_save_job.is_none(), egui::Button::new("Retry Saving Recovery Log")), "Write the retained capture log into SetupScope data again. This does not rerun the installer or packaging.").clicked() {
                                if let Some(log) = self.builder.log.clone() {
                                    let store = self.store.clone();
                                    self.builder.log_save_job = Some(Job::spawn(ctx, move |_| store.save_log(&log).map_err(|e| e.to_string())));
                                }
                            }
                        }
                        if self.builder.log_save_job.is_some() { ui.spinner(); ctx.request_repaint_after(std::time::Duration::from_millis(150)); }
                        if self.builder.busy() {
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
                    t("The installer runs in a separate Windows Sandbox guest. Explicitly shared host folders and package output remain accessible; this is not a guarantee against all host changes.")
                } else {
                    t("Unavailable: Windows Sandbox isn't turned on (see below).")
                };
                super::a11y::explain(
                    ui.radio_value(
                        &mut self.builder.use_sandbox,
                        true,
                        t("In Windows Sandbox (separate guest; shared host folders remain accessible)"),
                    ),
                    &sandbox_why,
                );
            });
            if !available {
                small_muted(ui, t("Windows Sandbox isn't turned on. It's part of Windows 11 Pro, Enterprise and Education: turn on \"Windows Sandbox\" in Windows Features and restart."));
                self.builder.use_sandbox = false;
            }
            super::a11y::explain(
                ui.radio_value(&mut self.builder.use_sandbox, false, t("On this PC (records observed changes; recovery is limited)")),
                &t("The installer runs on this PC while SetupScope records the installation, so recorded changes can be reviewed afterwards; unrecorded changes are outside recovery. Starting needs Administrator rights."),
            );
            ui.add_space(4.0);
            super::a11y::explain(
                ui.checkbox(
                    &mut self.builder.include_defaults,
                    t("Use the settings from the first run as the package's default settings"),
                ),
                &t("Keeps the settings the program has after its first run in the capture as the settings the package starts with."),
            );
            small_muted(ui, t("SetupScope first watches Windows for 30 seconds to learn what changes on its own, so it can leave that out."));
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
            t("Unavailable: installing on this PC needs Administrator rights. Run SetupScope as Administrator, or choose Windows Sandbox above.")
        } else if self.builder.use_sandbox {
            t("Starts the capture: the installer runs in Windows Sandbox, SetupScope records what it installs, and the PortableApps.com package is built into the Save to folder.")
        } else {
            t("Starts the capture: the installer runs on this PC while SetupScope records it, and the PortableApps.com package is built into the Save to folder.")
        };
        self.builder.review_selection(&self.settings.portableapps_dir);
        theme::warning_banner(ui, if self.builder.use_sandbox {
            "This runs the selected installer in a separate guest with shared host folders and writes a package. Verify the source and redistribution rights. Shared folders remain exposed."
        } else {
            "This runs the selected installer on this PC with SetupScope's privileges. Capture is incomplete evidence, not isolation or guaranteed undo. Use a disposable test machine for untrusted installers."
        });
        super::a11y::explain(ui.checkbox(&mut self.builder.capture_reviewed, t("I reviewed the installer, execution location, package output and toolchain requirements")),
            "Required before capture. Packaging may download the official launcher source and build tools on first use; vendor installers may access the network. Configuration edits clear review. No action starts from this checkbox.");
        ui.add_enabled_ui(ok && self.builder.capture_reviewed && (self.builder.use_sandbox || self.elevated), |ui| {
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
            ui.label(t("PortableApps.com Launcher toolchain"));
            small_muted(
                ui,
                if self.builder.toolchain_ready {
                    t("ready")
                } else {
                    t("the official source is downloaded from GitHub the first time (about 6 MB)")
                },
            );
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("✔").color(theme::OK));
            match tools.as_ref().and_then(|t| t.installer.clone()) {
                Some(p) => {
                    ui.label(t("Installer: the official PortableApps.com Installer"));
                    small_muted(ui, p.display().to_string());
                }
                None => {
                    ui.label(t("Installer: SetupScope-generated fallback"));
                    small_muted(ui, t("to publish on portableapps.com, install the official PortableApps.com Installer and SetupScope uses it instead"));
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
                    &t("Tells SetupScope the installation is finished and the program is set up the way you want, so it takes the second snapshot and builds the package."),
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
                RichText::new(format!("✔ {}", t("The package is ready")))
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
                use setupscope_core::portability::Severity;
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
            ui.horizontal_wrapped(|ui| {
                if super::a11y::explain(
                    ui.button(t("Open folder")),
                    &t("Opens the finished package's folder in File Explorer."),
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
                    &t("Make the launcher and the .paf.exe installer again, for example after editing the Launcher settings."),
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
                    ui.add_enabled(self.builder.log_save_job.is_none() && self.builder.log_save_error.is_none(), egui::Button::new(t("Build another"))),
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
                super::a11y::explain(
                    ui.add(egui::Label::new(RichText::new(&pkg.launcher_ini).monospace()).selectable(true)),
                    &t("The Launcher settings generated for this package, shown for reading. To change them, use Edit Launcher settings above, then Compile."),
                );
            });
        super::a11y::explain(
            launcher_hdr.header_response,
            &t("Show or hide the Launcher settings generated for this package."),
        );
    }
}

#[cfg(test)]
mod audit_tests {
    use super::*;
    #[test]
    fn execution_review_invalidates_when_inputs_or_toolchain_change() {
        let mut state = BuilderState::default();
        state.review_selection("tools-a");
        state.capture_reviewed = true;
        state.review_selection("tools-a");
        assert!(state.capture_reviewed);
        state.use_sandbox = !state.use_sandbox;
        state.review_selection("tools-a");
        assert!(!state.capture_reviewed);
        state.capture_reviewed = true;
        state.review_selection("tools-b");
        assert!(!state.capture_reviewed);
        state.capture_reviewed = true;
        state.out_dir = "changed-output".into();
        state.review_selection("tools-b");
        assert!(!state.capture_reviewed);
    }
}
