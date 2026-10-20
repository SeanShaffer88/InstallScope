//! Cleaner page: temporary files, caches, crash dumps, Recycle Bin.

use super::app::Confirm as DeepConfirm;
use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::cleaner::{self, Category, CategoryScan, CleanFile};
use installscope_core::i18n::{t, tf};
use installscope_core::model::human_size;
use installscope_core::slimmer::{self, Analysis, DeepItem};

pub struct CleanerState {
    pub cats: Vec<(Category, bool, Option<CategoryScan>)>,
    /// Deep cleanup: analysis per item (same order as `DeepItem::ALL`).
    pub deep: Vec<Option<Result<Analysis, String>>>,
    pub deep_job: Option<Job<Vec<Result<Analysis, String>>>>,
    pub deep_run: Option<(DeepItem, Job<Result<(), String>>)>,
    pub deep_restore_point: bool,
    pub scan_job: Option<Job<Vec<CategoryScan>>>,
    pub clean_job: Option<Job<Vec<installscope_core::cleaner::ScanResult>>>,
    pub freed: Option<u64>,
    /// Protected-paths editor text (one pattern per line); `None` until
    /// first shown, when it is loaded from the persisted setting.
    pub protect_text: Option<String>,
    /// Scheduled cleaning, read from Task Scheduler on first view.
    pub schedule: Option<Option<(installscope_core::schedule::Frequency, Vec<String>)>>,
    pub schedule_error: Option<String>,
    pub schedule_job: Option<Job<Option<(installscope_core::schedule::Frequency, Vec<String>)>>>,
}

impl Default for CleanerState {
    fn default() -> Self {
        CleanerState {
            cats: cleaner::categories()
                .into_iter()
                .map(|c| {
                    let on = c.default_on;
                    (c, on, None)
                })
                .collect(),
            deep: vec![None; DeepItem::ALL.len()],
            deep_job: None,
            deep_run: None,
            deep_restore_point: true,
            scan_job: None,
            clean_job: None,
            freed: None,
            protect_text: None,
            schedule: None,
            schedule_error: None,
            schedule_job: None,
        }
    }
}

fn cat_text(id: &str) -> (&'static str, &'static str) {
    match id {
        "user_temp" => (
            "Your temporary files",
            "Files programs left in your Temp folder",
        ),
        "windows_temp" => ("Windows temporary files", "The system-wide Temp folder"),
        "crash_dumps" => (
            "Crash dumps and error reports",
            "Memory dumps and Windows Error Reporting files",
        ),
        "thumbnails" => ("Thumbnail cache", "Explorer rebuilds thumbnails as needed"),
        "shader_cache" => (
            "Graphics shader caches",
            "DirectX and GPU driver shader caches",
        ),
        "update_cache" => (
            "Windows Update downloads",
            "Already-installed update packages",
        ),
        "chrome_cache" => (
            "Google Chrome cache",
            "Cached web content (not history, passwords or cookies)",
        ),
        "edge_cache" => (
            "Microsoft Edge cache",
            "Cached web content (not history, passwords or cookies)",
        ),
        "firefox_cache" => (
            "Firefox cache",
            "Cached web content (not history, passwords or cookies)",
        ),
        "windows_logs" => (
            "Windows setup and update logs",
            "Logs from Windows Update, servicing and upgrades",
        ),
        "memory_dumps" => (
            "System memory dumps",
            "Large files Windows writes after a crash (blue screen)",
        ),
        "delivery_optimization" => (
            "Delivery Optimization cache",
            "Update pieces Windows keeps to share with other PCs",
        ),
        "prefetch" => (
            "Prefetch data",
            "Start-up hints; programs open a little slower until Windows rebuilds them",
        ),
        "browser_cookies" => (
            "Browser cookies",
            "Signs you out of websites. Close your browsers first. Chrome, Edge, Brave, Vivaldi, Opera and Firefox-based browsers, every profile",
        ),
        "browser_history" => (
            "Browsing history",
            "Visited pages and address-bar suggestions; for Firefox only form history, because its history shares a file with bookmarks",
        ),
        "browser_site_data" => (
            "Website data",
            "Data websites store on your PC (local storage, databases, offline data). Sites may forget settings",
        ),
        "browser_sessions" => (
            "Saved browser sessions",
            "Tabs your browsers would reopen next time",
        ),
        "font_cache" => (
            "Font cache",
            "Windows rebuilds it; fixes fonts that look wrong. Takes effect after a restart",
        ),
        "recycle_bin" => (
            "Recycle Bin",
            "Permanently deletes what is in the Recycle Bin",
        ),
        _ => ("", ""),
    }
}

