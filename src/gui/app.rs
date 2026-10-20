//! Application shell: menus, tabs, dialogs, and per-frame polling.

use super::theme;
use super::widgets::*;
use super::*;
use egui::RichText;
use installscope_core::i18n::{self, t, tf};
use installscope_core::platform::{self, Registry};
use installscope_core::settings::Settings;
use installscope_core::store::Store;
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Dashboard,
    Programs,
    Monitor,
    Logs,
    Backups,
    Startup,
    Cleaner,
    Restore,
    Lab,
    Builder,
    Shell,
    Debloat,
    ExtInv,
    Settings,
}

/// Actions that need a yes/no confirmation first.
#[derive(Clone, Debug)]
pub enum Confirm {
    DeleteLog(String),
    RestoreBackup(String),
    DeleteBackup(String),
    CleanJunk,
    DeleteStartup(usize),
    DeleteTask(usize),
    /// Disable a browser extension (reversibly, nothing is deleted).
    DisableExtension(String),
    /// Restore a browser extension disabled through InstallScope.
    RestoreExtension(String),
    ResetSettings,
    CancelMonitoring,
    /// Turn an optional Windows feature on (true) or off.
    /// Run one deep (Windows) cleanup.
    DeepClean(installscope_core::slimmer::DeepItem),
    /// Apply the ticked Anti-Telemetry changes.
    /// Remove the ticked pre-installed apps.
    SetProtection(bool),
    RestoreTo(u32),
    DeleteRestorePoint(u32),
}

impl Confirm {
    /// Actions that always ask, even when the user turned confirmations off
    /// in Advanced mode: restoring the whole PC (with a restart), deleting a
    /// backup, and deep cleaning are hard or impossible to undo, so the
    /// "Ask before deleting things" setting must not silence them.
    pub fn always_confirm(&self) -> bool {
        matches!(
            self,
            Confirm::RestoreTo(_) | Confirm::DeleteBackup(_) | Confirm::DeepClean(_)
        )
    }

    /// Actions whose confirm button must not start focused in the dialog,
    /// so a stray Enter cannot run them: anything that deletes, restores,
    /// restarts, cleans or resets. The dialog starts on Cancel for these;
    /// benign actions keep starting on the confirm button.
    pub fn is_destructive(&self) -> bool {
        matches!(
            self,
            Confirm::DeleteLog(_)
                | Confirm::RestoreBackup(_)
                | Confirm::DeleteBackup(_)
                | Confirm::CleanJunk
                | Confirm::DeleteStartup(_)
                | Confirm::DeleteTask(_)
                | Confirm::ResetSettings
                | Confirm::DeepClean(_)
                | Confirm::RestoreTo(_)
                | Confirm::DeleteRestorePoint(_)
                // Turning System Protection off makes Windows delete every
                // existing restore point; turning it on is harmless.
                | Confirm::SetProtection(false)
        )
    }
}

pub struct ConfirmDialog {
    pub title: String,
    pub body: String,
    pub ok: String,
    pub action: Confirm,
}

