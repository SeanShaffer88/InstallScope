//! Compact terminal-inspired workflow launcher.
use super::{
    app::{App, NAVIGATION},
    theme,
};
impl App {
    pub fn ui_dashboard(&mut self, ctx: &egui::Context) {
        let mut open = None;
        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ctx, |ui| {
                super::tool_window::scroll_body(ui, "workflow_board", |ui| {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            theme::ACCENT_TEXT,
                            egui::RichText::new(setupscope_core::i18n::t(">"))
                                .monospace()
                                .size(24.),
                        );
                        ui.heading(setupscope_core::i18n::t("Workflow Board"));
                    });
                    ui.label(
                        egui::RichText::new(setupscope_core::i18n::t(
                            "Windows Desktop Development / Local Workspace",
                        ))
                        .small()
                        .color(theme::MUTED),
                    );
                    ui.add_space(10.);
                    for row in 0..2 {
                        ui.columns(3, |columns| {
                            for (column, ui) in columns.iter_mut().enumerate() {
                                let index = row * 3 + column;
                                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                                egui::Frame::new()
                                    .fill(theme::CARD)
                                    .corner_radius(4)
                                    .inner_margin(14.)
                                    .stroke(egui::Stroke::new(1.0_f32, theme::LINE))
                                    .show(ui, |ui| {
                                        ui.set_width(ui.available_width());
                                        ui.set_min_height(180.);
                                        if let Some(&(tab, _, title, description)) =
                                            NAVIGATION.get(index)
                                        {
                                            ui.colored_label(
                                                theme::ACCENT_TEXT,
                                                format!("{:02}", index + 1),
                                            );
                                            if ui
                                                .add(
                                                    egui::Button::new(
                                                        egui::RichText::new(
                                                            setupscope_core::i18n::t(title),
                                                        )
                                                        .monospace()
                                                        .size(16.)
                                                        .strong(),
                                                    )
                                                    .wrap()
                                                    .frame(false),
                                                )
                                                .on_hover_text(setupscope_core::i18n::t(description))
                                                .clicked()
                                            {
                                                open = Some(tab);
                                            }
                                            if self.tool_windows.contains(&tab) {
                                                ui.label(
                                                    egui::RichText::new(setupscope_core::i18n::t(
                                                        "[ Open / Focus ]",
                                                    ))
                                                    .small()
                                                    .color(theme::ACCENT_TEXT),
                                                );
                                            }
                                            ui.add_space(6.);
                                            for feature in tab.features() {
                                                ui.horizontal_top(|ui| {
                                                    ui.colored_label(theme::ACCENT_TEXT, "·");
                                                    ui.label(
                                                        egui::RichText::new(
                                                            setupscope_core::i18n::t(feature),
                                                        )
                                                        .size(12.)
                                                        .color(theme::MUTED),
                                                    );
                                                });
                                            }
                                        } else {
                                            ui.colored_label(theme::ACCENT_TEXT, "06");
                                            ui.label(
                                                egui::RichText::new(setupscope_core::i18n::t(
                                                    "System And OS",
                                                ))
                                                .monospace()
                                                .size(16.)
                                                .strong(),
                                            );
                                            ui.add_space(6.);
                                            for detail in self.system_info.localized_parts()
                                            {
                                                ui.label(
                                                    egui::RichText::new(detail)
                                                        .size(12.)
                                                        .color(theme::MUTED),
                                                );
                                            }
                                            ui.label(
                                                egui::RichText::new(setupscope_core::i18n::t(&format!(
                                                    "SetupScope {}.0",
                                                    env!("CARGO_PKG_VERSION")
                                                )))
                                                .small(),
                                            );
                                            ui.small(setupscope_core::i18n::t(
                                                "Read-only local snapshot",
                                            ));
                                        }
                                    });
                            }
                        });
                        ui.add_space(8.);
                    }
                    ui.add_space(2.);
                    ui.label(
                        egui::RichText::new(setupscope_core::i18n::t(
                            "// One Window Per Tool · Integrated Export And Recovery",
                        ))
                        .small()
                        .color(theme::MUTED),
                    );
                });
            });
        if let Some(tab) = open {
            self.open_tool(ctx, tab);
        }
    }
}
