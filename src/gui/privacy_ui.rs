//! Reviewed controls for a few documented Windows privacy preferences.

use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use egui::RichText;
use installscope_core::i18n::t;
use installscope_core::privacy::{self, Control, State};

impl App {
    pub fn ui_privacy(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ctx, |ui| {
                super::a11y::focus_hint(ui, theme::MUTED);
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Reviewed Windows privacy controls")).strong());
                    ui.add_space(3.0);
                    muted(ui, t("InstallScope reads the current registry value. Apply or clear one setting at a time; its previous value is saved in Backups before changing it."));
                    ui.add_space(6.0);
                    theme::warning_banner(ui, t("The badge shows the registry value, not proof that Windows enforced it. Windows version, edition, and organization policy can affect behavior. These settings do not turn off required diagnostic data, block network addresses, or disable security and update services."));
                });
                ui.add_space(8.0);

                for definition in privacy::CONTROLS {
                    let state = definition.state(self.reg.as_ref());
                    let mut change = None;
                    theme::card(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(t(definition.title)).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| match state {
                                    State::Limited => {
                                        theme::badge(ui, t("Value limits collection"), theme::OK)
                                    }
                                    State::Allowed => {
                                        theme::badge(ui, t("Value allows collection"), theme::WARN)
                                    }
                                    State::Mixed => {
                                        theme::badge(ui, t("Mixed values"), theme::WARN)
                                    }
                                    State::Unconfigured => {
                                        theme::badge(ui, t("No value set"), theme::MUTED)
                                    }
                                    State::Unrecognized => {
                                        theme::badge(ui, t("Unrecognized value"), theme::BAD)
                                    }
                                },
                            );
                        });
                        ui.add_space(4.0);
                        muted(ui, t(definition.effect));
                        small_muted(ui, t(definition.tradeoff));
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            if definition.requires_admin && !self.elevated {
                                theme::badge(ui, t("Administrator required"), theme::WARN);
                                small_muted(
                                    ui,
                                    t("Relaunch InstallScope as Administrator to change this PC-wide setting."),
                                );
                            } else {
                                let apply = state != State::Limited;
                                let label = if apply {
                                    t("Limit this setting")
                                } else {
                                    t("Already limited")
                                };
                                if super::a11y::explain(
                                    ui.add_enabled(
                                        apply,
                                        egui::Button::new(label).fill(theme::ACCENT),
                                    ),
                                    &t("Backs up the exact previous value, then writes the documented Windows setting."),
                                )
                                .clicked()
                                {
                                    change = Some(true);
                                }
                                let clear_enabled = state != State::Unconfigured;
                                if super::a11y::explain(
                                    ui.add_enabled(clear_enabled, egui::Button::new(t("Clear value"))),
                                    &t("Backs up the exact current value, then removes only this value so Windows can use its default or another setting."),
                                )
                                .clicked()
                                {
                                    change = Some(false);
                                }
                            }
                        });
                        ui.add_space(4.0);
                        small_muted(
                            ui,
                            format!(
                                "{}\\{} · {}",
                                definition.key,
                                definition.value_names.join(", "),
                                if definition.requires_admin {
                                    t("applies to this PC")
                                } else {
                                    t("applies to this Windows account")
                                }
                            ),
                        );
                    });
                    ui.add_space(8.0);
                    if let Some(limit) = change {
                        let action = Confirm::PrivacyChange(definition.control, limit);
                        let body = if limit {
                            format!(
                                "{}\n\n{}\n\nInstallScope will back up the current registry value and write the documented setting. You can restore that backup from the Backups page.",
                                t(definition.effect),
                                t(definition.tradeoff)
                            )
                        } else {
                            format!(
                                "Clear only {}\\{}? The exact current value or values will be backed up first. Windows or an organization policy may set the effective behavior afterward.",
                                definition.key,
                                definition.value_names.join(", ")
                            )
                        };
                        self.ask(
                            if limit { "Limit this privacy setting?" } else { "Clear this privacy setting value?" },
                            body,
                            if limit { "Back up and apply" } else { "Back up and clear" },
                            action,
                        );
                    }
                }
            });
    }

    pub fn apply_privacy_change(&mut self, control: Control, limit: bool) {
        let Some(definition) = privacy::CONTROLS
            .iter()
            .find(|definition| definition.control == control)
        else {
            self.set_status(t("Unknown privacy setting; nothing changed."));
            return;
        };
        if definition.requires_admin && !self.elevated {
            self.set_status(t("This setting applies to the whole PC. Relaunch InstallScope as Administrator to change it."));
            return;
        }
        match privacy::apply_with_backup(
            self.reg.as_ref(),
            &self.store.backups_dir(),
            control,
            limit,
        ) {
            Ok(true) => {
                self.set_status(format!(
                    "{} {}. Previous value saved in Backups.",
                    t(definition.title),
                    if limit {
                        t("limited")
                    } else {
                        t("override cleared")
                    }
                ));
                self.reload_backups();
            }
            Ok(false) => self.set_status(t("The requested privacy state was already in effect; nothing changed.")),
            Err(error) => {
                self.set_status(format!("Privacy change failed or may be incomplete. Check Backups before retrying: {error}"))
            }
        }
    }
}