impl App {
    fn start_scan(&mut self) {
        let cats: Vec<Category> = self
            .cleaner
            .cats
            .iter()
            .map(|(c, _, _)| c.clone())
            .collect();
        let protect = self.settings.cleaner_protect.clone();
        self.cleaner.freed = None;
        self.cleaner.scan_job = Some(Job::spawn(&self.ctx, move |pr| {
            cats.iter()
                .map(|c| {
                    pr.set_phase(c.id);
                    cleaner::scan_detailed(c, &protect)
                })
                .collect()
        }));
    }

    pub fn start_clean(&mut self) {
        let advanced = self.settings.advanced_mode;
        // Delete exactly the files the user reviewed: the retained scan
        // results of the ticked categories. A ticked category that was
        // never scanned is not cleaned (the confirmation names those it
        // leaves out). `None` in place of a file list means the Recycle
        // Bin, which can only be emptied as a whole.
        let jobs: Vec<(Category, Option<Vec<CleanFile>>)> = self
            .cleaner
            .cats
            .iter()
            .filter(|(_, on, _)| *on)
            .filter(|(c, _, _)| advanced || !cleaner::is_advanced(c.id))
            .filter_map(|(c, _, res)| {
                res.as_ref().map(|s| {
                    if c.id == "recycle_bin" {
                        (c.clone(), None)
                    } else {
                        (c.clone(), Some(s.files.clone()))
                    }
                })
            })
            .collect();
        self.cleaner.clean_job = Some(Job::spawn(&self.ctx, move |_| {
            jobs.iter()
                .map(|(c, files)| match files {
                    Some(files) => cleaner::clean_files(c, files),
                    None => cleaner::clean(c),
                })
                .collect()
        }));
    }

    fn analyze_deep(&mut self) {
        if self.cleaner.deep_job.is_some() {
            return;
        }
        self.cleaner.deep_job = Some(Job::spawn(&self.ctx, |pr| {
            DeepItem::ALL
                .iter()
                .map(|i| {
                    pr.set_phase(t(i.title()));
                    slimmer::analyze(*i).map_err(|e| e.to_string())
                })
                .collect()
        }));
    }

    pub fn start_deep(&mut self, item: DeepItem) {
        if self.cleaner.deep_run.is_some() {
            return;
        }
        let rp = self.cleaner.deep_restore_point && item != DeepItem::RestorePoints;
        let job = Job::spawn(&self.ctx, move |pr| {
            if rp {
                pr.set_phase(t("Creating a restore point…"));
                installscope_core::restore::create_restore_point(
                    "Before InstallScope deep cleanup",
                )
                .map_err(|error| {
                    format!("Restore point not created; deep cleanup was canceled: {error:#}")
                })?;
            }
            slimmer::clean(item, &|s| pr.set_phase(s)).map_err(|e| e.to_string())
        });
        self.cleaner.deep_run = Some((item, job));
    }

