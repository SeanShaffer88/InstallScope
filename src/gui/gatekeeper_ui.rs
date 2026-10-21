//! Autostart Gatekeeper page (Utilities): an opt-in review queue for programs
//! that start with Windows.
//!
//! Detection compares the live Run/RunOnce values and Startup-folder entries
//! with a saved baseline. Anything new or changed waits here until the person
//! decides: **Keep** (the entry stays exactly as it is) or **Remove** (deleted
//! through the same backed-up path the Startup page uses). The Gatekeeper never
//! blocks an installer, never watches in the background, and never removes
//! anything without that decision — a scan only fills the queue.
//!
//! Every control explains itself the same three ways as the rest of the app
//! (see `a11y::explain`): as a hover tooltip, on the page's focus line, and as
//! the description screen readers announce.

use super::a11y;
use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::decrap::Verdict;
use installscope_core::gatekeeper::{self as gk, Attribution, ItemState, Queue};
use installscope_core::i18n::{t, tf};

/// What one scan found, and the queue as it was saved.
pub struct ScanDone {
    pub queue: Queue,
    pub added: usize,
    pub changed: usize,
}

/// A decision taken while drawing the list. Carrying it out after the page
/// has finished drawing keeps the queue from changing underneath the rows
/// that are still being laid out.
enum Act {
    Keep(String),
    Remove(String),
}

#[derive(Default)]
pub struct GatekeeperState {
    /// The queue as last read from, or written to, `gatekeeper.json`.
    pub queue: Option<Queue>,
    /// Why the queue could not be read or scanned, shown instead of the list.
    pub error: Option<String>,
    pub load_job: Option<Job<Result<Queue, String>>>,
    pub scan_job: Option<Job<Result<ScanDone, String>>>,
    /// A Keep or Remove running in the background; carries the status line's
    /// message (Ok) or the error to report (Err).
    pub op_job: Option<Job<Result<(String, Queue), String>>>,
    /// Show the entries that already have a decision.
    pub show_decided: bool,
}

impl GatekeeperState {
    /// Entries still waiting for a decision.
    pub fn pending(&self) -> usize {
        self.queue.as_ref().map_or(0, |q| q.pending().len())
    }

    fn busy(&self) -> bool {
        self.load_job.is_some() || self.scan_job.is_some() || self.op_job.is_some()
    }
}

/// Where an entry starts, in plain words.
fn location_text(e: &gk::WatchedEntry) -> String {
    let kind = if e.kind == gk::KIND_FOLDER {
        t("Startup folder")
    } else {
        t("Run key")
    };
    let who = if e.all_users {
        t("all users")
    } else {
        t("your account")
    };
    format!("{kind} · {who}")
}

/// The Decrapifier's verdict as a coloured badge, marked as well as coloured
/// so it never relies on colour alone.
fn verdict_badge(ui: &mut egui::Ui, verdict: Option<Verdict>) {
    let Some(v) = verdict else {
        return;
    };
    let (mark, color) = match v {
        Verdict::Required => ("✔", theme::OK),
        Verdict::Optional => ("•", theme::WARN),
        Verdict::Bloat => ("⚠", theme::BAD),
        Verdict::Unknown => ("?", theme::MUTED),
    };
    theme::badge(ui, format!("{mark} {}", t(v.label())), color);
}

/// Who most likely added the entry, with the evidence stated honestly.
fn attribution_ui(ui: &mut egui::Ui, attribution: &Attribution) {
    match attribution {
        Attribution::Log {
            program, evidence, ..
        } => {
            ui.label(
                RichText::new(tf("Added by {program}", &[("program", program.clone())])).strong(),
            );
            small_muted(ui, evidence.clone());
        }
        Attribution::Unknown => {
            small_muted(
                ui,
                t("No install log explains this entry, so where it came from is unknown."),
            );
        }
    }
}

