//! System Restore page: restore points and System Protection settings.

use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use egui_extras::{Column, TableBuilder};
use installscope_core::i18n::{t, tf};
use installscope_core::model::human_size;
use installscope_core::restore::{self, Protection, RestorePoint};

/// Restore points (or why they couldn't be read) and System Protection.
type Loaded = (Result<Vec<RestorePoint>, String>, Protection);

#[derive(Default)]
pub struct RestoreState {
    pub loaded: bool,
    pub points: Option<Result<Vec<RestorePoint>, String>>,
    pub protection: Option<Protection>,
    pub load_job: Option<Job<Loaded>>,
    /// A create, delete or settings change in progress; returns a message.
    pub action_job: Option<Job<Result<String, String>>>,
    pub message: Option<Result<String, String>>,
    pub description: String,
    pub max_percent: u32,
}

impl App {
    fn refresh_restore(&mut self, ctx: &egui::Context) {
        self.restore.loaded = true;
        self.restore.load_job = Some(Job::spawn(ctx, |_| {
            (
                restore::list().map_err(|e| e.to_string()),
                restore::protection(),
            )
        }));
    }

    fn restore_action(
        &mut self,
        ctx: &egui::Context,
        f: impl FnOnce() -> Result<String, String> + Send + 'static,
    ) {
        self.restore.message = None;
        self.restore.action_job = Some(Job::spawn(ctx, move |_| f()));
    }

    pub fn delete_restore_point(&mut self, seq: u32) {
        let ctx = self.ctx.clone();
        self.restore_action(&ctx, move || {
            restore::delete(seq)
                .map(|_| t("Restore point deleted."))
                .map_err(|e| e.to_string())
        });
    }

    pub fn restore_to_point(&mut self, seq: u32) {
        let ctx = self.ctx.clone();
        self.restore_action(&ctx, move || {
            restore::restore_to(seq)
                .map(|_| t("Windows is restarting to restore your PC…"))
                .map_err(|e| e.to_string())
        });
    }

    pub fn set_protection(&mut self, on: bool) {
        let ctx = self.ctx.clone();
        self.restore_action(&ctx, move || {
            restore::set_protection(on)
                .map(|_| {
                    if on {
                        t("System Protection is on.")
                    } else {
                        t("System Protection is off.")
                    }
                })
                .map_err(|e| e.to_string())
        });
    }

