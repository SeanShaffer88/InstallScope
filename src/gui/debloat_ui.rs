//! Bloatware Shield page: one click to remove the trials, promotions and
//! pre-installed apps that come with new PCs, including Store apps and the
//! copies Windows keeps for new user accounts.

use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use setupsweep_core::backup::ActionResult;
use setupsweep_core::debloat::{self, BloatItem, Class, DeprovisionReport};
use setupsweep_core::i18n::{t, tf};
use std::collections::HashSet;

#[derive(Default)]
pub struct DebloatState {
    pub items: Option<Vec<BloatItem>>,
    pub job: Option<Job<Vec<BloatItem>>>,
    pub deprovision_job: Option<Job<Vec<ActionResult>>>,
    pub checked: HashSet<String>,
    pub keep_new_accounts_clean: bool,
    pub last_report: Option<DeprovisionReport>,
}

impl App {
    pub fn scan_bloat(&mut self, ctx: &egui::Context) {
        let programs = self.programs.list.clone();
        let reg = self.reg.clone();
        let hidden = self.settings.system_components_shown();
        self.debloat.items = None;
        self.debloat.keep_new_accounts_clean = true;
        self.debloat.last_report = self.store.load_deprovision_report();
        self.debloat.job = Some(Job::spawn(ctx, move |pr| {
            pr.set_phase("Reading installed programs".to_string());
            let programs = if programs.is_empty() {
                setupsweep_core::programs::list_programs(reg.as_ref(), hidden)
            } else {
                programs
            };
            pr.set_phase("Reading Store apps".to_string());
            let store = setupsweep_core::programs::list_store_apps();
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
        if let Some(rows) = self.debloat.deprovision_job.as_ref().and_then(|j| j.poll()) {
            self.debloat.deprovision_job = None;
            let failed = rows.iter().filter(|r| !r.ok).count();
            let ok = rows.len() - failed;
            let report = DeprovisionReport {
                version: 1,
                ran_at: setupsweep_core::util::now_string(),
                rows,
            };
            if let Err(error) = self.store.save_deprovision_report(&report) {
                self.set_status(tf(
                    "Deprovisioning finished but its record could not be saved: {error}",
                    &[("error", format!("{error:#}"))],
                ));
            } else {
                self.set_status(if failed == 0 {
                    tf(
                        "Deprovisioned {n} apps: new user accounts won't get them.",
                        &[("n", ok.to_string())],
                    )
                } else {
                    tf(
                        "{ok} apps deprovisioned, {bad} failed — details on this page.",
                        &[("ok", ok.to_string()), ("bad", failed.to_string())],
                    )
                });
            }
            self.debloat.last_report = Some(report);
        }
    }

    fn remove_bloat(&mut self) {
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
                // Deprovisioning is machine-wide and irreversible, so it
                // always gets its own confirmation (see Confirm::DeprovisionBloat).
                // The names ride with the wizard and are deprovisioned only
                // after those programs were actually removed.
                let programs: Vec<setupsweep_core::programs::InstalledProgram> =
                    chosen.into_iter().map(|i| i.program).collect();
                self.ask(
                    "Deprovision apps for new user accounts?",
                    tf(
                        "For each removed app, SetupSweep also removes the copy Windows keeps for new user accounts, so the apps don't come back. This affects the whole PC, and SetupSweep cannot undo it. {n} of the selected apps have such a copy; it is deprovisioned only after the app itself was removed.",
                        &[("n", names.len().to_string())],
                    ),
                    "Deprovision and start removal",
                    Confirm::DeprovisionBloat(names, programs),
                );
                return;
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
            // The focused control's explanation line, for keyboard
            // and touch users (see a11y::explain).
            super::a11y::focus_hint(ui, theme::MUTED);
            if let Some(j) = &self.debloat.job {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| job_progress(ui, &t(&j.progress.phase()), 0));
                return;
            }
            if let Some(report) = self.debloat.last_report.clone() {
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Last deprovisioning")).strong());
                    small_muted(ui, tf("Finished {at}.", &[("at", report.ran_at)]));
                    ui.add_space(4.0);
                    results_ui(ui, &report.rows);
                });
                ui.add_space(8.0);
            }
            let items = self.debloat.items.clone().unwrap_or_default();
            if items.is_empty() {
                ui.add_space(40.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("✔").size(36.0).color(theme::OK));
                    ui.heading(t("No bloatware found"));
                    muted(ui, t("Nothing from the list of known trials, promotions and pre-installed apps is on this PC."));
                    ui.add_space(8.0);
                    if super::a11y::explain(
                        ui.button(format!("⟳ {}", t("Scan again"))),
                        &t("Look again for known trials, promotions and pre-installed apps on this PC."),
                    )
                    .clicked()
                    {
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
                        if super::a11y::explain(
                            ui.button(format!("⟳ {}", t("Scan again"))),
                            &t("Look again for known trials, promotions and pre-installed apps on this PC."),
                        )
                        .clicked()
                        {
                            self.scan_bloat(ctx);
                        }
                        let enabled = chosen > 0 && self.wizard.is_none();
                        let rm_why = if self.wizard.is_some() {
                            t("Unavailable while a removal is being reviewed. Finish or cancel that review first.")
                        } else if chosen == 0 {
                            t("Tick the apps to remove first. Removing uninstalls them one after another, then removes what they left behind; you review every step, and everything is backed up first, unless you have turned backups off in Settings.")
                        } else {
                            t("Uninstalls them one after another, then removes what they left behind. You review every step, and everything is backed up first, unless you have turned backups off in Settings.")
                        };
                        if super::a11y::explain(
                            ui.add_enabled(enabled, egui::Button::new(RichText::new(tf("🛡 Remove {n} selected", &[("n", chosen.to_string())])).strong()).fill(theme::ACCENT)),
                            &rm_why,
                        )
                        .clicked()
                        {
                            self.remove_bloat();
                        }
                    });
                });
                small_muted(ui, t("Recommended items are ticked: trials, ads, sponsored games and promotions. Optional ones are maker tools some people use, such as support assistants; tick them only if you don't need them. Drivers, audio and hotkey tools and Windows' own parts are never listed."));
                if items.iter().any(|i| i.provisioned) {
                    let kp_why = if self.elevated {
                        t("Also remove the copy Windows keeps for new user accounts, so the removed apps don't come back for new accounts. This affects the whole PC and cannot be undone; it happens only after the app itself was removed, and asks for its own confirmation first.")
                    } else {
                        t("Unavailable: this needs administrator rights. Run SetupSweep as administrator to use it.")
                    };
                    ui.add_enabled_ui(self.elevated, |ui| {
                        super::a11y::explain(
                            ui.checkbox(&mut self.debloat.keep_new_accounts_clean, t("Also stop Windows installing the removed Store apps for new user accounts")),
                            &kp_why,
                        );
                    });
                    small_muted(ui, t("Removes the copy Windows keeps for new user accounts, so the app doesn't come back. It affects the whole PC, and SetupSweep cannot undo it."));
                    if !self.elevated {
                        small_muted(ui, t("Needs administrator rights to change what new user accounts get."));
                    }
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
                        let tick_why = if all {
                            tf("Untick every app in the {group} group, so none of them is removed.", &[("group", t(class.label()))])
                        } else {
                            tf("Tick every app in the {group} group, to include them all in the removal.", &[("group", t(class.label()))])
                        };
                        if super::a11y::explain(ui.small_button(if all { t("Untick all") } else { t("Tick all") }), &tick_why).clicked() {
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
                                    let box_resp = super::a11y::explain(
                                        ui.checkbox(&mut on, ""),
                                        &tf(
                                            "Tick to remove {name} with the other ticked apps. Removal is reviewed step by step and backed up first, unless backups are off in Settings.",
                                            &[("name", i.title())],
                                        ),
                                    );
                                    box_resp.widget_info(|| {
                                        egui::WidgetInfo::selected(
                                            egui::WidgetType::Checkbox,
                                            true,
                                            on,
                                            &i.title(),
                                        )
                                    });
                                    if box_resp.changed() {
                                        if on {
                                            self.debloat.checked.insert(key.clone());
                                        } else {
                                            self.debloat.checked.remove(&key);
                                        }
                                    }
                                    ui.vertical(|ui| {
                                        ui.horizontal_wrapped(|ui| {
                                            ui.label(RichText::new(i.title()).strong());
                                            if i.program.source == setupsweep_core::programs::Source::Store {
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