impl App {
    /// Read the queue from disk in the background.
    pub fn refresh_gatekeeper(&mut self) {
        if self.gatekeeper.load_job.is_some() {
            return;
        }
        let store = self.store.clone();
        self.gatekeeper.error = None;
        self.gatekeeper.load_job = Some(Job::spawn(&self.ctx, move |_| {
            Queue::load(&store).map_err(|e| format!("{e:#}"))
        }));
    }

    /// One detection pass over the live autostart set. Filling the queue is
    /// all a scan does; nothing is removed.
    pub fn scan_gatekeeper(&mut self) {
        if !self.settings.gatekeeper_enabled {
            self.set_status(t(
                "The Autostart Gatekeeper is off, so nothing was scanned. Turn it on first.",
            ));
            return;
        }
        if self.gatekeeper.busy() {
            return;
        }
        let store = self.store.clone();
        let reg = self.reg.clone();
        self.gatekeeper.error = None;
        self.gatekeeper.scan_job = Some(Job::spawn(&self.ctx, move |pr| {
            pr.set_phase(t("Reading the programs that start with Windows"));
            let mut queue = Queue::load(&store).map_err(|e| format!("{e:#}"))?;
            let current: Vec<gk::WatchedEntry> =
                installscope_core::startup::list_startup(reg.as_ref())
                    .iter()
                    .map(gk::WatchedEntry::from)
                    .collect();
            let logs = store.list_logs();
            let summary = gk::run_scan(&mut queue, &current, &logs);
            queue.save(&store).map_err(|e| format!("{e:#}"))?;
            Ok(ScanDone {
                added: summary.added.len(),
                changed: summary.changed.len(),
                queue,
            })
        }));
    }

    /// Record the decision to keep an entry. The entry itself is untouched;
    /// the baseline takes it in, so later scans stop asking about it.
    pub fn keep_gatekeeper_item(&mut self, id: &str) {
        if self.gatekeeper.op_job.is_some() {
            return;
        }
        let store = self.store.clone();
        let id = id.to_string();
        self.gatekeeper.op_job = Some(Job::spawn(&self.ctx, move |_| {
            let mut queue = Queue::load(&store).map_err(|e| format!("{e:#}"))?;
            let item = queue.allow(&id).map_err(|e| format!("{e:#}"))?;
            queue.save(&store).map_err(|e| format!("{e:#}"))?;
            let message = tf(
                "Kept \"{name}\". It stays exactly as it is, and future scans will not ask about it again.",
                &[("name", item.entry.name)],
            );
            Ok((message, queue))
        }));
    }

    /// Remove a queued entry, backed up first. The queue item only becomes
    /// removed once the entry really is gone.
    pub fn remove_gatekeeper_item(&mut self, id: &str) {
        if self.gatekeeper.op_job.is_some() {
            return;
        }
        let store = self.store.clone();
        let reg = self.reg.clone();
        let dir = self.store.backups_dir();
        let id = id.to_string();
        self.gatekeeper.op_job = Some(Job::spawn(&self.ctx, move |pr| {
            pr.set_phase(t("Removing the autostart entry"));
            let mut queue = Queue::load(&store).map_err(|e| format!("{e:#}"))?;
            let (item, backup) = gk::remove_entry(&mut queue, &id, reg.as_ref(), &dir)
                .map_err(|e| format!("{e:#}"))?;
            queue.save(&store).map_err(|e| format!("{e:#}"))?;
            let message = tf(
                "Removed \"{name}\". Its backup is on the Backups page ({backup}), so this can be undone.",
                &[("name", item.entry.name), ("backup", backup)],
            );
            Ok((message, queue))
        }));
    }