    pub fn ui_restore(&mut self, ctx: &egui::Context) {
        if !self.restore.loaded {
            self.refresh_restore(ctx);
        }
        if let Some((p, prot)) = self.restore.load_job.as_ref().and_then(|j| j.poll()) {
            self.restore.load_job = None;
            if self.restore.max_percent == 0 {
                self.restore.max_percent = match (prot.max, prot.drive_size) {
                    (Some(m), Some(d)) if d > 0 => ((m * 100 / d) as u32).clamp(1, 50),
                    _ => 10,
                };
            }
            self.restore.points = Some(p);
            self.restore.protection = Some(prot);
        }
        if let Some(r) = self.restore.action_job.as_ref().and_then(|j| j.poll()) {
            self.restore.action_job = None;
            self.restore.message = Some(r);
            self.refresh_restore(ctx);
        }
        let advanced = self.settings.advanced_mode;
        let busy = self.restore.action_job.is_some();
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            if !self.elevated {
                theme::warning_banner(ui, t("Run InstallScope as Administrator to see and manage restore points."));
                ui.add_space(6.0);
            }
            match &self.restore.message {
                Some(Ok(m)) => {
                    ui.label(RichText::new(m).color(theme::OK));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(theme::BAD));
                }
                None => {}
            }
            // System Protection.
            let prot = self.restore.protection.clone();
            theme::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(t("System Protection")).strong());
                    match &prot {
                        Some(p) if p.enabled => theme::badge(ui, t("On"), theme::OK),
                        Some(_) => theme::badge(ui, t("Off"), theme::WARN),
                        None => {
                            spinner(ui);
                        }
                    }
                });
                if let Some(p) = &prot {
                    if !p.enabled {
                        small_muted(ui, t("Restore points can't be created while System Protection is off."));
                    }
                    if let (Some(used), Some(max)) = (p.used, p.max) {
                        small_muted(ui, tf("Restore points use {used} of up to {max}.", &[("used", human_size(used)), ("max", human_size(max))]));
                    }
                    if advanced {
                        ui.horizontal_wrapped(|ui| {
                            let label = if p.enabled { t("Turn off") } else { t("Turn on") };
                            if ui.add_enabled(!busy && self.elevated, egui::Button::new(label)).clicked() {
                                if p.enabled {
                                    self.ask(
                                        "Turn off System Protection?",
                                        t("Windows deletes all existing restore points and stops creating new ones. InstallScope can then no longer create a restore point before changes."),
                                        "Turn off",
                                        Confirm::SetProtection(false),
                                    );
                                } else {
                                    self.set_protection(true);
                                }
                            }
                            ui.add_space(12.0);
                            ui.label(t("Maximum disk space:"));
                            ui.add(egui::Slider::new(&mut self.restore.max_percent, 1..=30).suffix(" %"));
                            if let Some(d) = p.drive_size {
                                small_muted(ui, human_size(d * self.restore.max_percent as u64 / 100));
                            }
                            if ui.add_enabled(!busy && self.elevated, egui::Button::new(t("Apply"))).clicked() {
                                let pct = self.restore.max_percent;
                                self.restore_action(ctx, move || {
                                    restore::set_max_space(pct)
                                        .map(|_| t("The disk space limit was changed."))
                                        .map_err(|e| e.to_string())
                                });
                            }
                        });
                        small_muted(ui, t("When the limit is reached, Windows deletes the oldest restore points to make room."));
                    }
                }
            });
            ui.add_space(8.0);
            // Create.
            theme::card(ui, |ui| {
                ui.label(RichText::new(t("Create a restore point")).strong());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut self.restore.description).hint_text(t("Description, e.g. Before installing drivers")).desired_width(320.0));
                    let ok = !busy && self.elevated && prot.as_ref().is_some_and(|p| p.enabled);
                    if ui.add_enabled(ok, egui::Button::new(t("Create"))).clicked() {
                        let d = if self.restore.description.trim().is_empty() {
                            "InstallScope manual restore point".to_string()
                        } else {
                            self.restore.description.trim().to_string()
                        };
                        self.restore.description.clear();
                        self.restore_action(ctx, move || {
                            restore::create_restore_point(&d)
                                .map(|_| t("Restore point created."))
                                .map_err(|e| e.to_string())
                        });
                    }
                    if ui.button(t("Open Windows System Restore")).clicked() {
                        restore::open_wizard();
                    }
                });
            });
            ui.add_space(8.0);
            if busy {
                job_progress(ui, &t("Working…"), 0);
            }
            ui.horizontal(|ui| {
                ui.label(RichText::new(t("Restore points")).heading());
                if ui.add_enabled(self.restore.load_job.is_none(), egui::Button::new(format!("⟳ {}", t("Refresh")))).clicked() {
                    self.refresh_restore(ctx);
                }
            });
            let points = match &self.restore.points {
                None => {
                    job_progress(ui, &t("Loading…"), 0);
                    return;
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(theme::BAD));
                    return;
                }
                Some(Ok(p)) => p.clone(),
            };
            if points.is_empty() {
                muted(ui, t("There are no restore points."));
                return;
            }
            let mut delete = None;
            let mut roll_back = None;
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::initial(140.0))
                .column(Column::remainder().at_least(200.0).clip(true))
                .column(Column::initial(140.0))
                .column(Column::exact(180.0))
                .header(24.0, |mut h| {
                    for l in ["Created", "Description", "Type", ""] {
                        h.col(|ui| {
                            ui.strong(t(l));
                        });
                    }
                })
                .body(|body| {
                    body.rows(26.0, points.len(), |mut row| {
                        let p = &points[row.index()];
                        row.col(|ui| {
                            ui.label(&p.created);
                        });
                        row.col(|ui| {
                            ui.add(egui::Label::new(&p.description).truncate());
                        });
                        row.col(|ui| {
                            ui.label(t(p.kind.label()));
                        });
                        row.col(|ui| {
                            if ui.add_enabled(!busy && self.elevated, egui::Button::new(t("Restore…")).small()).clicked() {
                                roll_back = Some(p.clone());
                            }
                            if advanced && ui.add_enabled(!busy && self.elevated, egui::Button::new(t("Delete")).small()).clicked() {
                                delete = Some(p.clone());
                            }
                        });
                    });
                });
            if let Some(p) = roll_back {
                self.ask(
                    "Restore your PC to this point?",
                    tf(
                        "Windows will restart and return system files, settings and installed programs to how they were on {date} ({name}). Your documents, photos and other personal files are not changed. Programs installed since then may need to be installed again. Save your work first; the restart happens straight away.",
                        &[("date", p.created.clone()), ("name", p.description.clone())],
                    ),
                    "Restore and restart",
                    Confirm::RestoreTo(p.sequence),
                );
            }
            if let Some(p) = delete {
                self.ask(
                    "Delete this restore point?",
                    tf("The restore point from {date} ({name}) will be deleted. This can't be undone.", &[("date", p.created.clone()), ("name", p.description.clone())]),
                    "Delete",
                    Confirm::DeleteRestorePoint(p.sequence),
                );
            }
        });
    }
}
