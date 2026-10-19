//! Cleaner page: temporary files, caches, crash dumps, Recycle Bin.

use super::app::Confirm as DeepConfirm;
use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::cleaner::{self, Category, ScanResult};
use installscope_core::i18n::{t, tf};
use installscope_core::model::human_size;
use installscope_core::slimmer::{self, Analysis, DeepItem};

pub struct CleanerState {
    pub cats: Vec<(Category, bool, Option<ScanResult>)>,
    /// Deep cleanup: analysis per item (same order as `DeepItem::ALL`).
    pub deep: Vec<Option<Result<Analysis, String>>>,
    pub deep_job: Option<Job<Vec<Result<Analysis, String>>>>,
    pub deep_run: Option<(DeepItem, Job<Result<(), String>>)>,
    pub deep_restore_point: bool,
    pub scan_job: Option<Job<Vec<ScanResult>>>,
    pub clean_job: Option<Job<Vec<ScanResult>>>,
    pub freed: Option<u64>,
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
        self.cleaner.freed = None;
        self.cleaner.scan_job = Some(Job::spawn(&self.ctx, move |pr| {
            cats.iter()
                .map(|c| {
                    pr.set_phase(c.id);
                    cleaner::scan(c)
                })
                .collect()
        }));
    }

    pub fn start_clean(&mut self) {
        let advanced = self.settings.advanced_mode;
        let cats: Vec<Category> = self
            .cleaner
            .cats
            .iter()
            .filter(|(_, on, _)| *on)
            .filter(|(c, _, _)| advanced || !cleaner::is_advanced(c.id))
            .map(|(c, _, _)| c.clone())
            .collect();
        self.cleaner.clean_job = Some(Job::spawn(&self.ctx, move |_| {
            cats.iter().map(cleaner::clean).collect()
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
                                    let color = if r.bytes > 100 * 1024 * 1024 { theme::WARN } else { theme::MUTED };
                                    ui.label(RichText::new(human_size(r.bytes)).strong().color(color));
                                }
                            });
                        });
                    });
                    if locked {
                        *on = false;
                    }
                    ui.add_space(6.0);
                }
                ui.add_space(4.0);
                self.ui_schedule(ui);
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
            let selected: u64 = self.cleaner.cats.iter().filter(|(_, on, _)| *on).filter(|(c, _, _)| advanced || !cleaner::is_advanced(c.id)).filter_map(|(_, _, r)| r.as_ref()).map(|r| r.bytes).sum();
            let scanned = self.cleaner.cats.iter().any(|(_, _, r)| r.is_some());
            ui.columns(2, |cols| {
                if theme::wide_button(&mut cols[0], format!("🔍 {}", t("Scan")), true).clicked() {
                    self.start_scan();
                }
                let any = self.cleaner.cats.iter().any(|(_, on, _)| *on);
                let label = if scanned { tf("Clean {size}", &[("size", human_size(selected))]) } else { t("Clean") };
                if theme::wide_button(&mut cols[1], format!("🗑 {label}"), any).clicked() {
                    self.ask(
                        "Clean selected items?",
                        t("Cleaned files are deleted permanently (they are temporary and regenerable, so they are not backed up)."),
                        "Clean",
                        Confirm::CleanJunk,
                    );
                }
            });
        });
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
