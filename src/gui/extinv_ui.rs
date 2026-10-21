//! Browser extensions page (Tools): inventory of installed extensions,
//! grouped by browser and profile, with reversible Disable/Restore.
//!
//! Disabling moves the extension's files into SetupSweep's data folder and
//! records the exact original location; restoring moves them back. Nothing
//! is ever deleted, and the profile's other data is never touched.

use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use setupsweep_core::extinv::{self, InstalledExtension};
use setupsweep_core::i18n::{t, tf};

#[derive(Default)]
pub struct ExtInvState {
    pub items: Option<Vec<InstalledExtension>>,
    pub job: Option<Job<Vec<InstalledExtension>>>,
    /// A disable or restore running in the background; the String is the
    /// message for the status line (Ok) or an error to report (Err).
    pub op_job: Option<Job<Result<String, String>>>,
}

/// A stable key for one extension row: browser + profile dir + extension id.
fn key_of(e: &InstalledExtension) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        e.browser,
        e.profile_dir.display(),
        e.id
    )
}

impl App {
    pub fn scan_extensions(&mut self) {
        let root = self.store.root.clone();
        let ctx = self.ctx.clone();
        self.extinv.items = None;
        self.extinv.job = Some(Job::spawn(&ctx, move |pr| {
            pr.set_phase(t("Reading browser profiles"));
            extinv::discover(&root)
        }));
    }

    pub fn poll_extinv(&mut self) {
        if let Some(items) = self.extinv.job.as_ref().and_then(|j| j.poll()) {
            self.extinv.job = None;
            self.extinv.items = Some(items);
        }
        if let Some(result) = self.extinv.op_job.as_ref().and_then(|j| j.poll()) {
            self.extinv.op_job = None;
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
        let Some(item) = self.extinv_item(key) else {
            return;
        };
        let root = self.store.root.clone();
        let ctx = self.ctx.clone();
        self.extinv.op_job = Some(Job::spawn(&ctx, move |pr| {
            pr.set_phase(tf("Disabling {name}", &[("name", item.name.clone())]));
            match extinv::disable(&root, &item) {
                Ok(rec) => Ok(tf(
                    "{name} disabled; restore it here any time. (You may need to restart the browser.)",
                    &[("name", rec.name)],
                )),
                Err(e) => Err(format!("{:#}", e)),
            }
        }));
    }

    pub fn restore_extension(&mut self, key: &str) {
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
                    "{name} ({browser}, profile \"{profile}\") will stop working in the browser, but its files are kept by SetupSweep and it can be restored with one click.\r\n\r\nPlease close {browser} first; a running browser may lock files.",
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
                    "{name} ({browser}, profile \"{profile}\") will be put back exactly where it was.",
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
            if items.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("🧩").size(36.0));
                    ui.heading(t("No browser extensions found"));
                    muted(
                        ui,
                        t("SetupSweep looked through the profiles of the supported Chromium-based and Firefox-based browsers and found no extensions."),
                    );
                    ui.add_space(8.0);
                    if ui.button(format!("⟳ {}", t("Scan again"))).clicked() {
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
                        if ui.button(format!("⟳ {}", t("Scan again"))).clicked() {
                            self.scan_extensions();
                        }
                    });
                });
                small_muted(ui, t("Disabling an extension moves its files into SetupSweep's data folder; nothing is deleted and the rest of your profile is untouched. Restoring puts the files back exactly. Close the browser before disabling or restoring."));
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
            t("Disabled by SetupSweep")
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
                            if ui
                                .button(RichText::new(t("Restore")).strong())
                                .on_hover_text(t("Put this extension back exactly where it was"))
                                .clicked()
                            {
                                self.confirm_restore_extension(&key);
                            }
                        } else {
                            if ui
                                .button(t("Disable"))
                                .on_hover_text(t("Move this extension's files into SetupSweep's data folder; nothing is deleted"))
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
