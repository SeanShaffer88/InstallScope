//! Monitor tab: snapshot before, install, snapshot after, review, save.

use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::decrap::{self, Component, Verdict};
use installscope_core::i18n::{t, tf};
use installscope_core::model::{human_size, InstallLog};
use installscope_core::platform::{self, ChildProcess};
use installscope_core::snapshot::{detect_program, diff, filter_changes, take_snapshot};
use installscope_core::store::{PendingSession, Store};

#[derive(PartialEq, Eq, Default, Clone, Copy, Debug)]
pub enum Phase {
    #[default]
    Idle,
    Before,
    Installing,
    After,
    Review,
}

#[derive(Default)]
pub struct MonitorState {
    /// Screening (and optional sandbox test) before installing.
    pub precheck: Option<super::precheck::PreCheck>,
    pub tree: super::difftree_ui::TreeView,
    pub show_list: bool,
    pub phase: Phase,
    pub name: String,
    pub installer: String,
    pub started: String,
    pub before_job: Option<Job<Result<(), String>>>,
    pub after_job: Option<Job<Result<InstallLog, String>>>,
    pub child: Option<ChildProcess>,
    pub review: Option<InstallLog>,
    /// Saving the reviewed log (large logs take a moment to write).
    pub save_job: Option<Job<Result<InstallLog, (InstallLog, String)>>>,
    pub error: String,
    /// "Try it, then undo": everything recorded is removed after review.
    pub trial: bool,
    /// Installer Decrapifier checklist for the log under review: each
    /// background component and whether it is ticked to be struck.
    pub decrap: Option<Vec<(Component, bool)>>,
    pub decrap_job: Option<Job<Result<Vec<Component>, String>>>,
    pub decrap_note: String,
}

impl MonitorState {
    pub fn is_active(&self) -> bool {
        self.phase != Phase::Idle
    }

    /// A monitoring session survives restarts (installers that need a reboot).
    pub fn resume_pending(&mut self, store: &Store) {
        if let Some(p) = store.pending() {
            self.phase = Phase::Installing;
            self.name = p.name;
            self.installer = p.installer;
            self.started = p.started;
            self.trial = p.trial;
        }
    }

    pub fn cancel(&mut self, store: &Store) {
        if let Some(j) = &self.before_job {
            j.cancel();
        }
        if let Some(j) = &self.after_job {
            j.cancel();
        }
        store.clear_pending();
        *self = MonitorState::default();
    }
}

/// Note under the Installer Decrapifier checklist in Monitor Review:
/// what happens to the ticked components when the log is saved, stated
/// for the backup configuration actually in force. The save job honors
/// `Settings::backup_enabled()` — with backups off nothing is backed
/// up, so the note must warn that the strike cannot be undone instead
/// of claiming a backup (audit P1-4).
fn strike_note(backup_on: bool) -> String {
    if backup_on {
        t("Ticked items are removed when you save the log, and backed up first, so the removal can be undone from the Backups page.")
    } else {
        t("Backups are off in Settings: ticked items are removed when you save the log without a backup, so the removal cannot be undone.")
    }
}

/// Body and action for the confirmation Monitor Review's Discard
/// button asks for (audit P1-2). Discarding deletes the only record of
/// what the install changed — the reviewed log was never saved, and
/// the pending session with its "before" snapshot is cleared — so the
/// click routes through `App::ask` with this text and never discards
/// directly.
fn discard_prompt(name: &str) -> (String, Confirm) {
    (
        tf(
            "The only record of everything {name} installed will be deleted, including the saved \"before\" snapshot. {name} stays installed, but without this log InstallScope cannot remove exactly what the install added, and the install cannot be undone from it.",
            &[("name", name.to_string())],
        ),
        Confirm::DiscardReview,
    )
}

impl App {
    pub fn start_install_and_monitor(&mut self, ctx: &egui::Context) {
        if self.monitor.is_active() {
            return;
        }
        let Some(path) = pick_file(
            &t("Choose an installer"),
            &t("Installers"),
            &["exe", "msi", "msix", "appx", "bat", "cmd"],
        ) else {
            return;
        };
        self.precheck_installer(ctx, path.to_string_lossy().to_string());
    }

