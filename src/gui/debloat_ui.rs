//! Bloatware Shield page: one click to remove the trials, promotions and
//! pre-installed apps that come with new PCs, including Store apps and the
//! copies Windows keeps for new user accounts.

use super::app::App;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::debloat::{self, BloatItem, Class};
use installscope_core::i18n::{t, tf};
use std::collections::HashSet;

#[derive(Default)]
pub struct DebloatState {
    pub items: Option<Vec<BloatItem>>,
    pub job: Option<Job<Vec<BloatItem>>>,
    pub deprovision_job: Option<Job<Vec<String>>>,
    pub checked: HashSet<String>,
    pub keep_new_accounts_clean: bool,
}

impl App {
    pub fn scan_bloat(&mut self, ctx: &egui::Context) {
        let programs = self.programs.list.clone();
        let reg = self.reg.clone();
        let hidden = self.settings.system_components_shown();
        self.debloat.items = None;
        self.debloat.keep_new_accounts_clean = true;
        self.debloat.job = Some(Job::spawn(ctx, move |pr| {
            pr.set_phase("Reading installed programs".to_string());
            let programs = if programs.is_empty() {
                installscope_core::programs::list_programs(reg.as_ref(), hidden)
            } else {
                programs
            };
            pr.set_phase("Reading Store apps".to_string());
            let store = installscope_core::programs::list_store_apps();
            let provisioned = debloat::provisioned_packages();
            debloat::scan(&programs, &store, &provisioned)
        }));
    }

    pub fn poll_debloat(&mut self) {
        if let Some(items) = self.debloat.job.as_ref().and_then(|j| j.poll()) {
            self.debloat.job = None;
            self.debloat.checked = items
                .iter()
                .filter(|i| i.class == Class::Recommended)
                .map(|i| i.program.key.clone())
                .collect();
            self.debloat.items = Some(items);
        }
        if let Some(errors) = self.debloat.deprovision_job.as_ref().and_then(|j| j.poll()) {
            self.debloat.deprovision_job = None;
            if errors.is_empty() {
                self.set_status(t("New user accounts won't get these apps."));
            } else {
                self.set_status(errors.join(" · "));
            }
        }
    }

    fn remove_bloat(&mut self, ctx: &egui::Context) {
        let Some(items) = self.debloat.items.clone() else {
            return;
        };
        let chosen: Vec<BloatItem> = items
            .into_iter()
            .filter(|i| self.debloat.checked.contains(&i.program.key))
            .collect();
        if chosen.is_empty() {
            return;
        }
        if self.debloat.keep_new_accounts_clean && self.elevated {
            let names: Vec<String> = chosen
                .iter()
                .filter(|i| i.provisioned)
                .map(|i| i.program.name.clone())
                .collect();
            if !names.is_empty() {
                self.debloat.deprovision_job = Some(Job::spawn(ctx, move |_| {
                    names
                        .iter()
                        .filter_map(|n| debloat::deprovision(n).err().map(|e| format!("{n}: {e}")))
                        .collect()
                }));
            }
        }
        let programs = chosen.into_iter().map(|i| i.program).collect();
        self.start_uninstall(programs, false);
        self.debloat.items = None;
    }

    pub fn ui_debloat(&mut self, ctx: &egui::Context) {
        if self.debloat.items.is_none() && self.debloat.job.is_none() {
            self.scan_bloat(ctx);
        }
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            if let Some(j) = &self.debloat.job {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| job_progress(ui, &t(&j.progress.phase()), 0));
                return;
            }
            let items = self.debloat.items.clone().unwrap_or_default();
            if items.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("✔").size(36.0).color(theme::OK));
                    ui.heading(t("No bloatware found"));
                    muted(ui, t("Nothing from the list of known trials, promotions and pre-installed apps is on this PC."));
                    ui.add_space(8.0);
                    if ui.button(format!("⟳ {}", t("Scan again"))).clicked() {
                        self.scan_bloat(ctx);
                    }
                });
                return;
            }
            let chosen = items.iter().filter(|i| self.debloat.checked.contains(&i.program.key)).count();
            theme::card(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(tf("{n} pre-installed apps and trials found", &[("n", items.len().to_string())])).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(format!("⟳ {}", t("Scan again"))).clicked() {
                            self.scan_bloat(ctx);
                        }
                        let enabled = chosen > 0 && self.wizard.is_none();
                        if ui
                            .add_enabled(enabled, egui::Button::new(RichText::new(tf("🛡 Remove {n} selected", &[("n", chosen.to_string())])).strong()).fill(theme::ACCENT))
                            .on_hover_text(t("Uninstalls them one after another, then removes what they left behind. You review every step, and everything is backed up."))
                            .clicked()
                        {
                            self.remove_bloat(ctx);
                        }
                    });
                });
                small_muted(ui, t("Recommended items are ticked: trials, ads, sponsored games and promotions. Optional ones are maker tools some people use, such as support assistants; tick them only if you don't need them. Drivers, audio and hotkey tools and Windows' own parts are never listed."));
                if items.iter().any(|i| i.provisioned) {
                    ui.add_enabled_ui(self.elevated, |ui| {
                        ui.checkbox(&mut self.debloat.keep_new_accounts_clean, t("Also stop Windows installing the removed Store apps for new user accounts"));
                    });
                }
            });
            ui.add_space(8.0);
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                for class in [Class::Recommended, Class::Optional] {
                    let group: Vec<&BloatItem> = items.iter().filter(|i| i.class == class).collect();
                    if group.is_empty() {
                        continue;
                    }
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(t(class.label())).heading());
                        let all = group.iter().all(|i| self.debloat.checked.contains(&i.program.key));
                        if ui.small_button(if all { t("Untick all") } else { t("Tick all") }).clicked() {
                            for i in &group {
                                if all {
                                    self.debloat.checked.remove(&i.program.key);
                                } else {
                                    self.debloat.checked.insert(i.program.key.clone());
                                }
                            }
                        }
                    });
                    ui.add_space(4.0);
                    for i in group {
                        let key = i.program.key.clone();
                        let mut on = self.debloat.checked.contains(&key);
                        egui::Frame::new()
                            .fill(theme::CARD)
                            .corner_radius(10.0)
                            .inner_margin(egui::Margin::symmetric(12, 8))
                            .show(ui, |ui| {
                                ui.set_min_width(ui.available_width());
                                ui.horizontal(|ui| {
                                    if ui.checkbox(&mut on, "").changed() {
                                        if on {
                                            self.debloat.checked.insert(key.clone());
                                        } else {
                                            self.debloat.checked.remove(&key);
                                        }
                                    }
                                    ui.vertical(|ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(RichText::new(i.title()).strong());
                                            if i.program.source == installscope_core::programs::Source::Store {
                                                theme::badge(ui, t("Store app"), theme::MUTED);
                                            }
                                            if i.provisioned {
                                                theme::badge(ui, t("Also for new accounts"), theme::WARN);
                                            }
                                        });
                                        small_muted(ui, format!("{} · {}", t(&i.why), i.program.publisher));
                                    });
                                });
                            });
                        ui.add_space(4.0);
                    }
                    ui.add_space(8.0);
                }
            });
        });
    }
}
