//! Recognized browser extensions with best-effort same-volume file holding.
//! Durable receipts support recovery; browser sync or policy may recreate files.

use super::a11y;
use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use setupscope_core::extinv::{self, InstalledExtension};
use setupscope_core::i18n::{t, tf};

#[derive(Default)]
pub struct ExtInvState {
    pub items: Option<Vec<InstalledExtension>>,
    pub job: Option<Job<ExtensionScan>>,
    pub receipt_error: Option<String>,
    pub operation_message: Option<Result<String, String>>,
    /// A disable or restore running in the background; the String is the
    /// message for the status line (Ok) or an error to report (Err).
    pub op_job: Option<Job<Result<String, String>>>,
}
pub type ExtensionScan = (Vec<InstalledExtension>, Option<String>);

/// A stable key for one extension row: browser + profile dir + extension id.
fn key_of(e: &InstalledExtension) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}\u{1f}{}",
        e.browser,
        e.profile_dir.display(),
        e.id,
        e.held.as_ref().map(|r| r.held_path.as_str()).unwrap_or("")
    )
}

impl App {
    pub fn scan_extensions(&mut self) {
        if self.extinv.job.is_some() || self.extinv.op_job.is_some() {
            return;
        }
        let root = self.store.root.clone();
        let ctx = self.ctx.clone();
        self.extinv.items = None;
        self.extinv.job = Some(Job::spawn(&ctx, move |pr| {
            pr.set_phase(t("Reading browser profiles"));
            let problem = extinv::recovery_problem(&root);
            (extinv::discover(&root), problem)
        }));
    }

    pub fn poll_extinv(&mut self) {
        if let Some((items, problem)) = self.extinv.job.as_ref().and_then(|j| j.poll()) {
            self.extinv.job = None;
            self.extinv.items = Some(items);
            self.extinv.receipt_error = problem;
        }
        if let Some(result) = self.extinv.op_job.as_ref().and_then(|j| j.poll()) {
            self.extinv.op_job = None;
            self.extinv.operation_message = Some(result.clone());
            match result {
                Ok(msg) => self.set_status(msg),
                Err(err) => self.set_status(err),
            }
            // Re-scan so the held/enabled states are fresh.
            self.extinv.items = None;
        }
    }

    pub fn extinv_item(&self, key: &str) -> Option<InstalledExtension> {
        self.extinv
            .items
            .as_ref()?
            .iter()
            .find(|e| key_of(e) == key)
            .cloned()
    }

    pub fn disable_extension(&mut self, key: &str) {
        if self.extinv.job.is_some() || self.extinv.op_job.is_some() { return; }
        let Some(item) = self.extinv_item(key) else {
            return;
        };
        let root = self.store.root.clone();
        let ctx = self.ctx.clone();
        self.extinv.op_job = Some(Job::spawn(&ctx, move |pr| {
            pr.set_phase(tf("Disabling {name}", &[("name", item.name.clone())]));
            match extinv::disable(&root, &item) {
                Ok(rec) => Ok(tf(
                    "{name} files moved to the holding area. Keep SetupScope data for recovery; browser sync or policy may recreate files.",
                    &[("name", rec.name)],
                )),
                Err(e) => Err(format!("{:#}", e)),
            }
        }));
    }

    pub fn restore_extension(&mut self, key: &str) {
        if self.extinv.job.is_some() || self.extinv.op_job.is_some() { return; }
        let Some(item) = self.extinv_item(key) else {
            return;
        };
        let Some(rec) = item.held.clone() else {
            return;
        };
        let root = self.store.root.clone();
        let ctx = self.ctx.clone();
        self.extinv.op_job = Some(Job::spawn(&ctx, move |pr| {
            pr.set_phase(tf("Restoring {name}", &[("name", rec.name.clone())]));
            match extinv::restore(&root, &rec) {
                Ok(()) => Ok(tf(
                    "{name} restored. (You may need to restart the browser.)",
                    &[("name", rec.name)],
                )),
                Err(e) => Err(format!("{:#}", e)),
            }
        }));
    }

    fn confirm_disable_extension(&mut self, key: &str) {
        if let Some(item) = self.extinv_item(key) {
            self.ask(
                "Disable extension",
                tf(
                    "{name} ({browser}, profile \"{profile}\") will have its extension files moved to SetupScope holding storage on the same volume. Keep that storage for recovery. Browser sync, policy or repair may recreate the extension; disabling is not guaranteed.\r\n\r\nPlease close {browser} first; a running browser may lock files.",
                    &[
                        ("name", item.name),
                        ("browser", item.browser),
                        ("profile", item.profile),
                    ],
                ),
                "Disable",
                Confirm::DisableExtension(key.to_string()),
            );
        }
    }

    fn confirm_restore_extension(&mut self, key: &str) {
        if let Some(item) = self.extinv_item(key) {
            self.ask(
                "Restore extension",
                tf(
                    "{name} ({browser}, profile \"{profile}\") will have its held files moved back to the original path only if it is vacant. Close the browser first. Browser settings may still require manual review.",
                    &[
                        ("name", item.name),
                        ("browser", item.browser),
                        ("profile", item.profile),
                    ],
                ),
                "Restore",
                Confirm::RestoreExtension(key.to_string()),
            );
        }
    }