    /// Record an installation. A `trial` is removed again right after review
    /// ("Try it, then undo", for PCs without Windows Sandbox).
    pub fn begin_monitoring(
        &mut self,
        ctx: &egui::Context,
        name: String,
        installer: String,
        trial: bool,
    ) {
        let reg = self.reg.clone();
        let settings = self.settings.clone();
        let root = self.store.root.clone();
        let session = PendingSession {
            name: name.clone(),
            started: installscope_core::util::now_string(),
            installer: installer.clone(),
            trial,
        };
        let point = tf("Before trying {name}", &[("name", name.clone())]);
        self.monitor = MonitorState {
            phase: Phase::Before,
            name,
            installer,
            started: session.started.clone(),
            trial,
            ..Default::default()
        };
        self.monitor.before_job = Some(Job::spawn(ctx, move |pr| {
            if trial {
                // A second safety net besides the log and the backup. Best
                // effort: System Protection may be off, or one was made today.
                pr.set_phase(t("Creating a restore point…"));
                let _ = installscope_core::restore::create_restore_point(&point);
            }
            let snap = take_snapshot(reg.as_ref(), &settings, pr).map_err(|e| e.to_string())?;
            let store = Store::at(&root).map_err(|e| e.to_string())?;
            store
                .save_pending(&session, &snap)
                .map_err(|e| e.to_string())
        }));
    }

    fn finish_monitoring(&mut self, ctx: &egui::Context) {
        let reg = self.reg.clone();
        let settings = self.settings.clone();
        let root = self.store.root.clone();
        let (name, installer) = (self.monitor.name.clone(), self.monitor.installer.clone());
        self.monitor.phase = Phase::After;
        self.monitor.child = None;
        self.monitor.after_job = Some(Job::spawn(ctx, move |pr| {
            let store = Store::at(&root).map_err(|e| e.to_string())?;
            pr.set_phase(t("Loading the first snapshot"));
            let before = store.pending_snapshot().map_err(|e| e.to_string())?;
            let after = take_snapshot(reg.as_ref(), &settings, pr).map_err(|e| e.to_string())?;
            pr.set_phase(t("Comparing"));
            let mut changes = diff(&before, &after);
            filter_changes(
                &mut changes,
                &settings.effective_registry_exclusions(),
                &settings.effective_file_exclusions(),
            );
            let mut log = InstallLog {
                id: installscope_core::util::new_id(),
                name,
                created: installscope_core::util::now_string(),
                installer,
                ..Default::default()
            };
            if let Some(d) = detect_program(&changes) {
                log.name = d.name;
                log.publisher = d.publisher;
                log.version = d.version;
                log.uninstall_key = d.key;
            }
            log.changes = changes;
            Ok(log)
        }));
    }

