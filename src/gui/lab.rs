//! Snapshot compare: capture the system at any moment and compare any two
//! captures.

use super::app::App;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::i18n::{t, tf};
use installscope_core::model::InstallLog;
use installscope_core::platform;
use installscope_core::snapshot::{
    detect_program, diff, filter_changes, load_snapshot, save_snapshot, take_snapshot,
};

#[derive(Default)]
pub struct LabState {
    snap_name: String,
    snap_job: Option<Job<Result<String, String>>>,
    /// Saved snapshots (name, path, date), read when needed rather than every frame.
    snaps: Option<Vec<(String, std::path::PathBuf, String)>>,
    cmp_a: Option<usize>,
    cmp_b: Option<usize>,
    cmp_job: Option<Job<Result<InstallLog, String>>>,
    cmp_result: Option<InstallLog>,
    message: String,
    /// Quick compare: the "before" snapshot, kept in memory.
    quick_before: Option<std::sync::Arc<installscope_core::model::Snapshot>>,
    quick_job: Option<Job<Result<std::sync::Arc<installscope_core::model::Snapshot>, String>>>,
    tree: super::difftree_ui::TreeView,
    show_list: bool,
}

impl App {
    pub fn poll_lab(&mut self) {
        if let Some(r) = self.lab.snap_job.as_ref().and_then(|j| j.poll()) {
            self.lab.snap_job = None;
            self.lab.snaps = None;
            self.lab.message = match r {
                Ok(name) => tf("Saved snapshot {name}", &[("name", name)]),
                Err(e) => e,
            };
        }
        if let Some(r) = self.lab.quick_job.as_ref().and_then(|j| j.poll()) {
            self.lab.quick_job = None;
            match r {
                Ok(s) => self.lab.quick_before = Some(s),
                Err(e) => self.lab.message = e,
            }
        }
        if let Some(r) = self.lab.cmp_job.as_ref().and_then(|j| j.poll()) {
            self.lab.cmp_job = None;
            match r {
                Ok(log) => self.lab.cmp_result = Some(log),
                Err(e) => self.lab.message = e,
            }
        }
    }