    /// Ask before removing an entry, stating what will happen to it and how
    /// it can be undone — including when backups are off.
    fn confirm_gatekeeper_remove(&mut self, item: &installscope_core::gatekeeper::QueueItem) {
        let backup = if self.settings.backup_enabled() {
            t("A backup is taken first, so it can be put back from the Backups page.")
        } else {
            t("Backups are off in Settings, so no backup is taken first and this cannot be undone from InstallScope.")
        };
        let body = tf(
            "\"{name}\" ({command}) will be deleted, so it no longer starts with Windows.\r\n\r\n{backup}\r\n\r\nIf you would rather it stayed, choose Cancel and click Keep instead.",
            &[
                ("name", item.entry.name.clone()),
                ("command", item.entry.command.clone()),
                ("backup", backup),
            ],
        );
        self.ask(
            "Remove this autostart entry?",
            body,
            "Remove",
            Confirm::GatekeeperRemove(item.id.clone()),
        );
    }

    pub fn poll_gatekeeper(&mut self) {
        if let Some(result) = self.gatekeeper.load_job.as_ref().and_then(|j| j.poll()) {
            self.gatekeeper.load_job = None;
            match result {
                Ok(queue) => {
                    self.gatekeeper.queue = Some(queue);
                    self.gatekeeper.error = None;
                }
                Err(error) => self.gatekeeper.error = Some(error),
            }
        }
        if let Some(result) = self.gatekeeper.scan_job.as_ref().and_then(|j| j.poll()) {
            self.gatekeeper.scan_job = None;
            match result {
                Ok(done) => {
                    let waiting = done.queue.pending().len();
                    self.gatekeeper.queue = Some(done.queue);
                    self.set_status(tf(
                        "Scan complete: {new} new, {changed} changed, {waiting} waiting for your decision.",
                        &[
                            ("new", done.added.to_string()),
                            ("changed", done.changed.to_string()),
                            ("waiting", waiting.to_string()),
                        ],
                    ));
                }
                Err(error) => self.gatekeeper.error = Some(error),
            }
        }
        if let Some(result) = self.gatekeeper.op_job.as_ref().and_then(|j| j.poll()) {
            self.gatekeeper.op_job = None;
            match result {
                Ok((message, queue)) => {
                    self.gatekeeper.queue = Some(queue);
                    self.set_status(message);
                    // A removal adds a backup to the Backups page.
                    self.reload_backups();
                }
                Err(error) => self.set_status(error),
            }
        }
    }