    pub fn poll_monitor(&mut self, ctx: &egui::Context) {
        if let Some(r) = self.monitor.decrap_job.as_ref().and_then(|j| j.poll()) {
            self.monitor.decrap_job = None;
            match r {
                Ok(judged) => {
                    if let Some(list) = &mut self.monitor.decrap {
                        for (c, ticked) in list.iter_mut() {
                            if let Some(n) =
                                judged.iter().find(|n| n.kind == c.kind && n.name == c.name)
                            {
                                if c.verdict == Verdict::Unknown && n.verdict != Verdict::Unknown {
                                    *ticked = n.verdict.strike_by_default();
                                }
                                *c = n.clone();
                            }
                        }
                    }
                    self.monitor.decrap_note.clear();
                }
                Err(e) => self.monitor.decrap_note = e,
            }
        }
        if let Some(r) = self.monitor.save_job.as_ref().and_then(|j| j.poll()) {
            self.monitor.save_job = None;
            match r {
                Ok(log) => {
                    let trial = self.monitor.trial;
                    self.store.clear_pending();
                    self.monitor = Default::default();
                    self.reload_logs();
                    self.programs.needs_refresh = true;
                    if trial {
                        self.undo_trial(&log);
                    } else {
                        self.set_status(tf(
                            "Saved install log for {name}",
                            &[("name", log.name.clone())],
                        ));
                        self.logs.selected = Some(log.id);
                    }
                }
                Err((log, e)) => {
                    self.monitor.review = Some(log);
                    self.monitor.error = e;
                }
            }
        }
        if let Some(r) = self.monitor.before_job.as_ref().and_then(|j| j.poll()) {
            self.monitor.before_job = None;
            match r {
                Ok(()) => {
                    self.monitor.phase = Phase::Installing;
                    let inst = self.monitor.installer.clone();
                    if !inst.is_empty() {
                        let lower = inst.to_lowercase();
                        let cmd = if lower.ends_with(".msi") {
                            format!("msiexec.exe /i \"{inst}\"")
                        } else if lower.ends_with(".msix") || lower.ends_with(".appx") {
                            platform::shell_open(&inst);
                            String::new()
                        } else {
                            format!("\"{inst}\"")
                        };
                        if !cmd.is_empty() {
                            match platform::spawn_command_line(&cmd) {
                                Ok(c) => self.monitor.child = Some(c),
                                Err(e) => self.monitor.error = e.to_string(),
                            }
                        }
                    }
                }
                Err(e) => {
                    self.monitor.cancel(&self.store);
                    self.monitor.error = e;
                }
            }
        }
        if let Some(r) = self.monitor.after_job.as_ref().and_then(|j| j.poll()) {
            self.monitor.after_job = None;
            match r {
                Ok(log) => {
                    self.monitor.phase = Phase::Review;
                    self.monitor.review = Some(log);
                }
                Err(e) => {
                    self.monitor.phase = Phase::Installing;
                    self.monitor.error = e;
                }
            }
        }
        if self.monitor.phase == Phase::Installing && self.monitor.child.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_secs(1));
        }
    }

    pub fn ui_monitor(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            if !self.monitor.error.is_empty() {
                ui.colored_label(theme::BAD, &self.monitor.error);
                ui.add_space(6.0);
            }
            match self.monitor.phase {
                Phase::Idle => {
                    self.profile_picker(ui, false);
                    self.monitor_idle(ui, ctx)
                }
                Phase::Before | Phase::After => {
                    let job_p = self
                        .monitor
                        .before_job
                        .as_ref()
                        .map(|j| j.progress.clone())
                        .or_else(|| self.monitor.after_job.as_ref().map(|j| j.progress.clone()));
                    ui.add_space(40.0);
                    ui.vertical_centered(|ui| {
                        ui.heading(if self.monitor.phase == Phase::Before {
                            t("Taking the \"before\" snapshot…")
                        } else {
                            t("Taking the \"after\" snapshot and comparing…")
                        });
                        ui.add_space(8.0);
                        if let Some(p) = job_p {
                            job_progress(ui, &p.phase(), p.count());
                        }
                        small_muted(ui, t("Don't install anything until this finishes."));
                        ui.add_space(10.0);
                        if ui.button(t("Cancel")).clicked() {
                            self.monitor.cancel(&self.store);
                        }
                    });
                }
                Phase::Installing => {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("⏺").size(36.0).color(theme::ACCENT_TEXT));
                        ui.heading(tf("Monitoring: {name}", &[("name", self.monitor.name.clone())]));
                        small_muted(ui, tf("Started {name}", &[("name", self.monitor.started.clone())]));
                        ui.add_space(10.0);
                        let running = self.monitor.child.as_ref().is_some_and(|c| c.try_wait().is_none());
                        if running {
                            ui.horizontal(|ui| {
                                super::widgets::spinner(ui);
                                ui.label(t("The installer is running."));
                            });
                        }
                        ui.label(t("Install the program now, and run it once if you like, so first-run settings are captured too."));
                        ui.label(t("When everything is finished, click Finish. You can close InstallScope or restart Windows in between; monitoring resumes."));
                        ui.add_space(14.0);
                        ui.horizontal(|ui| {
                            ui.add_space((ui.available_width() - 260.0).max(0.0) / 2.0);
                            if theme::primary_button(ui, format!("✔ {}", t("Finish monitoring"))).clicked() {
                                self.monitor.error.clear();
                                self.finish_monitoring(ctx);
                            }
                            if ui.button(t("Cancel")).clicked() {
                                self.ask(
                                    "Cancel monitoring?",
                                    t("The \"before\" snapshot will be discarded. Anything already installed stays installed."),
                                    "Cancel monitoring",
                                    Confirm::CancelMonitoring,
                                );
                            }
                        });
                    });
                }
                Phase::Review => self.monitor_review(ui),
            }
            if self.monitor.phase == Phase::Installing {
                ui.add_space(16.0);
                self.ui_live(ui);
            } else {
                self.live = None;
            }
        });
    }

    fn ui_live(&mut self, ui: &mut egui::Ui) {
        use installscope_core::live::{LiveKind, LiveWatcher};
        if self.live.is_none() {
            let folders: Vec<(std::path::PathBuf, bool)> = self
                .settings
                .file_roots
                .iter()
                .map(|r| {
                    (
                        std::path::PathBuf::from(installscope_core::platform::expand_env(&r.path)),
                        r.recursive,
                    )
                })
                .filter(|(p, _)| p.is_dir())
                .collect();
            let excl = installscope_core::util::Exclusions::new(
                &self.settings.effective_file_exclusions(),
            );
            self.live = Some(LiveWatcher::start(
                &folders,
                &self.settings.registry_roots,
                excl,
            ));
        }
        let Some(live) = &self.live else { return };
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
        egui::CollapsingHeader::new(RichText::new(t("Live activity")).strong())
            .default_open(self.settings.advanced_mode)
            .show(ui, |ui| {
                let (a, c, r, g) = live.counts();
                small_muted(ui, tf(
                    "Files added: {a} · changed: {c} · removed: {r} · registry areas changed: {g}",
                    &[("a", a.to_string()), ("c", c.to_string()), ("r", r.to_string()), ("g", g.to_string())],
                ));
                small_muted(ui, t("Changes appear here as they happen. The exact list, including registry values, is compared when you click Finish."));
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .stick_to_bottom(true)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        for e in live.recent(300) {
                            let (sym, color) = match e.kind {
                                LiveKind::Added => ("+", theme::ADDED),
                                LiveKind::Changed | LiveKind::Renamed => ("~", theme::CHANGED),
                                LiveKind::Removed => ("−", theme::REMOVED),
                                LiveKind::Registry => ("®", theme::MUTED),
                            };
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&e.time).monospace().small().color(theme::MUTED));
                                ui.label(RichText::new(sym).monospace().color(color));
                                ui.add(egui::Label::new(RichText::new(&e.path).monospace().small()).truncate());
                            });
                        }
                    });
            });
    }

    fn monitor_idle(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if self.ui_precheck(ui, ctx) {
            return;
        }
        ui.add_space(20.0);
        ui.vertical_centered(|ui| {
            ui.heading(RichText::new(t("Install with monitoring")).strong());
            ui.add_space(4.0);
            muted(ui, t("InstallScope records every file, folder, registry key, service and scheduled task an installation adds or changes."));
            muted(ui, t("Later it can remove exactly those changes, and nothing else."));
        });
        ui.add_space(20.0);
        ui.columns(2, |cols| {
            egui::Frame::group(cols[0].style()).inner_margin(16).show(&mut cols[0], |ui| {
                ui.set_min_height(150.0);
                ui.label(RichText::new(format!("📦 {}", t("Install and monitor"))).strong().size(16.0));
                small_muted(ui, t("Choose an installer. InstallScope takes a snapshot, runs it, and waits for you to finish."));
                ui.add_space(10.0);
                if theme::primary_button(ui, t("Choose installer…")).clicked() {
                    self.start_install_and_monitor(ctx);
                }
            });
            egui::Frame::group(cols[1].style()).inner_margin(16).show(&mut cols[1], |ui| {
                ui.set_min_height(150.0);
                ui.label(RichText::new(format!("📡 {}", t("Monitor manually"))).strong().size(16.0));
                small_muted(ui, t("Start monitoring, then install anything any way you like (Store, winget, a script)."));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(t("Name"));
                    ui.add(egui::TextEdit::singleline(&mut self.monitor.name).hint_text(t("e.g. Photo Editor")).desired_width(200.0));
                });
                ui.add_space(4.0);
                if ui.button(t("Start monitoring")).clicked() {
                    let name = if self.monitor.name.trim().is_empty() { t("Unnamed program") } else { self.monitor.name.trim().to_string() };
                    self.begin_monitoring(ctx, name, String::new(), false);
                }
            });
        });
        ui.add_space(16.0);
        let roots = self.settings.registry_roots.len() + self.settings.file_roots.len();
        small_muted(
            ui,
            tf(
                "Snapshots cover {n} locations (change them in Settings).",
                &[("n", roots.to_string())],
            ),
        );
    }

    fn monitor_review(&mut self, ui: &mut egui::Ui) {
        if self.monitor.save_job.is_some() {
            job_progress(ui, &t("Saving…"), 0);
            return;
        }
        let trial = self.monitor.trial;
        if self.monitor.decrap.is_none() {
            if let Some(log) = &self.monitor.review {
                let ctx = decrap::context_for(log);
                let comps = decrap::components(&log.changes, &ctx, &decrap::read_task_command);
                self.monitor.decrap = Some(
                    comps
                        .into_iter()
                        .map(|c| {
                            let tick = c.verdict.strike_by_default();
                            (c, tick)
                        })
                        .collect(),
                );
            }
        }
        let model_url = self.settings.local_model_url.clone();
        let model_name = self.settings.local_model_name.clone();
        // Captured as the review renders (the save job captures it again
        // at click time), so the strike note below states the backup
        // configuration that will actually apply.
        let backup_on = self.settings.backup_enabled();
        let Some(log) = &mut self.monitor.review else {
            return;
        };
        let st = log.changes.stats();
        ui.horizontal(|ui| {
            ui.heading(t("Review the installation"));
        });
        if trial {
            theme::warning_banner(
                ui,
                t("Trial install: after you review it, everything it added is removed."),
            );
        }
        egui::Grid::new("review_meta")
            .num_columns(2)
            .spacing([10.0, 6.0])
            .show(ui, |ui| {
                ui.label(t("Name"));
                ui.add(egui::TextEdit::singleline(&mut log.name).desired_width(320.0));
                ui.end_row();
                ui.label(t("Publisher"));
                ui.add(egui::TextEdit::singleline(&mut log.publisher).desired_width(320.0));
                ui.end_row();
                ui.label(t("Version"));
                ui.add(egui::TextEdit::singleline(&mut log.version).desired_width(160.0));
                ui.end_row();
                ui.label(t("Notes"));
                ui.add(egui::TextEdit::singleline(&mut log.notes).desired_width(320.0));
                ui.end_row();
            });
        ui.horizontal_wrapped(|ui| {
            theme::badge(
                ui,
                tf("{n} files", &[("n", st.files_added.to_string())]),
                theme::ADDED,
            );
            theme::badge(
                ui,
                tf("{n} folders", &[("n", st.dirs_added.to_string())]),
                theme::ADDED,
            );
            theme::badge(
                ui,
                tf("{n} keys", &[("n", st.keys_added.to_string())]),
                theme::ADDED,
            );
            theme::badge(
                ui,
                tf("{n} values", &[("n", st.values_added.to_string())]),
                theme::ADDED,
            );
            theme::badge(
                ui,
                tf(
                    "{n} changed",
                    &[("n", (st.values_modified + st.files_modified).to_string())],
                ),
                theme::CHANGED,
            );
            theme::badge(ui, human_size(st.bytes_added), theme::MUTED);
        });
        let mut ask_model = false;
        if let (false, Some(list)) = (trial, self.monitor.decrap.as_mut()) {
            if !list.is_empty() {
                ui.add_space(6.0);
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Background components")).heading());
                    let bloat = list
                        .iter()
                        .filter(|(c, _)| c.verdict == Verdict::Bloat)
                        .count();
                    small_muted(
                        ui,
                        tf(
                            "{name} added {n} things that run in the background. {b} look like telemetry, silent updaters or advertising and are ticked to be struck.",
                            &[
                                ("name", log.name.clone()),
                                ("n", list.len().to_string()),
                                ("b", bloat.to_string()),
                            ],
                        ),
                    );
                    ui.add_space(4.0);
                    for (c, ticked) in list.iter_mut() {
                        ui.horizontal_wrapped(|ui| {
                            ui.checkbox(ticked, "")
                                .on_hover_text(t("Strike: remove it when the log is saved"));
                            let (col, label) = match c.verdict {
                                Verdict::Bloat => {
                                    (theme::BAD, t("Bloatware / telemetry / silent updater"))
                                }
                                Verdict::Optional => (theme::WARN, t("Optional")),
                                Verdict::Required => (theme::OK, t("Required for the program")),
                                Verdict::Unknown => (theme::MUTED, t("Unknown")),
                            };
                            theme::badge(ui, label, col);
                            small_muted(ui, t(c.kind.label()));
                            ui.label(RichText::new(&c.name).strong()).on_hover_text(
                                if c.command.is_empty() {
                                    c.location.clone()
                                } else {
                                    c.command.clone()
                                },
                            );
                        });
                        if let Some(r) = c.reasons.first() {
                            small_muted(ui, r);
                        }
                    }
                    ui.add_space(4.0);
                    let unknown = list.iter().any(|(c, _)| c.verdict == Verdict::Unknown);
                    ui.horizontal_wrapped(|ui| {
                        if unknown && !model_url.is_empty() {
                            let busy = self.monitor.decrap_job.is_some();
                            if ui
                                .add_enabled(!busy, egui::Button::new(t("Ask the local model about the unknown ones")))
                                .clicked()
                            {
                                ask_model = true;
                            }
                            if busy {
                                spinner(ui);
                            }
                        } else if unknown {
                            small_muted(ui, t("A language model on this PC can judge the unknown ones: set it in Settings."));
                        }
                    });
                    if !self.monitor.decrap_note.is_empty() {
                        theme::warning_banner(ui, self.monitor.decrap_note.clone());
                    }
                    if backup_on {
                        small_muted(ui, strike_note(true));
                    } else {
                        theme::warning_banner(ui, strike_note(false));
                    }
                });
            }
        }
        if ask_model {
            let comps: Vec<Component> = self
                .monitor
                .decrap
                .as_ref()
                .map(|l| l.iter().map(|(c, _)| c.clone()).collect())
                .unwrap_or_default();
            let (app, publisher) = (log.name.clone(), log.publisher.clone());
            self.monitor.decrap_job = Some(Job::spawn(&self.ctx, move |_| {
                let mut comps = comps;
                installscope_core::llm::judge_unknowns(
                    &model_url,
                    &model_name,
                    &app,
                    &publisher,
                    &mut comps,
                )
                .map(|_| comps)
                .map_err(|e| e.to_string())
            }));
        }
        small_muted(ui, t("Remove anything that isn't part of this program (background noise) with ✖ before saving."));
        ui.separator();
        let h = ui.available_height() - 44.0;
        egui::ScrollArea::vertical()
            .max_height(h)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                super::difftree_ui::map_or_list(
                    ui,
                    &mut self.monitor.tree,
                    &mut self.monitor.show_list,
                    &mut log.changes,
                    true,
                );
            });
        ui.separator();
        // The name as reviewed (it is editable above), for the Discard
        // confirmation text.
        let log_name = log.name.clone();
        let mut save = false;
        let mut discard = false;
        let mut keep = false;
        ui.horizontal(|ui| {
            if trial {
                if theme::primary_button(ui, format!("↩ {}", t("Undo the trial"))).clicked() {
                    save = true;
                }
                // Keeping it means a normal install log, so it can still be
                // removed precisely later. No Discard: that would leave the
                // program installed with no record of what it added.
                if ui.button(t("Keep it installed")).clicked() {
                    keep = true;
                }
            } else {
                if theme::primary_button(ui, format!("💾 {}", t("Save install log"))).clicked() {
                    save = true;
                }
                if ui.button(t("Discard")).clicked() {
                    discard = true;
                }
            }
        });
        if keep {
            self.monitor.trial = false;
            save = true;
        }
        if save && self.monitor.save_job.is_none() {
            let mut log = self.monitor.review.take().unwrap();
            let store = self.store.clone();
            // Struck background components go first (backed up first
            // when backups are on), unless
            // this is a trial being undone, which removes everything anyway.
            let chosen: Vec<Component> = if self.monitor.trial {
                Vec::new()
            } else {
                self.monitor
                    .decrap
                    .as_ref()
                    .map(|l| {
                        l.iter()
                            .filter(|(_, t)| *t)
                            .map(|(c, _)| c.clone())
                            .collect()
                    })
                    .unwrap_or_default()
            };
            let reg = self.reg.clone();
            let backup_on = self.settings.backup_enabled();
            self.monitor.save_job = Some(Job::spawn(&self.ctx, move |_| {
                if !chosen.is_empty() {
                    let mut writer = if backup_on {
                        match installscope_core::backup::BackupWriter::new(
                            &store.backups_dir(),
                            &format!("Strike background items of {}", log.name),
                        ) {
                            Ok(writer) => Some(writer),
                            Err(error) => {
                                return Err((
                                    log,
                                    format!(
                                        "Could not create a backup; selected components were not removed or saved: {error:#}"
                                    ),
                                ));
                            }
                        }
                    } else {
                        None
                    };
                    let refs: Vec<&Component> = chosen.iter().collect();
                    let results = decrap::strike(&mut log, &refs, reg.as_ref(), writer.as_mut());
                    if let Some(w) = writer {
                        if let Err(error) = w.finish() {
                            return Err((
                                log,
                                format!(
                                    "Selected components may have been removed, but the backup could not be finalized: {error:#}"
                                ),
                            ));
                        }
                    }
                    let struck: Vec<&str> = chosen
                        .iter()
                        .filter(|c| {
                            !results
                                .iter()
                                .any(|r| !r.ok && r.description.contains(&c.name))
                        })
                        .map(|c| c.name.as_str())
                        .collect();
                    if !struck.is_empty() {
                        if !log.notes.is_empty() {
                            log.notes.push_str("; ");
                        }
                        log.notes
                            .push_str(&format!("Struck at install: {}", struck.join(", ")));
                    }
                }
                match store.save_log(&log) {
                    Ok(()) => Ok(log),
                    Err(e) => Err((log, e.to_string())),
                }
            }));
        } else if discard {
            // Never discard straight from the click: ask first, with
            // what will be lost stated (audit P1-2). Confirming runs
            // the same monitor.cancel() as before via run_confirmed.
            let (body, action) = discard_prompt(&log_name);
            self.ask("Discard install log?", body, "Discard", action);
        }
    }
}