    /// Deep cleanup card (Advanced mode).
    fn ui_deep(&mut self, ui: &mut egui::Ui) {
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("Deep cleanup")).heading());
            small_muted(ui, t("One-time cleanups of Windows itself, using Windows' own tools. Each one is explained and confirmed separately and can't be undone."));
            ui.add_space(6.0);
            if !self.elevated {
                theme::warning_banner(
                    ui,
                    t("Run InstallScope as Administrator to use deep cleanup."),
                );
                return;
            }
            if let Some((item, job)) = &self.cleaner.deep_run {
                job_progress(
                    ui,
                    &format!("{} — {}", t(item.title()), job.progress.phase()),
                    0,
                );
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(500));
                return;
            }
            let analyzing = self.cleaner.deep_job.is_some();
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        !analyzing,
                        egui::Button::new(format!("🔍 {}", t("Analyze"))),
                    )
                    .clicked()
                {
                    self.analyze_deep();
                }
                ui.checkbox(
                    &mut self.cleaner.deep_restore_point,
                    t("Create a restore point first"),
                );
            });
            if let Some(job) = &self.cleaner.deep_job {
                job_progress(
                    ui,
                    &tf("Analyzing {name}…", &[("name", job.progress.phase())]),
                    0,
                );
            }
            ui.add_space(4.0);
            let mut run: Option<DeepItem> = None;
            for (i, item) in DeepItem::ALL.iter().enumerate() {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_max_width(ui.available_width() - 190.0);
                        ui.label(RichText::new(t(item.title())).strong());
                        small_muted(ui, t(item.description()));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match &self.cleaner.deep[i] {
                            Some(Ok(a)) if a.applicable => {
                                if ui.button(t("Clean up")).clicked() {
                                    run = Some(*item);
                                }
                                let size = a.bytes.map(human_size);
                                let text = match (size, a.count) {
                                    (Some(s), _) => s,
                                    (None, Some(n)) => tf("{n} items", &[("n", n.to_string())]),
                                    _ => String::new(),
                                };
                                ui.label(RichText::new(text).strong().color(theme::WARN));
                            }
                            Some(Ok(_)) => {
                                ui.label(
                                    RichText::new(format!("✔ {}", t("Nothing to clean")))
                                        .color(theme::OK),
                                );
                            }
                            Some(Err(e)) => {
                                ui.label(RichText::new("⚠").color(theme::WARN))
                                    .on_hover_text(e);
                            }
                            None => {}
                        }
                    });
                });
            }
            ui.separator();
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.set_max_width(ui.available_width() - 190.0);
                    ui.label(RichText::new(t("Unneeded apps")).strong());
                    small_muted(ui, t("Pre-installed Store apps you don't use can be removed from the Programs page."));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t("Show Store apps")).clicked() {
                        self.tab = super::app::Tab::Programs;
                        self.programs.view = super::programs::View::Store;
                        if self.programs.store_apps.is_empty() && self.programs.store_job.is_none() {
                            self.programs.store_job = Some(Job::spawn(&self.ctx, |_| installscope_core::programs::list_store_apps()));
                        }
                    }
                });
            });
            if let Some(item) = run {
                self.ask(
                    "Run this cleanup?",
                    format!("{}\n\n{}", t(item.title()), t(item.description())),
                    "Clean up",
                    DeepConfirm::DeepClean(item),
                );
            }
        });
    }

    pub fn poll_cleaner(&mut self) {
        if let Some(r) = self.cleaner.deep_job.as_ref().and_then(|j| j.poll()) {
            self.cleaner.deep_job = None;
            self.cleaner.deep = r.into_iter().map(Some).collect();
        }
        if let Some(r) = self.cleaner.deep_run.as_ref().and_then(|(_, j)| j.poll()) {
            let (item, _) = self.cleaner.deep_run.take().unwrap();
            match r {
                Ok(()) => self.set_status(tf("{name}: done.", &[("name", t(item.title()))])),
                Err(e) => self.set_status(format!("{}: {e}", t(item.title()))),
            }
            self.analyze_deep();
        }
        if let Some(r) = self.cleaner.scan_job.as_ref().and_then(|j| j.poll()) {
            self.cleaner.scan_job = None;
            for ((_, _, res), s) in self.cleaner.cats.iter_mut().zip(r) {
                *res = Some(s);
            }
        }
        if let Some(r) = self.cleaner.clean_job.as_ref().and_then(|j| j.poll()) {
            self.cleaner.clean_job = None;
            let freed: u64 = r.iter().map(|s| s.bytes).sum();
            self.cleaner.freed = Some(freed);
            self.set_status(tf("Freed {size}", &[("size", human_size(freed))]));
            self.start_scan();
        }
    }

    /// The Clean confirmation text. It names every category about to be
    /// cleaned with its size and file count, states the total, and is
    /// honest about what is left out: files the protected list keeps,
    /// and ticked categories that were never scanned (only reviewed
    /// files are ever deleted). `None` when nothing ticked has a scan.
    fn clean_confirm_body(&self) -> Option<String> {
        let advanced = self.settings.advanced_mode;
        let (mut files, mut bytes, mut kept_files, mut kept_bytes) = (0u64, 0u64, 0u64, 0u64);
        let mut items: Vec<String> = Vec::new();
        let mut unscanned: Vec<String> = Vec::new();
        for (cat, on, res) in &self.cleaner.cats {
            if !*on || (!advanced && cleaner::is_advanced(cat.id)) {
                continue;
            }
            let (title, _) = cat_text(cat.id);
            match res {
                Some(s) => {
                    items.push(tf(
                        "{name}: {size} ({n} files)",
                        &[
                            ("name", t(title)),
                            ("size", human_size(s.totals.bytes)),
                            ("n", s.totals.files.to_string()),
                        ],
                    ));
                    files += s.totals.files;
                    bytes += s.totals.bytes;
                    let kt = s.kept_totals();
                    kept_files += kt.files;
                    kept_bytes += kt.bytes;
                }
                None => unscanned.push(t(title)),
            }
        }
        if items.is_empty() {
            return None;
        }
        let mut body = t("Cleaned files are deleted permanently (they are temporary and regenerable, so they are not backed up).");
        body.push_str("\n\n");
        body.push_str(&items.join("\n"));
        body.push_str("\n\n");
        body.push_str(&tf(
            "Total: {size} in {n} files",
            &[("size", human_size(bytes)), ("n", files.to_string())],
        ));
        if kept_files > 0 {
            body.push('\n');
            body.push_str(&tf(
                "{n} files ({size}) are on your protected list and will not be deleted.",
                &[
                    ("n", kept_files.to_string()),
                    ("size", human_size(kept_bytes)),
                ],
            ));
        }
        if !unscanned.is_empty() {
            body.push('\n');
            body.push_str(&tf(
                "Not included (scan to review them first): {list}",
                &[("list", unscanned.join(", "))],
            ));
        }
        Some(body)
    }

    pub fn ui_cleaner(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            if let Some(f) = self.cleaner.freed {
                theme::card(ui, |ui| {
                    ui.label(RichText::new(tf("✔ Freed {size}", &[("size", human_size(f))])).color(theme::OK).strong());
                    small_muted(ui, t("Files that were in use were skipped."));
                });
                ui.add_space(8.0);
            }
            let busy = self.cleaner.scan_job.is_some() || self.cleaner.clean_job.is_some();
            egui::ScrollArea::vertical().auto_shrink([false, false]).max_height(ui.available_height() - 52.0).show(ui, |ui| {
                let elevated = self.elevated;
                let advanced = self.settings.advanced_mode;
                small_muted(ui, t("Ticked categories are cleaned; unticked categories are left completely alone. Scan first, then review the exact files — cleaning deletes exactly the files the review lists."));
                ui.add_space(4.0);
                for (cat, on, res) in &mut self.cleaner.cats {
                    if !advanced && cleaner::is_advanced(cat.id) {
                        continue;
                    }
                    let (title, desc) = cat_text(cat.id);
                    let locked = cat.needs_admin && !elevated;
                    egui::Frame::new().fill(theme::CARD).corner_radius(12).inner_margin(egui::Margin::symmetric(16, 10)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(!locked, |ui| ui.checkbox(on, ""));
                            ui.vertical(|ui| {
                                ui.label(RichText::new(t(title)).strong());
                                small_muted(ui, t(desc));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if locked {
                                    theme::badge(ui, t("Needs Administrator"), theme::WARN);
                                } else if let Some(r) = res {
                                    let color = if r.totals.bytes > 100 * 1024 * 1024 { theme::WARN } else { theme::MUTED };
                                    ui.label(RichText::new(human_size(r.totals.bytes)).strong().color(color));
                                }
                            });
                        });
                        if let Some(s) = res.as_ref() {
                            if !s.kept.is_empty() {
                                let kt = s.kept_totals();
                                small_muted(ui, tf("🔒 {n} files ({size}) kept by your protected list — not deleted", &[("n", kt.files.to_string()), ("size", human_size(kt.bytes))]));
                            }
                            if cat.id == "recycle_bin" {
                                if s.totals.files > 0 {
                                    small_muted(ui, t("The Recycle Bin's items can't be listed one by one; cleaning empties the whole Bin."));
                                }
                            } else if !s.files.is_empty() || !s.kept.is_empty() {
                                egui::CollapsingHeader::new(tf("Review files ({n})", &[("n", s.files.len().to_string())]))
                                    .id_salt(cat.id)
                                    .show(ui, |ui| {
                                        for f in s.files.iter().take(300) {
                                            ui.label(RichText::new(format!("{} — {}", f.path.display(), human_size(f.bytes))).small());
                                        }
                                        if s.files.len() > 300 {
                                            small_muted(ui, tf("… and {n} more files", &[("n", (s.files.len() - 300).to_string())]));
                                        }
                                        if !s.kept.is_empty() {
                                            ui.add_space(4.0);
                                            ui.label(RichText::new(t("Kept by your protected list:")).strong().small());
                                            for f in s.kept.iter().take(50) {
                                                ui.label(RichText::new(format!("{} — {}", f.path.display(), human_size(f.bytes))).small());
                                            }
                                            if s.kept.len() > 50 {
                                                small_muted(ui, tf("… and {n} more kept files", &[("n", (s.kept.len() - 50).to_string())]));
                                            }
                                        }
                                    });
                            }
                        }
                    });
                    if locked {
                        *on = false;
                    }
                    ui.add_space(6.0);
                }
                ui.add_space(4.0);
                self.ui_schedule(ui);
                self.ui_protect(ui);
                if advanced {
                    ui.add_space(8.0);
                    self.ui_deep(ui);
                }
            });
            ui.add_space(6.0);
            if busy {
                job_progress(ui, &if self.cleaner.scan_job.is_some() { t("Scanning…") } else { t("Cleaning…") }, 0);
                return;
            }
            let advanced = self.settings.advanced_mode;
            let selected: u64 = self.cleaner.cats.iter().filter(|(_, on, _)| *on).filter(|(c, _, _)| advanced || !cleaner::is_advanced(c.id)).filter_map(|(_, _, r)| r.as_ref()).map(|r| r.totals.bytes).sum();
            let scanned = self.cleaner.cats.iter().any(|(_, _, r)| r.is_some());
            ui.columns(2, |cols| {
                if theme::wide_button(&mut cols[0], format!("🔍 {}", t("Scan")), true).clicked() {
                    self.start_scan();
                }
                let any = self.cleaner.cats.iter().any(|(_, on, _)| *on);
                let label = if scanned { tf("Clean {size}", &[("size", human_size(selected))]) } else { t("Clean") };
                if theme::wide_button(&mut cols[1], format!("🗑 {label}"), any).clicked() {
                    match self.clean_confirm_body() {
                        Some(body) => self.ask(
                            "Clean selected items?",
                            body,
                            "Clean",
                            Confirm::CleanJunk,
                        ),
                        None => self.set_status(t(
                            "Scan first — the review shows exactly which files will be deleted.",
                        )),
                    }
                }
            });
        });
    }

    /// Protected-paths editor (the Cleaner's keep-list). Persisted in
    /// `Settings.cleaner_protect`; matching uses the same exclusion
    /// rules as the monitoring scope lists (`util::Exclusions`).
    fn ui_protect(&mut self, ui: &mut egui::Ui) {
        if self.cleaner.protect_text.is_none() {
            self.cleaner.protect_text = Some(self.settings.cleaner_protect.join("\n"));
        }
        let mut save = false;
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("Protected paths")).strong());
            small_muted(ui, t("Files and folders listed here are never cleaned, even inside a ticked category. One per line. A folder protects everything inside it. You can use %VARIABLES% and * wildcards, in the same style as the monitoring scope lists in Settings. To stay signed in to websites, protect a browser's Cookies file — for example %LOCALAPPDATA%\\Google\\Chrome\\User Data\\Default\\Network\\Cookies."));
            let text = self.cleaner.protect_text.as_mut().unwrap();
            ui.add(
                egui::TextEdit::multiline(text)
                    .code_editor()
                    .desired_rows(4)
                    .desired_width(f32::INFINITY),
            );
            let parsed: Vec<String> = text
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            let changed = parsed != self.settings.cleaner_protect;
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(changed, egui::Button::new(t("Save protected list")))
                    .clicked()
                {
                    save = true;
                }
                if !self.settings.cleaner_protect.is_empty() {
                    small_muted(
                        ui,
                        tf(
                            "{n} protected entries in effect",
                            &[("n", self.settings.cleaner_protect.len().to_string())],
                        ),
                    );
                }
            });
        });
        if save {
            let parsed: Vec<String> = self
                .cleaner
                .protect_text
                .as_ref()
                .unwrap()
                .lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect();
            self.settings.cleaner_protect = parsed;
            self.save_settings();
            // Existing reviews were computed under the old list; clear
            // them so a stale deletion set can't be confirmed by mistake.
            for (_, _, res) in &mut self.cleaner.cats {
                *res = None;
            }
            self.set_status(t(
                "Protected list saved. Scan again to see the updated review.",
            ));
        }
    }

    fn ui_schedule(&mut self, ui: &mut egui::Ui) {
        use installscope_core::schedule::{self, Frequency};
        // Task Scheduler is asked in the background (it takes a moment).
        if let Some(r) = self.cleaner.schedule_job.as_ref().and_then(|j| j.poll()) {
            self.cleaner.schedule_job = None;
            self.cleaner.schedule = Some(r);
        }
        if self.cleaner.schedule.is_none() && self.cleaner.schedule_job.is_none() {
            self.cleaner.schedule_job = Some(Job::spawn(ui.ctx(), |_| schedule::current()));
        }
        let current = self.cleaner.schedule.clone().flatten();
        theme::card(ui, |ui| {
            let mut choice: Option<Option<Frequency>> = None;
            ui.horizontal(|ui| {
                ui.label(RichText::new(t("Clean automatically")).strong());
                let label = current
                    .as_ref()
                    .map(|c| t(c.0.label()))
                    .unwrap_or_else(|| t("Off"));
                egui::ComboBox::from_id_salt("clean_schedule")
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(current.is_none(), t("Off")).clicked() {
                            choice = Some(None);
                        }
                        for f in Frequency::ALL {
                            let on = current.as_ref().is_some_and(|c| c.0 == f);
                            if ui.selectable_label(on, t(f.label())).clicked() {
                                choice = Some(Some(f));
                            }
                        }
                    });
            });
            small_muted(ui, t("Cleans the kinds of junk ticked above through Windows Task Scheduler, even when InstallScope is closed."));
            if let Some((_, cats)) = &current {
                let names: Vec<String> = cats.iter().map(|id| t(cat_text(id).0)).collect();
                small_muted(
                    ui,
                    tf("Currently cleans: {list}", &[("list", names.join(", "))]),
                );
            }
            if let Some(e) = &self.cleaner.schedule_error {
                ui.label(RichText::new(e).small().color(theme::BAD));
            }
            if let Some(c) = choice {
                let advanced = self.settings.advanced_mode;
                let ids: Vec<&str> = self
                    .cleaner
                    .cats
                    .iter()
                    .filter(|(cat, on, _)| *on && (advanced || !cleaner::is_advanced(cat.id)))
                    .map(|(cat, _, _)| cat.id)
                    .collect();
                let r = match c {
                    None => schedule::remove(),
                    Some(f) => schedule::set(&ids, f, self.elevated),
                };
                self.cleaner.schedule_error = r.err().map(|e| e.to_string());
                self.cleaner.schedule = None;
            }
        });
    }
}