pub struct App {
    pub ctx: egui::Context,
    pub logo: egui::TextureHandle,
    pub logo_small: egui::TextureHandle,
    pub reg: Arc<dyn Registry>,
    pub store: Store,
    pub settings: Settings,
    pub tab: Tab,
    pub status: String,
    pub elevated: bool,
    pub admin_prompt: bool,
    pub about_open: bool,
    pub style_frames: u32,
    pub frame: u64,
    /// The page shown last frame and when the current one appeared, for
    /// the short fade between pages.
    pub shown_tab: Option<Tab>,
    pub tab_since: f64,
    pub confirm: Option<ConfirmDialog>,
    pub languages: Vec<(String, String)>,
    pub programs: programs::ProgramsState,
    pub restore: super::restore_ui::RestoreState,
    pub builder: super::builder::BuilderState,
    pub shell: super::shell_ui::ShellState,
    pub debloat: super::debloat_ui::DebloatState,
    pub extinv: super::extinv_ui::ExtInvState,
    pub alt: super::alt_ui::AltState,
    /// "Ask InstallScope" plain-language search (Ctrl+K).
    pub ask_ui: super::ask::AskState,
    /// Live activity while monitoring an installation.
    pub live: Option<installscope_core::live::LiveWatcher>,
    /// Whether background monitoring is on (read from Task Scheduler once).
    pub auto_watch: Option<bool>,
    pub auto_watch_job: Option<super::Job<bool>>,
    /// Transfer export or restore window.
    pub transfer: Option<super::transfer_ui::TransferUi>,
    pub monitor: monitor::MonitorState,
    pub logs: logs::LogsState,
    pub backups: backups::BackupsState,
    /// A file operation (deleting a backup) running in the background.
    pub file_job: Option<Job<Result<(), String>>>,
    /// System-wide scan for registrations left by removed programs.
    pub orphan_job: Option<Job<installscope_core::revert::Plan>>,
    pub startup: startup_tab::StartupState,
    pub cleaner: cleaner::CleanerState,
    pub settings_ui: settings_tab::SettingsState,
    pub lab: lab::LabState,
    pub help_open: bool,
    /// F10 was pressed: move keyboard focus to the menu bar.
    pub focus_menu: bool,
    /// Ctrl+Tab (false) or Ctrl+Shift+Tab (true), taken out of the input
    /// before egui sees it, so it switches pages instead of moving focus.
    pub ctrl_tab: Option<bool>,
    pub welcome_open: bool,
    pub focus_search: bool,
    /// Installer passed from Explorer's "Install with InstallScope".
    pub pending_install: Option<String>,
    /// Arguments to pass on when relaunching as Administrator.
    pub launch_args: Vec<String>,
    pub wizard: Option<programs::Wizard>,
    pub removal: Option<plan::RemovalFlow>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        install_target: Option<String>,
        launch_args: Vec<String>,
    ) -> Self {
        let store = Store::open().unwrap_or_else(|_| {
            Store::at(&std::env::temp_dir().join("InstallScope")).expect("data folder")
        });
        let settings = store.load_settings();
        let first_run_done = settings.first_run_done;
        theme::apply(&cc.egui_ctx, settings.high_contrast);
        i18n::set_language(&settings.language);
        cc.egui_ctx.set_zoom_factor(settings.zoom.clamp(0.75, 2.0));
        let reg: Arc<dyn Registry> = Arc::from(platform::system_registry());
        let elevated = platform::is_elevated();
        let languages = i18n::languages()
            .into_iter()
            .map(|l| (l.code, l.name))
            .collect();
        let mut app = App {
            ctx: cc.egui_ctx.clone(),
            logo: cc
                .egui_ctx
                .load_texture("logo", logo_image(), egui::TextureOptions::LINEAR),
            logo_small: cc.egui_ctx.load_texture(
                "logo_small",
                logo_small_image(),
                egui::TextureOptions::LINEAR,
            ),
            reg,
            store,
            settings,
            tab: Tab::Dashboard,
            status: String::new(),
            elevated,
            admin_prompt: !elevated && first_run_done,
            about_open: false,
            style_frames: 30,
            frame: 0,
            confirm: None,
            languages,
            programs: Default::default(),
            restore: Default::default(),
            builder: Default::default(),
            shell: Default::default(),
            debloat: Default::default(),
            extinv: Default::default(),
            shown_tab: None,
            tab_since: 0.0,
            alt: Default::default(),
            ask_ui: Default::default(),
            live: None,
            auto_watch: None,
            auto_watch_job: None,
            transfer: None,
            monitor: Default::default(),
            logs: Default::default(),
            backups: Default::default(),
            file_job: None,
            orphan_job: None,
            startup: Default::default(),
            cleaner: Default::default(),
            settings_ui: Default::default(),
            lab: Default::default(),
            help_open: false,
            focus_menu: false,
            ctrl_tab: None,
            welcome_open: false,
            focus_search: false,
            pending_install: install_target,
            launch_args,
            wizard: None,
            removal: None,
        };
        app.programs.cache = std::sync::Arc::new(std::sync::Mutex::new(
            installscope_core::cache::Cache::load(&app.store.cache_path()),
        ));
        app.reload_logs();
        app.reload_backups();
        app.refresh_programs(&cc.egui_ctx);
        app.monitor.resume_pending(&app.store);
        app.welcome_open = !app.settings.first_run_done;
        app
    }

    pub fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }

    pub fn save_settings(&mut self) {
        if let Err(e) = self.store.save_settings(&self.settings) {
            self.set_status(tf(
                "Could not save settings: {error}",
                &[("error", e.to_string())],
            ));
        }
    }

    /// Re-read install logs from disk in the background.
    pub fn reload_logs(&mut self) {
        let store = self.store.clone();
        self.logs
            .loader
            .request(&self.ctx, move || store.list_logs());
    }

    /// Re-read backups from disk in the background.
    pub fn reload_backups(&mut self) {
        let dir = self.store.backups_dir();
        self.backups.loader.request(&self.ctx, move || {
            installscope_core::backup::list_backups(&dir)
        });
    }

    /// Start the system-wide orphan scan.
    pub fn scan_orphans(&mut self) {
        if self.orphan_job.is_some() {
            return;
        }
        let reg = self.reg.clone();
        self.set_status(t("Looking for leftovers of removed programs…"));
        self.orphan_job = Some(Job::spawn(&self.ctx, move |_| {
            installscope_core::orphans::scan_system(reg.as_ref())
        }));
    }

    fn poll_orphans(&mut self) {
        let Some(plan) = self.orphan_job.as_ref().and_then(|j| j.poll()) else {
            return;
        };
        self.orphan_job = None;
        if plan.items.is_empty() {
            self.set_status(t("No leftovers of removed programs found."));
            return;
        }
        let n = plan.items.len();
        self.set_status(tf(
            "Found {n} leftovers of removed programs.",
            &[("n", n.to_string())],
        ));
        let mut flow =
            super::plan::RemovalFlow::new(t("Leftovers of removed programs"), plan, false, false);
        flow.plan.info.push(t(
            "Uninstall entries are included only when both their declared install folder and uninstaller are missing. MSI, hidden, protected, and Windows-owned entries are excluded. Review every item before removal; selected registry data is backed up.",
        ));
        self.open_removal(flow);
    }

    fn poll_lists(&mut self) {
        if let Some((list, again)) = self.logs.loader.poll() {
            self.logs.stats = list
                .iter()
                .map(|l| (l.id.clone(), l.changes.stats()))
                .collect();
            self.logs.logs = list;
            if again {
                self.reload_logs();
            }
        }
        if let Some((list, again)) = self.backups.loader.poll() {
            self.backups.list = list;
            if again {
                self.reload_backups();
            }
        }
        if let Some(r) = self.file_job.as_ref().and_then(|j| j.poll()) {
            self.file_job = None;
            if let Err(e) = r {
                self.set_status(e);
            }
            self.reload_logs();
            self.reload_backups();
        }
    }

    pub fn ask(&mut self, title: &str, body: String, ok: &str, action: Confirm) {
        if !self.settings.must_confirm(action.always_confirm()) {
            self.run_confirmed(action);
            return;
        }
        self.confirm = Some(ConfirmDialog {
            title: t(title),
            body,
            ok: t(ok),
            action,
        });
    }

    fn run_confirmed(&mut self, action: Confirm) {
        match action {
            Confirm::DeleteLog(id) => self.delete_log(&id),
            Confirm::RestoreBackup(id) => self.restore_backup(&id),
            Confirm::DeleteBackup(id) => self.delete_backup(&id),
            Confirm::CleanJunk => self.start_clean(),
            Confirm::DeepClean(item) => self.start_deep(item),
            Confirm::SetProtection(on) => self.set_protection(on),
            Confirm::RestoreTo(seq) => self.restore_to_point(seq),
            Confirm::DeleteRestorePoint(seq) => self.delete_restore_point(seq),
            Confirm::DeleteStartup(i) => self.delete_startup(i),
            Confirm::DeleteTask(i) => self.delete_task(i),
            Confirm::DisableExtension(key) => self.disable_extension(&key),
            Confirm::RestoreExtension(key) => self.restore_extension(&key),
            Confirm::ResetSettings => {
                let lang = self.settings.language.clone();
                self.settings = Settings {
                    language: lang,
                    ..Settings::default()
                };
                self.settings_ui.load(&self.settings);
                self.save_settings();
            }
            Confirm::CancelMonitoring => {
                self.monitor.cancel(&self.store);
            }
        }
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu")
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ctx, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    let file = ui.menu_button(t("File"), |ui| {
                        if ui.button(t("Restore programs from a package…")).clicked() {
                            ui.close();
                            self.start_transfer_import();
                        }
                        if ui.button(t("Import install log…")).clicked() {
                            ui.close();
                            self.import_log();
                        }
                        if ui.button(t("Export program list (CSV)…")).clicked() {
                            ui.close();
                            self.export_program_list();
                        }
                        ui.separator();
                        if ui.button(t("Open data folder")).clicked() {
                            ui.close();
                            platform::shell_open(&self.store.root.to_string_lossy());
                        }
                        ui.separator();
                        if ui.button(t("Exit")).clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    if std::mem::take(&mut self.focus_menu) {
                        file.response.request_focus();
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new("installscope_keyboard_mode"), true)
                        });
                    }
                    ui.menu_button(t("Utilities"), |ui| {
                        if ui.button(t("Install and monitor…")).clicked() {
                            ui.close();
                            self.tab = Tab::Monitor;
                            self.start_install_and_monitor(ctx);
                        }
                        if ui.button(t("System Restore")).clicked() {
                            ui.close();
                            self.tab = Tab::Restore;
                        }
                        if ui.button(t("PortableApps Forge")).clicked() {
                            ui.close();
                            self.tab = Tab::Builder;
                        }
                        if ui.button(t("Right-click menu")).clicked() {
                            ui.close();
                            self.tab = Tab::Shell;
                        }
                        if ui.button(t("Bloatware Shield")).clicked() {
                            ui.close();
                            self.tab = Tab::Debloat;
                        }
                        if ui.button(t("Junk cleaner")).clicked() {
                            ui.close();
                            self.tab = Tab::Cleaner;
                        }
                        if self.settings.advanced_mode
                            && ui
                                .button(t("Find leftovers of removed programs…"))
                                .clicked()
                        {
                            ui.close();
                            self.scan_orphans();
                        }
                        if ui.button(t("Create restore point")).clicked() {
                            ui.close();
                            self.programs.restore_job = Some(Job::spawn(ctx, |_| {
                                installscope_core::restore::create_restore_point(
                                    "InstallScope manual restore point",
                                )
                                .map_err(|e| e.to_string())
                            }));
                        }
                        ui.separator();
                        ui.add_enabled_ui(!self.elevated, |ui| {
                            if ui.button(t("Restart as Administrator")).clicked() {
                                ui.close();
                                self.relaunch_elevated(ctx);
                            }
                        });
                    });
                    ui.menu_button(t("View"), |ui| {
                        ui.menu_button(t("Language"), |ui| {
                            let mut changed = false;
                            changed |= ui
                                .radio_value(
                                    &mut self.settings.language,
                                    String::new(),
                                    t("Windows default"),
                                )
                                .clicked();
                            for (code, name) in self.languages.clone() {
                                changed |= ui
                                    .radio_value(&mut self.settings.language, code, name)
                                    .clicked();
                            }
                            if changed {
                                i18n::set_language(&self.settings.language);
                                self.save_settings();
                            }
                        });
                        ui.separator();
                        if ui.button(t("Zoom in")).clicked() {
                            self.zoom(ctx, 0.1);
                        }
                        if ui.button(t("Zoom out")).clicked() {
                            self.zoom(ctx, -0.1);
                        }
                        if ui.button(t("Reset zoom")).clicked() {
                            self.settings.zoom = 1.0;
                            ctx.set_zoom_factor(1.0);
                            self.save_settings();
                        }
                    });
                    ui.menu_button(t("Help"), |ui| {
                        if ui.button(t("Project page")).clicked() {
                            ui.close();
                            platform::shell_open(env!("CARGO_PKG_REPOSITORY"));
                        }
                        if ui.button(t("Report a problem")).clicked() {
                            ui.close();
                            platform::shell_open(&format!(
                                "{}/issues",
                                env!("CARGO_PKG_REPOSITORY")
                            ));
                        }
                        ui.separator();
                        if ui.button(t("About InstallScope")).clicked() {
                            ui.close();
                            self.about_open = true;
                        }
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if self.elevated {
                            theme::badge(ui, t("Administrator"), theme::OK);
                        } else if ui
                            .add(
                                egui::Button::new(
                                    RichText::new(format!("🛡 {}", t("Not elevated")))
                                        .color(theme::WARN),
                                )
                                .frame(false),
                            )
                            .on_hover_text(t("Restart as Administrator"))
                            .clicked()
                        {
                            self.admin_prompt = true;
                        }
                    });
                });
            });
    }

    fn zoom(&mut self, ctx: &egui::Context, delta: f32) {
        self.settings.zoom = (self.settings.zoom + delta).clamp(0.75, 2.0);
        ctx.set_zoom_factor(self.settings.zoom);
        self.save_settings();
    }

    pub fn relaunch_elevated(&mut self, ctx: &egui::Context) {
        if platform::relaunch_elevated(&self.launch_args) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else {
            self.set_status(t("Elevation was cancelled."));
        }
    }

    fn nav_sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(214.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin {
                        left: 10,
                        right: 10,
                        top: 14,
                        bottom: 10,
                    })
                    .stroke(egui::Stroke::new(1.0_f32, theme::LINE)),
            )
            .show(ctx, |ui| {
                let footer_height = if self.elevated { 48.0 } else { 90.0 };
                egui::ScrollArea::vertical()
                    .id_salt("nav_items")
                    .auto_shrink([false, false])
                    .max_height((ui.available_height() - footer_height).max(0.0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let tex = self.logo_small.clone();
                            ui.add(
                                egui::Image::new(&tex).fit_to_exact_size(egui::vec2(28.0, 28.0)),
                            );
                            ui.label(
                                RichText::new("InstallScope")
                                    .size(19.0)
                                    .strong()
                                    .color(egui::Color32::WHITE),
                            );
                        });
                        ui.add_space(14.0);
                        let monitoring = self.monitor.is_active();
                        let broken = self.programs.broken.len();
                        let items: [(Tab, &str, &str, Option<String>, &str); 7] = [
                            (
                                Tab::Dashboard,
                                "🏠",
                                "Dashboard",
                                None,
                                "Your system at a glance",
                            ),
                            (
                                Tab::Programs,
                                "📦",
                                "Programs",
                                (broken > 0).then(|| broken.to_string()),
                                "Uninstall programs completely, including everything they leave behind",
                            ),
                            (
                                Tab::Monitor,
                                if monitoring { "⏺" } else { "📡" },
                                "Monitor",
                                monitoring.then(|| "REC".to_string()),
                                "Record installations so they can be removed precisely",
                            ),
                            (
                                Tab::Logs,
                                "📋",
                                "Install Logs",
                                None,
                                "Everything each monitored installation changed",
                            ),
                            (
                                Tab::Backups,
                                "💾",
                                "Backups",
                                None,
                                "Undo any removal, cleanup or startup change",
                            ),
                            (
                                Tab::Startup,
                                "🚀",
                                "Startup",
                                None,
                                "Programs, services and tasks that start with Windows",
                            ),
                            (
                                Tab::Cleaner,
                                "🗑",
                                "Cleaner",
                                None,
                                "Free space taken by temporary files and caches",
                            ),
                        ];
                        for (tab, icon, label, badge, hint) in items {
                            if theme::nav_item_with_hint(
                                ui,
                                icon,
                                &t(label),
                                self.tab == tab,
                                badge,
                                Some(&t(hint)),
                            )
                            .clicked()
                            {
                                self.tab = tab;
                                self.on_tab_opened(ctx, tab);
                            }
                        }
                        ui.add_space(14.0);
                        ui.label(RichText::new(t("Utilities")).small().color(theme::MUTED));
                        ui.add_space(2.0);
                        if theme::nav_item_with_hint(
                            ui,
                            "⏪",
                            &t("System Restore"),
                            self.tab == Tab::Restore,
                            None,
                            Some(&t("Create, delete and roll back to restore points")),
                        )
                        .clicked()
                        {
                            self.tab = Tab::Restore;
                        }
                        if theme::nav_item_with_hint(
                            ui,
                            "📸",
                            &t("Snapshots"),
                            self.tab == Tab::Lab,
                            None,
                            Some(&t(
                                "Capture the system at any moment and compare any two captures",
                            )),
                        )
                        .clicked()
                        {
                            self.tab = Tab::Lab;
                            self.reload_logs();
                            self.on_lab_opened();
                        }
                        if theme::nav_item_with_hint(
                            ui,
                            "🎒",
                            &t("PortableApps Forge"),
                            self.tab == Tab::Builder,
                            None,
                            Some(&t(
                                "Capture an installation and turn it into a PortableApps.com package",
                            )),
                        )
                        .clicked()
                        {
                            self.tab = Tab::Builder;
                        }
                        for (icon, label, hint) in [
                            (
                                "📦",
                                "Setup Package Creator",
                                "Planned: Build a guided installer from a reviewed project, with complete dependency checks.",
                            ),
                            (
                                "🧪",
                                "Developer Sandbox",
                                "Planned: Run development apps in a disposable, restricted environment.",
                            ),
                            (
                                "💿",
                                "Temporary Scratch Disk",
                                "Planned: Create a size-limited temporary workspace for testing and builds.",
                            ),
                            (
                                "🗂",
                                "VSS Viewer",
                                "Planned: Browse VSS snapshots read-only and copy selected files.",
                            ),
                        ] {
                            ui.add_enabled_ui(false, |ui| {
                                theme::nav_item_with_hint(
                                    ui,
                                    icon,
                                    &t(label),
                                    false,
                                    Some(t("Planned")),
                                    Some(&t(hint)),
                                )
                                .on_disabled_hover_text(t(hint));
                            });
                        }
                        if theme::nav_item_with_hint(
                            ui,
                            "🛡",
                            &t("Bloatware Shield"),
                            self.tab == Tab::Debloat,
                            None,
                            Some(&t(
                                "Remove the trials, promotions and pre-installed apps that came with this PC",
                            )),
                        )
                        .clicked()
                        {
                            self.tab = Tab::Debloat;
                        }
                        if theme::nav_item_with_hint(
                            ui,
                            "🧩",
                            &t("Browser extensions"),
                            self.tab == Tab::ExtInv,
                            None,
                            Some(&t(
                                "See every browser extension, with safe disable and restore",
                            )),
                        )
                        .clicked()
                        {
                            self.tab = Tab::ExtInv;
                        }
                        if theme::nav_item_with_hint(
                            ui,
                            "☰",
                            &t("Right-Click Menu"),
                            self.tab == Tab::Shell,
                            None,
                            Some(&t(
                                "Every shell extension and menu command, with broken ones left by removed programs",
                            )),
                        )
                        .clicked()
                        {
                            self.tab = Tab::Shell;
                        }
                    });
                ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
                    if !self.elevated
                        && theme::nav_item_with_hint(
                            ui,
                            "🛡",
                            &t("Run as Administrator"),
                            false,
                            None,
                            Some(&t("Restart as Administrator")),
                        )
                            .clicked()
                    {
                        self.admin_prompt = true;
                    }
                    if theme::nav_item_with_hint(
                        ui,
                        "⚙",
                        &t("Settings"),
                        self.tab == Tab::Settings,
                        None,
                        Some(&t(
                            "Manage language, appearance, monitoring scope, backup protection and advanced behavior",
                        )),
                    )
                        .clicked()
                    {
                        self.tab = Tab::Settings;
                        self.on_tab_opened(ctx, Tab::Settings);
                    }
                });
            });
    }

    fn page_header(&mut self, ctx: &egui::Context) {
        let (title, sub) = match self.tab {
            Tab::Dashboard => ("Dashboard", "Your system at a glance"),
            Tab::Programs => (
                "Programs",
                "Uninstall programs completely, including everything they leave behind",
            ),
            Tab::Monitor => (
                "Monitor",
                "Record installations so they can be removed precisely",
            ),
            Tab::Logs => (
                "Install logs",
                "Everything each monitored installation changed",
            ),
            Tab::Backups => ("Backups", "Undo any removal, cleanup or startup change"),
            Tab::Startup => (
                "Startup",
                "Programs, services and tasks that start with Windows",
            ),
            Tab::Cleaner => ("Cleaner", "Free space taken by temporary files and caches"),
            Tab::Restore => (
                "System Restore",
                "Create, delete and roll back to restore points",
            ),
            Tab::Lab => (
                "Snapshots",
                "Capture the system at any moment and compare any two captures",
            ),
            Tab::Builder => (
                "PortableApps Forge",
                "Capture an installation and turn it into a PortableApps.com package",
            ),
            Tab::Shell => (
                "Right-click menu",
                "Every shell extension and menu command, with broken ones left by removed programs",
            ),
            Tab::Debloat => (
                "Bloatware Shield",
                "Remove the trials, promotions and pre-installed apps that came with this PC",
            ),
            Tab::ExtInv => (
                "Browser extensions",
                "See every browser extension, with safe disable and restore",
            ),
            Tab::Settings => ("Settings", ""),
        };
        let help = page_help(self.tab);
        egui::TopBottomPanel::top("page_header")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin {
                        left: 18,
                        right: 18,
                        top: 10,
                        bottom: 0,
                    }),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| theme::page_title(ui, &t(title), &t(sub)));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                        ui.add_space(4.0);
                        let label = if self.help_open {
                            format!("✖ {}", t("Close help"))
                        } else {
                            format!("❓ {}", t("Help"))
                        };
                        if ui
                            .button(label)
                            .on_hover_text(t("What is this page for?"))
                            .clicked()
                        {
                            self.help_open = !self.help_open;
                        }
                        ui.add_space(4.0);
                        if ui
                            .button(format!("🔎 {}", t("Ask InstallScope")))
                            .on_hover_text(t(
                                "Search programs and install logs in plain words (Ctrl+K)",
                            ))
                            .clicked()
                        {
                            self.open_ask();
                        }
                    });
                });
                if self.help_open {
                    theme::card(ui, |ui| {
                        for line in help {
                            ui.label(t(line));
                        }
                    });
                    ui.add_space(8.0);
                }
            });
    }

    pub fn on_tab_opened(&mut self, ctx: &egui::Context, tab: Tab) {
        match tab {
            Tab::Logs => self.reload_logs(),
            Tab::Backups => self.reload_backups(),
            Tab::Startup if !self.startup.loaded => self.refresh_startup(ctx),
            Tab::Settings => self.settings_ui.load(&self.settings),
            Tab::Lab => self.on_lab_opened(),
            _ => {}
        }
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .frame(
                egui::Frame::new()
                    .fill(theme::SIDEBAR)
                    .inner_margin(egui::Margin::symmetric(10, 4)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    small_muted(ui, &self.status);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        small_muted(
                            ui,
                            format!(
                                "InstallScope {} · © {}",
                                installscope_core::APP_VERSION,
                                installscope_core::APP_AUTHOR
                            ),
                        );
                        if self.store.portable {
                            theme::badge(ui, t("Portable"), theme::MUTED);
                        }
                    });
                });
            });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.admin_prompt {
            let resp = egui::Modal::new(egui::Id::new("admin_prompt")).show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("🛡").size(28.0).color(theme::WARN));
                    ui.heading(t("Administrator rights needed"));
                });
                ui.add_space(6.0);
                ui.label(t(
                    "InstallScope is not running as Administrator. It needs admin rights to work effectively: \
                     to monitor and undo installations system-wide, remove programs installed for all users, \
                     manage services, and clean protected folders.",
                ));
                ui.add_space(4.0);
                small_muted(ui, t("Without them, many changes will fail with \"access denied\"."));
                ui.add_space(10.0);
                let mut close = false;
                ui.horizontal(|ui| {
                    let r = theme::primary_button(ui, format!("🛡 {}", t("Relaunch as Administrator")));
                    if ui.memory(|m| m.focused().is_none()) {
                        r.request_focus();
                    }
                    if r.clicked() {
                        self.relaunch_elevated(ctx);
                        close = true;
                    }
                    if ui.button(t("Continue anyway")).clicked() {
                        close = true;
                    }
                });
                close
            });
            if resp.inner || resp.should_close() {
                self.admin_prompt = false;
            }
        }

        if let Some(c) = &self.confirm {
            let (title, body, ok) = (c.title.clone(), c.body.clone(), c.ok.clone());
            let destructive = c.action.is_destructive();
            let resp = egui::Modal::new(egui::Id::new("confirm")).show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.heading(title);
                ui.add_space(6.0);
                ui.label(body);
                ui.add_space(10.0);
                let mut choice = None;
                ui.horizontal(|ui| {
                    let r = theme::primary_button(ui, ok);
                    let cancel = ui.button(t("Cancel"));
                    // Keyboard users start on the safe button. Destructive
                    // actions start on Cancel, so a stray Enter cannot run
                    // them; Tab still reaches the action button and Enter
                    // works there, and Esc cancels. Benign actions start on
                    // the confirm button: Enter accepts, Tab moves to
                    // Cancel, Esc cancels.
                    if ui.memory(|m| m.focused().is_none()) {
                        if destructive {
                            cancel.request_focus();
                        } else {
                            r.request_focus();
                        }
                    }
                    if r.clicked() {
                        choice = Some(true);
                    }
                    if cancel.clicked() {
                        choice = Some(false);
                    }
                });
                choice
            });
            match resp.inner {
                Some(true) => {
                    let action = self.confirm.take().unwrap().action;
                    self.run_confirmed(action);
                }
                Some(false) => self.confirm = None,
                None if resp.should_close() => self.confirm = None,
                None => {}
            }
        }

        if self.about_open && super::a11y::escape_pressed(ctx) {
            self.about_open = false;
        }
        if self.about_open {
            let logo = self.logo.clone();
            egui::Window::new(t("About InstallScope"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut self.about_open)
                .show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        let tex = logo.clone();
                        ui.add(egui::Image::new(&tex).fit_to_exact_size(egui::vec2(96.0, 96.0)));
                        ui.heading(
                            RichText::new("InstallScope")
                                .color(theme::ACCENT_TEXT)
                                .strong(),
                        );
                        ui.label(tf(
                            "Version {n}",
                            &[("n", installscope_core::APP_VERSION.to_string())],
                        ));
                        ui.add_space(6.0);
                        ui.label(t("Windows Install, Uninstall & Developer Tools"));
                        ui.label(format!("© 2026 {}", installscope_core::APP_AUTHOR));
                        ui.add_space(6.0);
                        small_muted(
                            ui,
                            t("Free software under the GNU General Public License v3."),
                        );
                        ui.hyperlink_to(t("Project page"), env!("CARGO_PKG_REPOSITORY"));
                    });
                });
        }
    }

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        self.poll_lists();
        self.poll_orphans();
        self.poll_programs(ctx);
        self.poll_monitor(ctx);
        self.poll_startup();
        self.poll_cleaner();
        self.poll_learning();
        self.poll_backups();
        self.poll_removal();
        self.poll_lab();
        self.poll_builder(ctx);
        self.poll_shell();
        self.poll_debloat();
        self.poll_extinv();
        self.poll_alternatives();
        self.poll_silent_package();
        self.poll_precheck(ctx);
        // Resource-use history: a quick sample of running programs every
        // five minutes while InstallScope is open (background monitoring does the
        // same when it is on).
        {
            use std::sync::atomic::{AtomicI64, Ordering};
            static LAST: AtomicI64 = AtomicI64::new(0);
            let now = installscope_core::time::unix_now();
            if now - LAST.load(Ordering::Relaxed) >= 300 {
                LAST.store(now, Ordering::Relaxed);
                let root = self.store.root.clone();
                std::thread::spawn(move || {
                    #[cfg(windows)]
                    installscope_core::platform::windows::lower_thread_priority();
                    installscope_core::usage::sample_and_save(&root, 290);
                });
            }
        }
        if let Some(job) = &self.programs.restore_job {
            if let Some(r) = job.poll() {
                self.programs.restore_job = None;
                self.set_status(match r {
                    Ok(()) => t("Restore point created."),
                    Err(e) => tf("Could not create a restore point: {error}", &[("error", e)]),
                });
            }
        }
    }

    fn import_log(&mut self) {
        let Some(path) = pick_file(&t("Import install log"), "InstallScope log", &["json"]) else {
            return;
        };
        match self.store.import_log(&path) {
            Ok(log) => {
                self.set_status(tf("Imported {name}", &[("name", log.name)]));
                self.reload_logs();
                self.tab = Tab::Logs;
            }
            Err(e) => self.set_status(tf("Import failed: {error}", &[("error", format!("{e:#}"))])),
        }
    }

    fn export_program_list(&mut self) {
        let Some(path) = save_file(&t("Export program list"), "programs.csv", "CSV", "csv") else {
            return;
        };
        let mut all = self.programs.list.clone();
        all.extend(self.programs.store_apps.clone());
        match std::fs::write(&path, installscope_core::programs::to_csv(&all)) {
            Ok(()) => self.set_status(tf("Saved {path}", &[("path", path.display().to_string())])),
            Err(e) => self.set_status(e.to_string()),
        }
    }
}

