//! Reviewed controls for a few documented Windows privacy preferences.

use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use egui::RichText;
use setupscope_core::i18n::t;
use setupscope_core::privacy::{self, Control, State};

impl App {
    pub fn ui_privacy(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ctx, |ui| {
                super::a11y::focus_hint(ui, theme::MUTED);
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Reviewed Windows privacy controls")).strong());
                    ui.add_space(3.0);
                    muted(ui, t("SetupScope reads the current registry value. Apply or clear one setting at a time; its previous value is saved in Backups before changing it."));
                    ui.add_space(6.0);
                    theme::warning_banner(ui, t("The badge shows the registry value, not proof that Windows enforced it. Windows version, edition, and organization policy can affect behavior. These settings do not turn off required diagnostic data, block network addresses, or disable security and update services."));
                });
                ui.add_space(8.0);

                for definition in privacy::CONTROLS {
                    let state = definition.state(self.reg.as_ref());
                    let recall_supported = definition.control != Control::RecallSnapshots
                        || privacy::recall_policy_supported(self.reg.as_ref());
                    let mut change = None;
                    theme::card(ui, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(t(definition.title)).strong());
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let (label, color) = match (definition.control, state) {
                                        (Control::RecallSnapshots, State::Limited) => {
                                            ("Snapshot saving blocked by policy", theme::OK)
                                        }
                                        (Control::RecallSnapshots, State::Allowed) => {
                                            ("Policy permits snapshot saving", theme::WARN)
                                        }
                                        (Control::RecallSnapshots, State::Unconfigured) => {
                                            ("No policy override", theme::MUTED)
                                        }
                                        (_, State::Limited) => {
                                            ("Value limits collection", theme::OK)
                                        }
                                        (_, State::Allowed) => {
                                            ("Value allows collection", theme::WARN)
                                        }
                                        (_, State::Mixed) => ("Mixed values", theme::WARN),
                                        (_, State::Unconfigured) => ("No value set", theme::MUTED),
                                        (_, State::Unrecognized) => {
                                            ("Unrecognized value", theme::BAD)
                                        }
                                    };
                                    theme::badge(ui, t(label), color);
                                },
                            );
                        });
                        ui.add_space(4.0);
                        muted(ui, t(definition.effect));
                        small_muted(ui, t(definition.tradeoff));
                        if let Some(warning) = definition.warning {
                            ui.add_space(4.0);
                            theme::warning_banner(ui, t(warning));
                        }
                        ui.add_space(6.0);
                        ui.horizontal_wrapped(|ui| {
                            if definition.requires_admin && !self.elevated {
                                theme::badge(ui, t("Administrator required"), theme::WARN);
                                small_muted(
                                    ui,
                                    t("Relaunch SetupScope as Administrator to change this PC-wide setting."),
                                );
                            } else {
                                if !recall_supported {
                                    theme::badge(ui, t("Unsupported Windows version or edition"), theme::WARN);
                                    small_muted(
                                        ui,
                                        t("Limiting Recall snapshots requires Windows 11 build 26100.3915 or later and a supported Pro, Enterprise, Education, or IoT Enterprise edition. You can still clear an existing override."),
                                    );
                                }
                                let apply = state != State::Limited && recall_supported;
                                let label = if apply {
                                    t("Limit this setting")
                                } else {
                                    t("Already limited")
                                };
                                if super::a11y::explain(
                                    ui.add_enabled(
                                        apply && recall_supported,
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
                            let warning = definition.warning.map(|warning| format!("\n\n{}", t(warning))).unwrap_or_default();
                            format!(
                                "{}\n\n{}\n\nSetupScope will back up the current registry value and write the documented setting. You can restore that backup from the Backups page.{}",
                                t(definition.effect),
                                t(definition.tradeoff),
                                warning,
                            )
                        } else {
                            let warning = definition.warning.map(|warning| format!("\n\n{}", t(warning))).unwrap_or_default();
                            format!(
                                "Clear only {}\\{}? The exact current value or values will be backed up first. Windows or an organization policy may set the effective behavior afterward.",
                                definition.key,
                                definition.value_names.join(", ")
                            ) + &warning
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
        if limit
            && control == Control::RecallSnapshots
            && !privacy::recall_policy_supported(self.reg.as_ref())
        {
            self.set_status(t("Recall snapshot policy is unavailable on this Windows version or edition; nothing changed."));
            return;
        }
        if definition.requires_admin && !self.elevated {
            self.set_status(t("This setting applies to the whole PC. Relaunch SetupScope as Administrator to change it."));
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