    pub fn ui_gatekeeper(&mut self, ctx: &egui::Context) {
        if self.gatekeeper.queue.is_none()
            && self.gatekeeper.load_job.is_none()
            && self.gatekeeper.scan_job.is_none()
            && self.gatekeeper.error.is_none()
        {
            self.refresh_gatekeeper();
        }
        let mut act: Option<Act> = None;
        let mut scan = false;
        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ctx, |ui| {
                // The focused control's explanation, for keyboard and touch
                // users (see a11y::explain).
                a11y::focus_hint(ui, theme::MUTED);
                let queue = self.gatekeeper.queue.clone();
                let pending = queue.as_ref().map_or(0, |q| q.pending().len());
                let decided = queue.as_ref().map_or(0, |q| q.items.len() - q.pending().len());
                let busy = self.gatekeeper.busy();

                theme::card(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        if self.settings.gatekeeper_enabled {
                            theme::badge(ui, t("On"), theme::OK);
                        } else {
                            theme::badge(ui, t("Off"), theme::MUTED);
                        }
                        let waiting = if pending == 1 {
                            t("1 entry waiting for your decision")
                        } else {
                            tf(
                                "{n} entries waiting for your decision",
                                &[("n", pending.to_string())],
                            )
                        };
                        ui.label(RichText::new(waiting).strong());
                    });
                    ui.add_space(6.0);
                    ui.horizontal_wrapped(|ui| {
                        let mut on = self.settings.gatekeeper_enabled;
                        let switched = a11y::explain(
                            ui.checkbox(&mut on, t("Watch for new autostart entries")),
                            &t("Compare the programs that start with Windows with a saved baseline, and keep anything new in this review queue. Nothing is blocked while it installs and nothing is removed without your decision. A scan runs only when you ask for one here, with the button below, or with \"installscope gatekeeper scan\"."),
                        );
                        if switched.changed() {
                            self.settings.gatekeeper_enabled = on;
                            self.save_settings();
                            if on {
                                // First scan straight away, so the queue is
                                // not empty for no visible reason.
                                scan = true;
                            }
                        }
                        ui.with_layout(
                            egui::Layout::right_to_left(egui::Align::Center),
                            |ui| {
                                let can_scan = self.settings.gatekeeper_enabled && !busy;
                                let why = if !self.settings.gatekeeper_enabled {
                                    t("Turn the Gatekeeper on first, then InstallScope can look for new autostart entries.")
                                } else if busy {
                                    t("InstallScope is already working on the queue.")
                                } else {
                                    t("Compare the programs that start with Windows with the saved baseline now. New or changed entries are added to the queue; nothing is removed.")
                                };
                                let button = ui.add_enabled(
                                    can_scan,
                                    egui::Button::new(format!("⟳ {}", t("Scan now"))),
                                );
                                if a11y::explain(button, &why).clicked() {
                                    scan = true;
                                }
                            },
                        );
                    });
                    ui.add_space(4.0);
                    small_muted(ui, t("The Gatekeeper looks at Run and RunOnce entries and Startup-folder shortcuts. Services and scheduled tasks stay on the Startup page, and each entry here waits until you decide."));
                    if busy {
                        let mut phase = self
                            .gatekeeper
                            .scan_job
                            .as_ref()
                            .map(|j| j.progress.phase())
                            .or_else(|| {
                                self.gatekeeper
                                    .op_job
                                    .as_ref()
                                    .map(|j| j.progress.phase())
                            })
                            .or_else(|| {
                                self.gatekeeper
                                    .load_job
                                    .as_ref()
                                    .map(|j| j.progress.phase())
                            })
                            .unwrap_or_default();
                        if phase.trim().is_empty() {
                            phase = t("Working…");
                        }
                        job_progress(ui, &phase, 0);
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_millis(200));
                    }
                });
                if let Some(error) = self.gatekeeper.error.clone() {
                    ui.add_space(8.0);
                    theme::warning_banner(
                        ui,
                        tf(
                            "The Gatekeeper could not finish that: {error}",
                            &[("error", error)],
                        ),
                    );
                }
                ui.add_space(8.0);
                if !self.settings.gatekeeper_enabled {
                    theme::card(ui, |ui| {
                        ui.label(RichText::new(t("The Gatekeeper is off")).strong());
                        muted(ui, t("Turn it on above to start watching for new startup entries. Until then this page stays empty and nothing is compared, so clicking Scan now does nothing."));
                    });
                    return;
                }
                let Some(queue) = queue else {
                    if !busy {
                        job_progress(ui, &t("Loading the review queue…"), 0);
                    }
                    return;
                };
                if pending == 0 {
                    theme::card(ui, |ui| {
                        ui.label(RichText::new(format!("✔ {}", t("Nothing is waiting for your decision"))).strong());
                        muted(ui, t("Click Scan now to compare the programs that start with Windows with the saved baseline. Anything new or changed since the last scan appears here for review."));
                    });
                } else {
                    ui.label(RichText::new(t("Waiting for your decision")).heading());
                    ui.add_space(4.0);
                    let mut items: Vec<&installscope_core::gatekeeper::QueueItem> =
                        queue.pending();
                    items.sort_by(|a, b| a.first_seen.cmp(&b.first_seen));
                    for item in items {
                        theme::card(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(RichText::new(&item.entry.name).strong());
                                        verdict_badge(ui, item.verdict);
                                        if !item.entry.enabled {
                                            theme::badge(ui, t("Switched off"), theme::MUTED);
                                        }
                                    });
                                    small_muted(
                                        ui,
                                        format!(
                                            "{} · {}",
                                            item.entry.command,
                                            location_text(&item.entry)
                                        ),
                                    );
                                    attribution_ui(ui, &item.attribution);
                                    if !item.note.is_empty() {
                                        small_muted(
                                            ui,
                                            tf(
                                                "Changed since the baseline: {note}",
                                                &[("note", item.note.clone())],
                                            ),
                                        );
                                    }
                                    if let Some(reason) = item.verdict_reasons.first() {
                                        small_muted(ui, reason.clone());
                                    }
                                    small_muted(
                                        ui,
                                        tf(
                                            "First seen {time}",
                                            &[("time", item.first_seen.clone())],
                                        ),
                                    );
                                });
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let remove = ui.button(
                                            RichText::new(format!("🗑 {}", t("Remove")))
                                                .color(theme::BAD),
                                        );
                                        if a11y::explain(
                                            remove,
                                            &tf(
                                                "Delete \"{name}\" from where it starts with Windows. You confirm first, and it is backed up first unless backups are off in Settings.",
                                                &[("name", item.entry.name.clone())],
                                            ),
                                        )
                                        .clicked()
                                        {
                                            act = Some(Act::Remove(item.id.clone()));
                                        }
                                        if a11y::explain(
                                            ui.button(t("Keep")),
                                            &tf(
                                                "Keep \"{name}\". It stays exactly as it is, and future scans will not ask about it again.",
                                                &[("name", item.entry.name.clone())],
                                            ),
                                        )
                                        .clicked()
                                        {
                                            act = Some(Act::Keep(item.id.clone()));
                                        }
                                    },
                                );
                            });
                        });
                        ui.add_space(6.0);
                    }
                }
                if decided > 0 {
                    ui.add_space(8.0);
                    let mut show = self.gatekeeper.show_decided;
                    let toggled = a11y::explain(
                        ui.checkbox(
                            &mut show,
                            tf(
                                "Show {n} entries you already decided",
                                &[("n", decided.to_string())],
                            ),
                        ),
                        &t("Show the entries you kept or removed, so you can check what the Gatekeeper did with them."),
                    );
                    if toggled.changed() {
                        self.gatekeeper.show_decided = show;
                    }
                    if self.gatekeeper.show_decided {
                        let mut items: Vec<&installscope_core::gatekeeper::QueueItem> = queue
                            .items
                            .iter()
                            .filter(|i| i.state != ItemState::Pending)
                            .collect();
                        items.sort_by(|a, b| b.last_seen.cmp(&a.last_seen));
                        for item in items {
                            ui.horizontal_wrapped(|ui| {
                                let (mark, color) = match item.state {
                                    ItemState::Allowed => ("✔", theme::OK),
                                    ItemState::Removed => ("🗑", theme::MUTED),
                                    ItemState::Pending => ("•", theme::MUTED),
                                };
                                ui.label(RichText::new(mark).color(color));
                                ui.label(RichText::new(&item.entry.name).strong());
                                if let Attribution::Log { program, .. } = &item.attribution {
                                    small_muted(
                                        ui,
                                        tf(
                                            "added by {program}",
                                            &[("program", program.clone())],
                                        ),
                                    );
                                }
                                small_muted(ui, format!("· {}", item.last_seen));
                                small_muted(ui, t(item.state.label()));
                            });
                        }
                    }
                }
            });
        if scan {
            self.scan_gatekeeper();
        }
        match act {
            Some(Act::Keep(id)) => self.keep_gatekeeper_item(&id),
            Some(Act::Remove(id)) => {
                let item = self
                    .gatekeeper
                    .queue
                    .as_ref()
                    .and_then(|q| q.items.iter().find(|i| i.id == id).cloned());
                if let Some(item) = item {
                    self.confirm_gatekeeper_remove(&item);
                }
            }
            None => {}
        }
    }
}