/// Large logo (dark card, red E) for the About box and welcome screen.
pub fn logo_image() -> egui::ColorImage {
    decode_png(include_bytes!("../../assets/icon-256.png"))
}

/// Small logo (red card, black E), legible at sidebar/taskbar size.
pub fn logo_small_image() -> egui::ColorImage {
    decode_png(include_bytes!("../../assets/icon-taskbar.png"))
}

fn decode_png(bytes: &[u8]) -> egui::ColorImage {
    match installscope_core::png::decode(bytes) {
        Ok(img) => {
            let size = [img.width as usize, img.height as usize];
            egui::ColorImage::from_rgba_unmultiplied(size, &img.pixels)
        }
        Err(_) => egui::ColorImage::new([1, 1], vec![theme::ACCENT]),
    }
}

/// Plain-language explanation of each page, shown by the Help button.
fn page_help(tab: Tab) -> &'static [&'static str] {
    match tab {
        Tab::Dashboard => &[
            "The dashboard shows how many programs are installed, which ones InstallScope recorded, and anything that needs attention.",
            "Click a tile or a quick action to jump straight to it.",
        ],
        Tab::Programs => &[
            "This is the list of programs installed on this PC. Select one to see its details, or double-click it to uninstall.",
            "Uninstall first runs the program's own uninstaller, then InstallScope looks for anything left behind (leftovers) and shows you before removing it.",
            "Tick several programs to uninstall them one after another. \"Broken\" lists entries whose files are already gone.",
            "Everything InstallScope removes is backed up first, so you can undo it on the Backups page.",
        ],
        Tab::Monitor => &[
            "Monitoring means InstallScope takes a \"before\" picture of your system, you install a program, and InstallScope takes an \"after\" picture.",
            "The difference is exactly what the installation added or changed. InstallScope saves it as an install log.",
            "Later, InstallScope can use that log to remove the program completely and precisely — even things its own uninstaller would forget.",
        ],
        Tab::Logs => &[
            "Each install log lists everything one monitored installation added or changed: files, folders, registry entries, services and tasks.",
            "Use \"Which program created this?\" to find out which program a file or registry entry belongs to.",
            "You can export a log as a readable report, or as scripts that undo the installation on another PC.",
        ],
        Tab::Backups => &[
            "Before InstallScope deletes or changes anything, it copies it here.",
            "If something stops working after a removal, click Restore to put everything back exactly as it was.",
        ],
        Tab::Startup => &[
            "These programs start automatically when Windows starts. Fewer startup programs means Windows starts faster.",
            "Turning an entry off is safe and reversible: it stays in the list and you can turn it back on.",
            "In Advanced mode you can also manage services (background programs) and scheduled tasks.",
        ],
        Tab::Cleaner => &[
            "Programs and Windows leave temporary files behind. They are safe to delete and are recreated when needed.",
            "Click Scan to see how much space each kind uses, choose what to clean, then click Clean.",
        ],
        Tab::Restore => &[
            "A restore point is a saved state of Windows' system files, settings and installed programs. Restoring one undoes system changes made since, without touching your documents.",
            "Create a restore point before risky changes, or use Restore to roll Windows back. Windows restarts to do this.",
            "Advanced mode can also delete restore points, turn System Protection on or off, and set how much disk space restore points may use.",
        ],
        Tab::Lab => &[
            "A snapshot records the registry and files at one moment. Take one, make changes (install something, change settings), take another and compare the two to see exactly what changed.",
            "The differences can be saved as an install log, so they can be undone like a monitored installation. Everything is also available from the command line (installscope --help).",
        ],
        Tab::Shell => &[
            "Programs add commands and shell extensions to the right-click menu of files, folders, drives and the desktop. Many stay behind after the program is gone, which clutters the menu and can slow down File Explorer.",
            "Untick an entry to turn it off; nothing is deleted and you can turn it back on. Broken entries belong to programs that are no longer installed and can be removed with the usual review and backup.",
            "Deep clean looks further: COM classes, \"Open with\" entries, and PATH or environment entries left by removed programs anywhere in Windows.",
        ],
        Tab::Debloat => &[
            "New PCs come with antivirus trials, sponsored games, promotions and maker apps. The Bloatware Shield finds them, including the Store apps that ordinary uninstallers skip.",
            "Recommended items are ticked; optional maker tools are not. Drivers, audio and hotkey tools and Windows' own parts are never listed. Removal goes through the usual review, and everything is backed up.",
        ],
        Tab::ExtInv => &[
            "This page lists every extension installed in the supported Chromium-based and Firefox-based browsers, grouped by browser and profile.",
            "Disable moves an extension's files into InstallScope's own data folder: nothing is deleted, and the rest of the profile (bookmarks, history, settings) is never touched. Restore puts the files back exactly where they were.",
            "Close the browser before disabling or restoring, and restart it afterwards for the change to take effect.",
        ],
        Tab::Builder => &[
            "Choose a program's installer and describe the program. InstallScope snapshots Windows, runs the installer, lets you start the program once, snapshots again and builds a PortableApps.com Format package from the difference.",
            "In Windows Sandbox nothing touches this PC. On this PC, the installation is recorded so you can remove it precisely afterwards.",
            "With the PortableApps.com Launcher and Installer installed, InstallScope also makes the launcher and the .paf.exe installer. Review the Launcher settings it wrote before publishing.",
        ],
        Tab::Settings => &[
            "Simple mode keeps the interface focused and plain. Advanced mode shows technical details, services and scheduled tasks.",
            "The monitoring scope decides which parts of the system snapshots look at. The defaults suit almost everyone.",
        ],
    }
}