    pub fn ui_lab(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.ui_lab_snapshots(ui));
            });
    }

    /// Before/after in two clicks, without naming or saving snapshots.
    fn ui_quick_compare(&mut self, ui: &mut egui::Ui) {
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("Quick before and after")).strong());
            small_muted(ui, t("Snapshot now, then run, use or install something (and close it), then snapshot again to see exactly what it left behind."));
            ui.add_space(6.0);
            let busy = self.lab.quick_job.is_some() || self.lab.cmp_job.is_some();
            ui.horizontal(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    let label = if self.lab.quick_before.is_some() {
                        format!("📷 {}", t("1. Snapshot again (start over)"))
                    } else {
                        format!("📷 {}", t("1. Snapshot before"))
                    };
                    if ui.button(label).clicked() {
                        let reg = self.reg.clone();
                        let settings = self.settings.clone();
                        self.lab.quick_before = None;
                        self.lab.cmp_result = None;
                        self.lab.quick_job = Some(Job::spawn(&self.ctx, move |pr| {
                            take_snapshot(reg.as_ref(), &settings, pr)
                                .map(std::sync::Arc::new)
                                .map_err(|e| e.to_string())
                        }));
                    }
                });
                ui.add_enabled_ui(!busy && self.lab.quick_before.is_some(), |ui| {
                    if theme::primary_button(
                        ui,
                        format!("🔍 {}", t("2. Snapshot after and compare")),
                    )
                    .clicked()
                    {
                        let before = self.lab.quick_before.clone().unwrap();
                        let reg = self.reg.clone();
                        let settings = self.settings.clone();
                        self.lab.cmp_result = None;
                        self.lab.cmp_job = Some(Job::spawn(&self.ctx, move |pr| {
                            let after = take_snapshot(reg.as_ref(), &settings, pr)
                                .map_err(|e| e.to_string())?;
                            let mut changes = diff(&before, &after);
                            filter_changes(
                                &mut changes,
                                &settings.effective_registry_exclusions(),
                                &settings.effective_file_exclusions(),
                            );
                            let mut log = InstallLog {
                                id: installscope_core::util::new_id(),
                                name: format!("{} {}", t("Changes since"), before.created),
                                created: installscope_core::util::now_string(),
                                ..Default::default()
                            };
                            if let Some(d) = detect_program(&changes) {
                                log.name = d.name.clone();
                                log.publisher = d.publisher;
                                log.version = d.version;
                                log.uninstall_key = d.key;
                            }
                            log.changes = changes;
                            Ok(log)
                        }));
                    }
                });
            });
            if let Some(j) = &self.lab.quick_job {
                job_progress(ui, &j.progress.phase(), j.progress.count());
            } else if let Some(j) = &self.lab.cmp_job {
                job_progress(ui, &j.progress.phase(), j.progress.count());
            } else if let Some(b) = &self.lab.quick_before {
                small_muted(
                    ui,
                    tf(
                        "Before: {time}. Now make the changes you want to see, then click 2.",
                        &[("time", b.created.clone())],
                    ),
                );
            }
        });
    }

    /// Re-read the snapshot folder next time the list is shown.
    pub fn on_lab_opened(&mut self) {
        self.lab.snaps = None;
    }

    fn ui_lab_snapshots(&mut self, ui: &mut egui::Ui) {
        if self.lab.snaps.is_none() {
            self.lab.snaps = Some(self.store.list_snapshots());
        }
        let snaps = self.lab.snaps.clone().unwrap_or_default();
        self.profile_picker(ui, false);
        self.ui_quick_compare(ui);
        ui.add_space(8.0);
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("Take a named snapshot")).strong());
            small_muted(ui, t("Capture the system now, do anything, capture again, and compare any two snapshots."));
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.lab.snap_name)
                        .hint_text(t("e.g. before-build-42"))
                        .desired_width(240.0),
                );
                ui.add_enabled_ui(self.lab.snap_job.is_none(), |ui| {
                    if theme::primary_button(ui, format!("📷 {}", t("Take snapshot"))).clicked() {
                        let name = if self.lab.snap_name.trim().is_empty() {
                            format!(
                                "snapshot-{}",
                                installscope_core::time::DateTime::now_local().compact()
                            )
                        } else {
                            installscope_core::util::safe_file_name(self.lab.snap_name.trim())
                        };
                        let path = self.store.snapshots_dir().join(format!("{name}.snap"));
                        let reg = self.reg.clone();
                        let settings = self.settings.clone();
                        self.lab.snap_job = Some(Job::spawn(&self.ctx, move |pr| {
                            let s = take_snapshot(reg.as_ref(), &settings, pr)
                                .map_err(|e| e.to_string())?;
                            save_snapshot(&path, &s).map_err(|e| e.to_string())?;
                            Ok(name)
                        }));
                    }
                });
            });
            if let Some(j) = &self.lab.snap_job {
                job_progress(ui, &j.progress.phase(), j.progress.count());
            }
            if !self.lab.message.is_empty() {
                small_muted(ui, &self.lab.message);
            }
        });
        ui.add_space(8.0);
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("Compare two snapshots")).strong());
            if snaps.len() < 2 {
                small_muted(ui, t("Take at least two snapshots to compare them."));
                return;
            }
            let label = |i: Option<usize>| {
                i.and_then(|i| snaps.get(i))
                    .map(|s| format!("{}  ({})", s.0, s.2))
                    .unwrap_or_else(|| t("Choose…"))
            };
            ui.horizontal(|ui| {
                ui.label(t("Before"));
                egui::ComboBox::from_id_salt("cmp_a")
                    .width(260.0)
                    .selected_text(label(self.lab.cmp_a))
                    .show_ui(ui, |ui| {
                        for (i, s) in snaps.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.lab.cmp_a,
                                Some(i),
                                format!("{}  ({})", s.0, s.2),
                            );
                        }
                    });
                ui.label(t("After"));
                egui::ComboBox::from_id_salt("cmp_b")
                    .width(260.0)
                    .selected_text(label(self.lab.cmp_b))
                    .show_ui(ui, |ui| {
                        for (i, s) in snaps.iter().enumerate() {
                            ui.selectable_value(
                                &mut self.lab.cmp_b,
                                Some(i),
                                format!("{}  ({})", s.0, s.2),
                            );
                        }
                    });
            });
            let ready = self.lab.cmp_a.is_some()
                && self.lab.cmp_b.is_some()
                && self.lab.cmp_a != self.lab.cmp_b
                && self.lab.cmp_job.is_none();
            ui.horizontal(|ui| {
                ui.add_enabled_ui(ready, |ui| {
                    if theme::primary_button(ui, t("Compare")).clicked() {
                        let a = snaps[self.lab.cmp_a.unwrap()].clone();
                        let b = snaps[self.lab.cmp_b.unwrap()].clone();
                        let settings = self.settings.clone();
                        self.lab.cmp_result = None;
                        self.lab.cmp_job = Some(Job::spawn(&self.ctx, move |_| {
                            let before = load_snapshot(&a.1).map_err(|e| e.to_string())?;
                            let after = load_snapshot(&b.1).map_err(|e| e.to_string())?;
                            let mut changes = diff(&before, &after);
                            filter_changes(
                                &mut changes,
                                &settings.effective_registry_exclusions(),
                                &settings.effective_file_exclusions(),
                            );
                            let mut log = InstallLog {
                                id: installscope_core::util::new_id(),
                                name: format!("{} → {}", a.0, b.0),
                                created: installscope_core::util::now_string(),
                                ..Default::default()
                            };
                            if let Some(d) = detect_program(&changes) {
                                log.publisher = d.publisher;
                                log.version = d.version;
                                log.uninstall_key = d.key;
                            }
                            log.changes = changes;
                            Ok(log)
                        }));
                    }
                });
                if ui.button(t("Open snapshot folder")).clicked() {
                    platform::shell_open(&self.store.snapshots_dir().to_string_lossy());
                }
            });
            if self.lab.cmp_job.is_some() {
                job_progress(ui, &t("Comparing"), 0);
            }
        });
        if self.lab.cmp_result.is_some() {
            ui.add_space(8.0);
            let mut save = false;
            let mut export_ini = false;
            theme::card(ui, |ui| {
                let log = self.lab.cmp_result.as_mut().unwrap();
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&log.name).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(t("Save as install log")).clicked() {
                            save = true;
                        }
                        if ui
                            .button(t("PortableApps.com Launcher settings…"))
                            .on_hover_text(t("Save what changed as [RegistryKeys], [DirectoriesMove] and [FilesMove] sections for a PortableApps.com Launcher .ini. Remove unrelated changes with ✖ first."))
                            .clicked()
                        {
                            export_ini = true;
                        }
                    });
                });
                super::difftree_ui::map_or_list(
                    ui,
                    &mut self.lab.tree,
                    &mut self.lab.show_list,
                    &mut log.changes,
                    true,
                );
            });
            if export_ini {
                let log = self.lab.cmp_result.as_ref().unwrap();
                let r = installscope_core::portable::launcher_ini(
                    &log.changes,
                    &log.name,
                    &installscope_core::portable::Folders::current(),
                );
                let name = format!("{}.ini", installscope_core::util::safe_file_name(&log.name));
                if let Some(p) = save_file(&t("Save Launcher settings"), &name, "INI", "ini") {
                    self.lab.message = match std::fs::write(&p, r.text) {
                        Ok(()) => tf(
                            "Saved {path}: {dirs} folders, {files} files and {keys} registry keys to move.",
                            &[
                                ("path", p.display().to_string()),
                                ("dirs", r.directories.to_string()),
                                ("files", r.files.to_string()),
                                ("keys", r.keys.to_string()),
                            ],
                        ),
                        Err(e) => e.to_string(),
                    };
                }
            }
            if save {
                let log = self.lab.cmp_result.take().unwrap();
                match self.store.save_log(&log) {
                    Ok(()) => {
                        self.reload_logs();
                        self.lab.message =
                            tf("Saved install log for {name}", &[("name", log.name)]);
                    }
                    Err(e) => self.lab.message = e.to_string(),
                }
            }
        }
    }
}
