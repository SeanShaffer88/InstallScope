//! Backups page: everything InstallScope removed, ready to be put back.

use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::backup::{self, ActionResult, Backup};
use installscope_core::i18n::{t, tf};

#[derive(Default)]
pub struct BackupsState {
    pub list: Vec<Backup>,
    pub job: Option<Job<Result<Vec<ActionResult>, String>>>,
    pub results: Option<Vec<ActionResult>>,
    pub loader: super::Reload<Vec<Backup>>,
}

impl App {
    pub fn restore_backup(&mut self, id: &str) {
        let reg = self.reg.clone();
        let root = self.store.backups_dir();
        let id = id.to_string();
        self.backups.results = None;
        self.backups.job = Some(Job::spawn(&self.ctx, move |_| {
            backup::restore_backup(&root, &id, reg.as_ref()).map_err(|e| e.to_string())
        }));
    }

    pub fn delete_backup(&mut self, id: &str) {
        // A backup can be a large folder: delete it in the background.
        let (dir, id) = (self.store.backups_dir(), id.to_string());
        self.backups.list.retain(|b| b.id != id);
        self.file_job = Some(Job::spawn(&self.ctx, move |_| {
            backup::delete_backup(&dir, &id).map_err(|e| e.to_string())
        }));
    }

    /// A card for operations that were cut off (crash, power loss) before
    /// they finished, with the choice to undo or keep what they did.
    pub fn interrupted_ui(&mut self, ui: &mut egui::Ui) {
        if self.backups.job.is_some() {
            return;
        }
        let list: Vec<(String, String, String)> = backup::interrupted(&self.backups.list)
            .into_iter()
            .map(|b| (b.id.clone(), b.title.clone(), b.created.clone()))
            .collect();
        for (id, title, created) in list {
            let mut undo = false;
            let mut keep = false;
            theme::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⚠").color(theme::WARN));
                    ui.label(RichText::new(t("An earlier operation didn't finish")).strong());
                });
                small_muted(
                    ui,
                    tf(
                        "{title}, started {created}, was interrupted, perhaps by a crash or a power cut. Part of it may already be done. Everything it changed so far is in its backup.",
                        &[("title", title.clone()), ("created", created.clone())],
                    ),
                );
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if super::a11y::explain(theme::primary_button(ui, t("Undo what was done")), &t("Undo everything this operation already changed, restoring it from the operation's backup.")).clicked() {
                        undo = true;
                    }
                    if super::a11y::explain(
                        ui.button(t("Keep as it is")),
                        &t("Keep the changes this operation already made and dismiss this warning. The backup stays on the Backups page, so you can still undo from there."),
                    )
                    .clicked()
                    {
                        keep = true;
                    }
                });
            });
            ui.add_space(8.0);
            if undo {
                self.restore_backup(&id);
            }
            if keep {
                let root = self.store.backups_dir();
                if let Err(e) = backup::mark_complete(&root, &id) {
                    self.status = e.to_string();
                }
                if let Some(b) = self.backups.list.iter_mut().find(|b| b.id == id) {
                    b.complete = true;
                }
            }
        }
    }

    pub fn poll_backups(&mut self) {
        if let Some(r) = self.backups.job.as_ref().and_then(|j| j.poll()) {
            self.backups.job = None;
            match r {
                Ok(results) => {
                    let failed = results.iter().filter(|r| !r.ok).count();
                    self.status = if failed == 0 {
                        t("Backup restored.")
                    } else {
                        tf("Finished with {n} problems.", &[("n", failed.to_string())])
                    };
                    self.backups.results = Some(results);
                    self.programs.needs_refresh = true;
                    self.startup.loaded = false;
                }
                Err(e) => self.status = e,
            }
            self.reload_backups();
        }
    }

    pub fn ui_backups(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            if self.backups.job.is_some() {
                job_progress(ui, &t("Restoring…"), 0);
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            self.interrupted_ui(ui);
            if let Some(results) = &self.backups.results {
                let mut close = false;
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Restore finished")).strong());
                    results_ui(ui, results);
                    if ui.button(t("Close")).clicked() {
                        close = true;
                    }
                });
                if close {
                    self.backups.results = None;
                }
                ui.add_space(10.0);
            }
            let total: u64 = self.backups.list.iter().map(|b| b.bytes()).sum();
            ui.horizontal(|ui| {
                muted(ui, tf("{n} backups", &[("n", self.backups.list.len().to_string())]));
                muted(ui, format!("· {}", installscope_core::model::human_size(total)));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(t("Open backup folder")).clicked() {
                        installscope_core::platform::shell_open(&self.store.backups_dir().to_string_lossy());
                    }
                });
            });
            ui.add_space(6.0);
            if self.backups.list.is_empty() && self.backups.loader.busy() {
                job_progress(ui, &t("Loading…"), 0);
                return;
            }
            if self.backups.list.is_empty() {
                theme::card(ui, |ui| {
                    muted(ui, t("No backups yet. Every removal InstallScope makes is backed up here first, unless you have turned backups off in Settings."));
                });
                return;
            }
            let mut action: Option<Confirm> = None;
            let backups_dir = self.store.backups_dir();
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for b in &self.backups.list {
                    theme::card(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.vertical(|ui| {
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(&b.title).strong());
                                    if b.restored {
                                        theme::badge(ui, t("Restored"), theme::OK);
                                    }
                                });
                                small_muted(ui, format!("{}  ·  {}", b.created, b.summary()));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui
                                    .button(t("Open folder"))
                                    .on_hover_text(t("Backed-up files, plus a .reg file you can double-click to restore registry entries without InstallScope."))
                                    .clicked()
                                {
                                    installscope_core::platform::shell_open(&backups_dir.join(&b.id).to_string_lossy());
                                }
                                if ui.button(t("Delete")).clicked() {
                                    action = Some(Confirm::DeleteBackup(b.id.clone()));
                                }
                                if theme::primary_button(ui, format!("↺ {}", t("Restore"))).clicked() {
                                    action = Some(Confirm::RestoreBackup(b.id.clone()));
                                }
                            });
                        });
                    });
                    ui.add_space(6.0);
                }
            });
            match action {
                Some(Confirm::RestoreBackup(id)) => self.ask(
                    "Restore backup?",
                    t("Everything in this backup will be put back where it was."),
                    "Restore",
                    Confirm::RestoreBackup(id),
                ),
                Some(Confirm::DeleteBackup(id)) => self.ask(
                    "Delete backup?",
                    t("This backup will be deleted permanently and can no longer be restored."),
                    "Delete",
                    Confirm::DeleteBackup(id),
                ),
                _ => {}
            }
        });
    }
}