impl App {
    /// First-run welcome: language, mode, and what InstallScope does.
    fn welcome(&mut self, ctx: &egui::Context) {
        if !self.welcome_open {
            return;
        }
        let mut done = false;
        egui::Modal::new(egui::Id::new("welcome")).show(ctx, |ui| {
            ui.set_max_width(560.0);
            ui.horizontal(|ui| {
                ui.add(egui::Image::new(&self.logo).fit_to_exact_size(egui::vec2(48.0, 48.0)));
                ui.vertical(|ui| {
                    ui.heading(RichText::new(t("Welcome to InstallScope")).strong());
                    small_muted(
                        ui,
                        t("Remove programs completely — and only what belongs to them."),
                    );
                });
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.label(t("Language"));
                let current = self
                    .languages
                    .iter()
                    .find(|(c, _)| *c == self.settings.language)
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| t("Windows default"));
                egui::ComboBox::from_id_salt("welcome_lang")
                    .selected_text(current)
                    .show_ui(ui, |ui| {
                        let mut changed = ui
                            .selectable_value(
                                &mut self.settings.language,
                                String::new(),
                                t("Windows default"),
                            )
                            .changed();
                        for (code, name) in &self.languages {
                            changed |= ui
                                .selectable_value(&mut self.settings.language, code.clone(), name)
                                .changed();
                        }
                        if changed {
                            i18n::set_language(&self.settings.language);
                        }
                    });
            });
            ui.add_space(10.0);
            for (icon, title, body) in [
                (
                    "🗑",
                    "Uninstall completely",
                    "Remove programs plus the leftovers their uninstallers forget.",
                ),
                (
                    "📡",
                    "Monitor installations",
                    "Record what an installer changes so it can be undone precisely later.",
                ),
                (
                    "💾",
                    "Undo anything",
                    "Everything removed is backed up first and can be restored.",
                ),
            ] {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(icon).size(20.0).color(theme::ACCENT_TEXT));
                    ui.vertical(|ui| {
                        ui.label(RichText::new(t(title)).strong());
                        small_muted(ui, t(body));
                    });
                });
                ui.add_space(4.0);
            }
            ui.add_space(8.0);
            ui.label(RichText::new(t("How do you want InstallScope to look?")).strong());
            ui.columns(2, |cols| {
                for (i, (adv, title, body)) in [
                    (
                        false,
                        "Simple",
                        "Plain language, fewer technical details. Recommended if you're not sure.",
                    ),
                    (
                        true,
                        "Advanced",
                        "All details: registry keys, services, scheduled tasks and every tool.",
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let selected = self.settings.advanced_mode == adv;
                    let resp = egui::Frame::new()
                        .fill(if selected {
                            theme::NAV_SELECTED
                        } else {
                            theme::CARD
                        })
                        .corner_radius(12)
                        .inner_margin(12)
                        .show(&mut cols[i], |ui| {
                            ui.set_width(ui.available_width());
                            ui.with_layout(egui::Layout::top_down(egui::Align::Min), |ui| {
                                ui.label(RichText::new(t(title)).strong());
                                small_muted(ui, t(body));
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    super::a11y::selectable(&resp, &format!("{}. {}", t(title), t(body)), selected);
                    if resp.clicked() {
                        self.settings.advanced_mode = adv;
                    }
                }
            });
            ui.add_space(10.0);
            small_muted(
                ui,
                t("You can change both at any time in Settings. Press F1 on any page for help."),
            );
            ui.add_space(6.0);
            let start = theme::wide_button(ui, t("Get started"), true);
            if ui.memory(|m| m.focused().is_none()) {
                start.request_focus();
            }
            if start.clicked() {
                done = true;
            }
        });
        if done {
            self.welcome_open = false;
            self.settings.first_run_done = true;
            self.save_settings();
            self.admin_prompt = !self.elevated;
        }
    }

    /// Start monitoring an installer handed over by Explorer, once the
    /// welcome and admin dialogs are out of the way.
    fn start_pending_install(&mut self, ctx: &egui::Context) {
        if self.pending_install.is_none() || self.welcome_open || self.admin_prompt {
            return;
        }
        let path = self.pending_install.take().unwrap();
        self.tab = Tab::Monitor;
        if self.monitor.is_active() {
            self.set_status(t(
                "Another installation is already being monitored. Finish or cancel it first.",
            ));
            return;
        }
        // "Install with InstallScope": check the setup file first.
        self.precheck_installer(ctx, path);
    }

    /// A quick fade-in when the page changes (120 ms; skipped when the
    /// person has asked Windows for fewer animations).
    fn page_transition(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        if self.shown_tab != Some(self.tab) {
            let first = self.shown_tab.is_none();
            self.shown_tab = Some(self.tab);
            self.tab_since = if first || !theme::animations_enabled() {
                -1.0
            } else {
                now
            };
        }
        let t = ((now - self.tab_since) / 0.12) as f32;
        if (0.0..1.0).contains(&t) {
            let screen = ctx.content_rect();
            let area = egui::Rect::from_min_max(
                egui::pos2(screen.left() + 214.0, screen.top() + 24.0),
                egui::pos2(screen.right(), screen.bottom() - 26.0),
            );
            let eased = 1.0 - (1.0 - t) * (1.0 - t);
            let alpha = ((1.0 - eased) * 200.0) as u8;
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Middle,
                egui::Id::new("page_fade"),
            ))
            .rect_filled(
                area,
                0.0,
                egui::Color32::from_rgba_unmultiplied(
                    theme::BG.r(),
                    theme::BG.g(),
                    theme::BG.b(),
                    alpha,
                ),
            );
            ctx.request_repaint();
        }
    }

    /// Keyboard shortcuts.
    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let tabs = [
            Tab::Dashboard,
            Tab::Programs,
            Tab::Monitor,
            Tab::Logs,
            Tab::Backups,
            Tab::Startup,
            Tab::Cleaner,
            Tab::Restore,
            Tab::Settings,
        ];
        let mut go: Option<Tab> = None;
        let mut ask = false;
        ctx.input_mut(|i| {
            for (n, key) in [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
            ]
            .iter()
            .enumerate()
            {
                if i.consume_key(Modifiers::CTRL, *key) {
                    go = Some(tabs[n]);
                }
            }
            if i.consume_key(Modifiers::CTRL, Key::F) {
                self.focus_search = true;
            }
            if i.consume_key(Modifiers::CTRL, Key::K) {
                ask = true;
            }
            if i.consume_key(Modifiers::NONE, Key::F1) {
                self.help_open = !self.help_open;
            }
            if i.consume_key(Modifiers::NONE, Key::F10) {
                self.focus_menu = true;
            }
            let ctrl_tab = self.ctrl_tab.take();
            if let Some(back) = ctrl_tab {
                let mut order = tabs.to_vec();
                order.insert(8, Tab::Lab);
                order.insert(9, Tab::Builder);
                order.insert(10, Tab::Debloat);
                order.insert(11, Tab::Shell);
                let pos = order.iter().position(|t| *t == self.tab).unwrap_or(0);
                let n = order.len();
                let next = if back {
                    (pos + n - 1) % n
                } else {
                    (pos + 1) % n
                };
                go = Some(order[next]);
            }
        });
        if self.help_open
            && self.confirm.is_none()
            && self.wizard.is_none()
            && self.removal.is_none()
            && !self.about_open
            && super::a11y::escape_pressed(ctx)
        {
            self.help_open = false;
        }
        if ask {
            self.open_ask();
        }
        if let Some(tab) = go {
            self.tab = tab;
            self.on_tab_opened(ctx, tab);
        }
        let typing = ctx.wants_keyboard_input();
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F5)) {
            match self.tab {
                Tab::Programs | Tab::Dashboard => self.refresh_programs(ctx),
                Tab::Startup => self.refresh_startup(ctx),
                Tab::Logs => self.reload_logs(),
                Tab::Backups => self.reload_backups(),
                _ => {}
            }
        }
        if !typing
            && self.tab == Tab::Programs
            && self.wizard.is_none()
            && self.removal.is_none()
            && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Delete))
        {
            if let Some(key) = self.programs.selected.clone() {
                let all: Vec<_> = self
                    .programs
                    .list
                    .iter()
                    .chain(self.programs.store_apps.iter())
                    .filter(|p| p.key == key)
                    .cloned()
                    .collect();
                self.start_uninstall(all, false);
            }
        }
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        raw_input.events.retain(|e| match e {
            egui::Event::Key {
                key: egui::Key::Tab,
                pressed,
                modifiers,
                ..
            } if modifiers.ctrl => {
                if *pressed {
                    self.ctrl_tab = Some(modifiers.shift);
                }
                false
            }
            _ => true,
        });
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        // Benchmarks: record why each frame was drawn (INSTALLSCOPE_REPAINT_LOG=file).
        if let Some(path) = std::env::var_os("INSTALLSCOPE_REPAINT_LOG") {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                let causes: Vec<String> = ctx
                    .repaint_causes()
                    .iter()
                    .map(|c| format!("{}:{} {}", c.file, c.line, c.reason))
                    .collect();
                let _ = writeln!(f, "{:.2} {}", ctx.input(|i| i.time), causes.join(" | "));
            }
        }
        super::a11y::track_input_mode(ctx);
        // Ctrl+Plus / Ctrl+Minus / Ctrl+0 zoom (built into egui): remember it.
        let z = ctx.zoom_factor();
        if (z - self.settings.zoom).abs() > 0.001 && !ctx.input(|i| i.pointer.any_down()) {
            self.settings.zoom = z.clamp(0.75, 2.0);
            if (self.settings.zoom - z).abs() > 0.001 {
                ctx.set_zoom_factor(self.settings.zoom);
            }
            self.save_settings();
        }
        if self.style_frames > 0 || self.frame.is_multiple_of(120) {
            platform::apply_window_style(theme::BORDER_RGB, true);
            self.style_frames = self.style_frames.saturating_sub(1);
        }
        self.poll_jobs(ctx);
        self.start_pending_install(ctx);

        self.menu_bar(ctx);
        self.status_bar(ctx);
        self.nav_sidebar(ctx);
        self.page_header(ctx);
        match self.tab {
            Tab::Dashboard => self.ui_dashboard(ctx),
            Tab::Programs => self.ui_programs(ctx),
            Tab::Monitor => self.ui_monitor(ctx),
            Tab::Logs => self.ui_logs(ctx),
            Tab::Backups => self.ui_backups(ctx),
            Tab::Startup => self.ui_startup(ctx),
            Tab::Cleaner => self.ui_cleaner(ctx),
            Tab::Restore => self.ui_restore(ctx),
            Tab::Lab => self.ui_lab(ctx),
            Tab::Builder => self.ui_builder(ctx),
            Tab::Shell => self.ui_shell(ctx),
            Tab::Debloat => self.ui_debloat(ctx),
            Tab::ExtInv => self.ui_extinv(ctx),
            Tab::Settings => self.ui_settings(ctx),
        }
        self.page_transition(ctx);
        self.ui_wizard(ctx);
        self.ui_removal(ctx);
        self.ui_transfer(ctx);
        self.dialogs(ctx);
        self.ui_ask(ctx);
        self.welcome(ctx);
        self.shortcuts(ctx);
        super::a11y::focus_ring(ctx, theme::ACCENT_TEXT);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [17.0 / 255.0, 17.0 / 255.0, 19.0 / 255.0, 1.0]
    }
}