    pub fn ui_extinv(&mut self, ctx: &egui::Context) {
        if self.extinv.items.is_none() && self.extinv.job.is_none() && self.extinv.op_job.is_none()
        {
            self.scan_extensions();
        }
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            if self.extinv.job.is_some() || self.extinv.op_job.is_some() {
                ui.add_space(30.0);
                let phase = self
                    .extinv
                    .job
                    .as_ref()
                    .map(|j| j.progress.phase())
                    .or_else(|| self.extinv.op_job.as_ref().map(|j| j.progress.phase()))
                    .unwrap_or_default();
                ui.vertical_centered(|ui| job_progress(ui, &t(&phase), 0));
                return;
            }
            let items = self.extinv.items.clone().unwrap_or_default();
            a11y::focus_hint(ui, theme::MUTED);
            if let Some(message) = &self.extinv.operation_message {
                match message { Ok(message) => { ui.label(message); }, Err(error) => { ui.colored_label(theme::BAD, error); } }
            }
            if let Some(problem) = &self.extinv.receipt_error {
                ui.colored_label(theme::BAD, problem);
                ui.label(t("Disable/restore are blocked. Reconciliation changes receipt metadata only, never extension files. Corrupt or conflicting records require manual recovery; preserve the holding folder and journal."));
                ui.add(egui::Label::new(extinv::extinv_dir(&self.store.root).display().to_string()).selectable(true));
                if a11y::explain(ui.button(t("Reconcile interrupted receipt")), &t("Check the journal and source/held paths. If exactly one copy is available, update the local receipt to match it. No extension data moves or deletes; ambiguous or corrupt states remain blocked.")).clicked() {
                    let root = self.store.root.clone();
                    self.extinv.op_job = Some(Job::spawn(ctx, move |_| extinv::reconcile(&root).map(|_| t("Receipt reconciled; no extension files moved.")).map_err(|e| format!("{e:#}"))));
                }
            }
            if items.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🧩").size(36.0));
                    ui.heading(t("No browser extensions found"));
                    muted(
                        ui,
                        t("No extensions were listed in the checked profiles. Missing permissions, unsupported profiles or recovery errors can leave coverage incomplete."),
                    );
                    ui.add_space(8.0);
                    if a11y::explain(ui.button(format!("⟳ {}", t("Scan again"))), &t("Read the supported profiles and local recovery receipts again; no files move.")).clicked() {
                        self.scan_extensions();
                    }
                });
                return;
            }
            theme::card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(tf(
                            "{n} extensions in {b} browsers",
                            &[
                                ("n", items.len().to_string()),
                                (
                                    "b",
                                    items
                                        .iter()
                                        .map(|i| i.browser.clone())
                                        .collect::<std::collections::HashSet<_>>()
                                        .len()
                                        .to_string(),
                                ),
                            ],
                        ))
                        .strong(),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if a11y::explain(ui.button(format!("⟳ {}", t("Scan again"))), &t("Read the supported profiles and local recovery receipts again; no files move.")).clicked() {
                            self.scan_extensions();
                        }
                    });
                });
                small_muted(ui, t("Close the browser before a move. Holding extension files is not a guarantee of browser-disable state: sync, policy or browser repair can recreate them. Only same-volume moves are supported; no copy/delete fallback. Existing destinations and linked/reparse paths are refused. The journal preserves recovery intent after interruption. Other profile data is not edited."));
            });
            ui.add_space(8.0);
            // Group by browser, then profile.
            let mut browsers: Vec<&str> = items
                .iter()
                .map(|i| i.browser.as_str())
                .collect::<std::collections::HashSet<_>>()
                .into_iter()
                .collect();
            browsers.sort();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for browser in browsers {
                    ui.label(RichText::new(t(browser)).heading());
                    let mut profiles: Vec<&str> = items
                        .iter()
                        .filter(|i| i.browser == browser)
                        .map(|i| i.profile.as_str())
                        .collect::<std::collections::HashSet<_>>()
                        .into_iter()
                        .collect();
                    profiles.sort();
                    for profile in profiles {
                        ui.label(RichText::new(tf("Profile: {p}", &[("p", profile.to_string())])).strong());
                        for item in items.iter().filter(|i| i.browser == browser && i.profile == profile) {
                            self.ui_extension_row(ui, item);
                        }
                        ui.add_space(6.0);
                    }
                    ui.add_space(6.0);
                }
            });
        });
    }

    fn ui_extension_row(&mut self, ui: &mut egui::Ui, item: &InstalledExtension) {
        let key = key_of(item);
        let held = item.held.is_some();
        let state_text = if held {
            t("Held copy — restore available")
        } else {
            match item.enabled {
                Some(true) => t("Enabled"),
                Some(false) => t("Disabled in the browser"),
                None => t("State unknown"),
            }
        };
        let state_color = if held {
            theme::WARN
        } else if item.enabled == Some(true) {
            theme::OK
        } else {
            theme::MUTED
        };
        egui::Frame::new()
            .fill(theme::CARD)
            .corner_radius(10.0)
            .inner_margin(egui::Margin::symmetric(12, 8))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(&item.name).strong());
                            theme::badge(ui, state_text, state_color);
                        });
                        let mut detail = format!("{}: {}", t("id"), item.id);
                        if !item.version.is_empty() {
                            detail = format!("v{} · {}", item.version, detail);
                        }
                        small_muted(ui, detail);
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if held {
                            if a11y::explain(ui.add_enabled(self.extinv.receipt_error.is_none(), egui::Button::new(RichText::new(t("Restore")).strong())), &t("Review restoring the held files to the recorded profile. Existing destinations are preserved. Recovery errors block this action."))
                                .clicked()
                            {
                                self.confirm_restore_extension(&key);
                            }
                        } else {
                            if a11y::explain(ui.add_enabled(self.extinv.receipt_error.is_none(), egui::Button::new(t("Disable"))), &t("Review holding this extension's files in SetupScope's data folder. Close the browser first; file moves do not guarantee browser-disable state. Recovery errors block this action."))
                                .clicked()
                            {
                                self.confirm_disable_extension(&key);
                            }
                        }
                    });
                });
            });
        ui.add_space(4.0);
    }
}