impl App {
    /// Remove everything a trial install added: the exact log, with files
    /// created since and without Simple mode's trimming (every item was
    /// made minutes ago by this installer). Protected locations are still
    /// refused, and everything is backed up as usual.
    pub(super) fn undo_trial(&mut self, log: &InstallLog) {
        let plan = installscope_core::revert::build_plan(&log.changes, self.reg.as_ref());
        let mut flow = super::plan::RemovalFlow::new(
            tf("Undo the trial of {name}", &[("name", log.name.clone())]),
            plan,
            true,
            true,
        );
        flow.trial = true;
        flow.logs_to_delete = vec![log.id.clone()];
        flow.names = vec![log.name.clone()];
        self.open_removal(flow);
    }
}

#[cfg(test)]
mod tests {
    use super::{discard_prompt, strike_note, Confirm};

    // Audit P1-4: the Monitor Review strike note must describe the
    // backup configuration actually in force — the save job honors
    // Settings::backup_enabled(), so with backups off the old
    // unconditional "backed up first" line was false.

    #[test]
    fn strike_note_only_claims_a_backup_when_backups_are_on() {
        let on = strike_note(true);
        assert!(on.contains("backed up first"), "{on}");
        // The undo path is named.
        assert!(on.contains("Backups page"), "{on}");
        let off = strike_note(false);
        assert!(off.contains("Backups are off"), "{off}");
        assert!(off.contains("without a backup"), "{off}");
        assert!(off.contains("cannot be undone"), "{off}");
        assert!(!off.contains("backed up first"), "{off}");
    }

    // Audit P1-2: Discard never runs straight from the click; it asks
    // through the confirmation system, and the text states what will
    // be lost.

    #[test]
    fn discard_prompt_asks_and_states_what_will_be_lost() {
        let (body, action) = discard_prompt("Foo Player");
        // The click routes to the DiscardReview confirmation, whose
        // classification (asks per the user's setting, dialog starts
        // on Cancel) is pinned in app.rs's confirm_tests.
        assert!(matches!(action, Confirm::DiscardReview));
        assert!(body.contains("Foo Player"), "{body}");
        assert!(body.contains("only record"), "{body}");
        assert!(body.contains("stays installed"), "{body}");
        assert!(
            body.contains("cannot remove exactly what the install added"),
            "{body}"
        );
        assert!(body.contains("cannot be undone"), "{body}");
    }
}