#[cfg(test)]
mod confirm_tests {
    use super::Confirm;
    use installscope_core::slimmer::DeepItem;

    #[test]
    fn only_the_irreversible_three_always_confirm() {
        assert!(Confirm::RestoreTo(1).always_confirm());
        assert!(Confirm::DeleteBackup("b".into()).always_confirm());
        assert!(Confirm::DeepClean(DeepItem::ComponentStore).always_confirm());
        // Everything else follows the user's confirmation setting.
        assert!(!Confirm::DeleteLog("l".into()).always_confirm());
        assert!(!Confirm::RestoreBackup("b".into()).always_confirm());
        assert!(!Confirm::CleanJunk.always_confirm());
        assert!(!Confirm::DeleteStartup(0).always_confirm());
        assert!(!Confirm::DeleteTask(0).always_confirm());
        assert!(!Confirm::ResetSettings.always_confirm());
        assert!(!Confirm::CancelMonitoring.always_confirm());
        assert!(!Confirm::SetProtection(false).always_confirm());
        assert!(!Confirm::SetProtection(true).always_confirm());
        assert!(!Confirm::DeleteRestorePoint(1).always_confirm());
    }

    #[test]
    fn destructive_actions_start_on_cancel() {
        for action in [
            Confirm::DeleteLog("l".into()),
            Confirm::RestoreBackup("b".into()),
            Confirm::DeleteBackup("b".into()),
            Confirm::CleanJunk,
            Confirm::DeleteStartup(0),
            Confirm::DeleteTask(0),
            Confirm::ResetSettings,
            Confirm::DeepClean(DeepItem::RestorePoints),
            Confirm::RestoreTo(1),
            Confirm::DeleteRestorePoint(1),
            // Turning protection off deletes every restore point.
            Confirm::SetProtection(false),
        ] {
            assert!(action.is_destructive(), "{action:?}");
        }
        // Benign actions keep starting on the confirm button.
        assert!(!Confirm::CancelMonitoring.is_destructive());
        assert!(!Confirm::SetProtection(true).is_destructive());
    }
}
