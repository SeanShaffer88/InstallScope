//! Dashboard: status at a glance and quick actions.

use super::app::{App, Tab};
use super::theme;
use super::widgets::*;
use egui::RichText;
use installscope_core::i18n::{t, tf};
use installscope_core::model::human_size;

/// One Command Deck metric tile: a big tabular numeral, a label, and — only
/// when the tile needs attention — a 3px red top rule and a coloured numeral.
///
/// The whole card is one button. It is registered with `Ui::interact` under
/// a stable id (not just sensed on the frame's own response), which is what
/// puts it in the keyboard focus order: Tab reaches it, the app-wide focus
/// ring outlines it, and Enter/Space — or a screen reader's activate
/// command — clicks it, exactly like a mouse click.
fn stat_tile(
    ui: &mut egui::Ui,
    key: &str,
    value: String,
    label: String,
    numeral_color: egui::Color32,
    attention: bool,
) -> egui::Response {
    let frame = egui::Frame::new()
        .fill(theme::CARD)
        .stroke(egui::Stroke::new(1.0_f32, theme::LINE))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(16, 14))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            if attention {
                let (_, rule) = ui.allocate_space(egui::vec2(ui.available_width(), 3.0));
                ui.painter().rect_filled(rule, 0.0, theme::ACCENT);
                ui.add_space(8.0);
            }
            ui.label(
                RichText::new(&value)
                    .monospace()
                    .size(32.0)
                    .strong()
                    .color(numeral_color),
            );
            ui.label(RichText::new(&label).size(13.0).color(theme::MUTED));
        });
    let resp = ui
        .interact(
            frame.response.rect,
            ui.id().with("stat_tile").with(key),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    super::a11y::button(&resp, &format!("{value} {label}"));
    resp
}

/// One quick-action row: icon, title, description, chevron. Like `stat_tile`,
/// registered as a real button under a stable id so keyboard and screen
/// reader users can reach and activate it, not just the mouse.
fn action_row(ui: &mut egui::Ui, icon: &str, title: &str, desc: &str) -> bool {
    let frame = egui::Frame::new()
        .fill(theme::CARD)
        .stroke(egui::Stroke::new(1.0_f32, theme::LINE))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(16, 12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(icon).size(20.0).color(theme::MUTED));
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(t(title)).strong());
                    small_muted(ui, t(desc));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new("›").size(20.0).color(theme::FAINT));
                });
            });
        });
    let resp = ui
        .interact(
            frame.response.rect,
            ui.id().with("action_row").with(title),
            egui::Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    super::a11y::button(&resp, &format!("{}. {}", t(title), t(desc)));
    resp.clicked()
}

impl App {
    pub fn ui_dashboard(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                if !self.elevated {
                    theme::warning_banner(ui, t("InstallScope is not running as Administrator. Some removals will fail. Use \"Run as Administrator\" in the left column."));
                    ui.add_space(10.0);
                }
                self.interrupted_ui(ui);
                if self.monitor.is_active() {
                    theme::deck_panel(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("⏺").size(22.0).color(theme::ACCENT_TEXT));
                            ui.vertical(|ui| {
                                ui.label(RichText::new(tf("Monitoring: {name}", &[("name", self.monitor.name.clone())])).strong());
                                small_muted(ui, t("Finish monitoring when the installation is complete."));
                            });
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if theme::primary_button(ui, t("Go to Monitor")).clicked() {
                                    self.tab = Tab::Monitor;
                                }
                            });
                        });
                    });
                    ui.add_space(10.0);
                }

                let programs = self.programs.list.len();
                let monitored = self.monitored_keys().len();
                let broken = self.programs.broken.len();
                let backups = self.backups.list.len();
                let backup_bytes: u64 = self.backups.list.iter().map(|b| b.bytes()).sum();

                // Slim status strip: the same facts the tiles below show,
                // summarised in one line.
                let (dot, status) = if broken > 0 {
                    (
                        theme::WARN,
                        tf("{n} broken entries", &[("n", broken.to_string())]),
                    )
                } else {
                    (theme::OK, t("All systems healthy"))
                };
                egui::Frame::new()
                    .fill(theme::CARD)
                    .stroke(egui::Stroke::new(1.0_f32, theme::LINE))
                    .corner_radius(6)
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("●").size(13.0).color(dot));
                            ui.label(
                                RichText::new(format!(
                                    "{} · {}",
                                    status,
                                    tf("{n} backups ready", &[("n", backups.to_string())])
                                ))
                                .size(13.0)
                                .color(theme::MUTED),
                            );
                        });
                    });
                ui.add_space(12.0);

                let mut go: Option<(Tab, Option<super::programs::View>)> = None;
                ui.columns(4, |cols| {
                    if stat_tile(&mut cols[0], "programs", programs.to_string(), t("Programs installed"), theme::TEXT, false).clicked() {
                        go = Some((Tab::Programs, Some(super::programs::View::All)));
                    }
                    if stat_tile(&mut cols[1], "monitored", monitored.to_string(), t("Monitored"), theme::TEXT, false).clicked() {
                        go = Some((Tab::Programs, Some(super::programs::View::Monitored)));
                    }
                    let nc = if broken > 0 { theme::WARN } else { theme::TEXT };
                    if stat_tile(&mut cols[2], "broken", broken.to_string(), t("Broken entries"), nc, broken > 0).clicked() {
                        go = Some((Tab::Programs, Some(super::programs::View::Broken)));
                    }
                    if stat_tile(&mut cols[3], "backups", backups.to_string(), tf("Backups ({size})", &[("size", human_size(backup_bytes))]), theme::TEXT, false).clicked() {
                        go = Some((Tab::Backups, None));
                    }
                });
                if let Some((tab, view)) = go {
                    self.tab = tab;
                    if let Some(v) = view {
                        self.programs.view = v;
                    }
                }

                ui.add_space(16.0);
                theme::section_label(ui, t("Quick actions"));
                ui.add_space(6.0);
                if action_row(ui, "📡", "Install a program with monitoring", "Record everything it changes so it can be removed precisely later") {
                    self.tab = Tab::Monitor;
                    self.start_install_and_monitor(ctx);
                }
                ui.add_space(6.0);
                if action_row(ui, "🗑", "Uninstall programs", "Remove programs and everything they leave behind") {
                    self.tab = Tab::Programs;
                }
                ui.add_space(6.0);
                if action_row(ui, "🚀", "Speed up startup", "Choose which programs, services and tasks start with Windows") {
                    self.tab = Tab::Startup;
                    self.on_tab_opened(ctx, Tab::Startup);
                }
                ui.add_space(6.0);
                if action_row(ui, "✨", "Clean junk files", "Temporary files, caches and crash dumps") {
                    self.tab = Tab::Cleaner;
                }
                if self.settings.advanced_mode {
                    ui.add_space(6.0);
                    if self.orphan_job.is_some() {
                        job_progress(ui, &t("Looking for leftovers of removed programs…"), 0);
                    } else if action_row(
                        ui,
                        "🔍",
                        "Find leftovers of removed programs",
                        "Includes clearly stale uninstall entries and other registrations pointing to missing files",
                    ) {
                        self.scan_orphans();
                    }
                }
                ui.add_space(6.0);
                if action_row(ui, "⏪", "System Restore", "Create restore points or roll Windows back to an earlier state") {
                    self.tab = Tab::Restore;
                }
                if action_row(ui, "🎒", "Make a portable app", "Turn a program's installer into a PortableApps.com package") {
                    self.tab = Tab::Builder;
                }
                if action_row(ui, "🛡", "Remove bloatware", "Trials, promotions and pre-installed apps that came with this PC") {
                    self.tab = Tab::Debloat;
                }

                // "What are the top 5 space-wasters I haven't opened in 6 months?"
                if !self.programs.usage.is_empty() {
                    let mut wasters: Vec<(installscope_core::programs::InstalledProgram, u64, i64)> = self
                        .programs
                        .list
                        .iter()
                        .filter_map(|p| {
                            let days = self.unused_days(p)?;
                            let size = if p.size_kb > 0 { p.size_kb * 1024 } else { self.programs.sizes.get(&p.key).copied().unwrap_or(0) };
                            (days >= 180 && size > 0).then(|| (p.clone(), size, days))
                        })
                        .collect();
                    wasters.sort_by_key(|w| std::cmp::Reverse(w.1));
                    wasters.truncate(5);
                    if !wasters.is_empty() {
                        ui.add_space(16.0);
                        theme::section_label(ui, t("Space-wasters you don't use"));
                        small_muted(ui, t("The biggest programs not opened in the last 6 months."));
                        ui.add_space(6.0);
                        let total: u64 = wasters.iter().map(|w| w.1).sum();
                        theme::deck_panel(ui, |ui| {
                            for (p, size, days) in &wasters {
                                ui.horizontal(|ui| {
                                    if ui.link(RichText::new(&p.name).strong()).clicked() {
                                        self.programs.selected = Some(p.key.clone());
                                        self.programs.filter.clear();
                                        self.tab = Tab::Programs;
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        small_muted(ui, tf("not opened for {n} months", &[("n", (days / 30).to_string())]));
                                        ui.add_space(8.0);
                                        ui.label(
                                            RichText::new(installscope_core::model::human_size(*size))
                                                .monospace(),
                                        );
                                    });
                                });
                                ui.separator();
                            }
                            ui.horizontal(|ui| {
                                small_muted(ui, tf("{size} in total", &[("size", installscope_core::model::human_size(total))]));
                                if ui.button(t("Review and uninstall…")).clicked() {
                                    let chosen: Vec<_> = wasters.iter().map(|w| w.0.clone()).collect();
                                    self.start_uninstall(chosen, false);
                                }
                            });
                        });
                    }
                }

                if !self.logs.logs.is_empty() {
                    ui.add_space(16.0);
                    theme::section_label(ui, t("Recently monitored"));
                    ui.add_space(6.0);
                    let recent: Vec<(String, String, String)> =
                        self.logs.logs.iter().take(5).map(|l| (l.id.clone(), l.name.clone(), l.created.clone())).collect();
                    theme::deck_panel(ui, |ui| {
                        let last = recent.len().saturating_sub(1);
                        for (i, (id, name, created)) in recent.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui.link(RichText::new(name).strong()).clicked() {
                                    self.logs.selected = Some(id.clone());
                                    self.tab = Tab::Logs;
                                }
                                small_muted(ui, created.clone());
                            });
                            if i < last {
                                ui.separator();
                            }
                        }
                    });
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Id, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

    /// What one rendered pass produced: the tile's id and rect, and whether
    /// the tile and the action row reported a click in that pass.
    #[derive(Default)]
    struct Pass {
        tile_id: Option<Id>,
        tile_rect: Option<Rect>,
        tile_clicked: bool,
        row_clicked: bool,
    }

    fn input(events: Vec<Event>) -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            events,
            ..Default::default()
        }
    }

    fn key_press(key: Key) -> Vec<Event> {
        vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]
    }

    /// Render one frame holding a stat tile above an action row, the same
    /// helpers the Dashboard itself uses.
    fn render(ctx: &Context, events: Vec<Event>, value: &str) -> Pass {
        let mut pass = Pass::default();
        let _ = ctx.run(input(events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let tile = stat_tile(
                    ui,
                    "programs",
                    value.to_string(),
                    "Programs installed".to_string(),
                    theme::TEXT,
                    false,
                );
                pass.tile_id = Some(tile.id);
                pass.tile_rect = Some(tile.rect);
                pass.tile_clicked = tile.clicked();
                pass.row_clicked = action_row(
                    ui,
                    "🗑",
                    "Uninstall programs",
                    "Remove programs and everything they leave behind",
                );
            });
        });
        pass
    }

    #[test]
    fn tiles_and_rows_are_keyboard_operable() {
        let ctx = Context::default();
        let first = render(&ctx, Vec::new(), "42");
        let tile = first.tile_id.expect("tile has an id");
        assert_eq!(ctx.memory(|m| m.focused()), None);

        // Tab reaches the tile first (layout order), then the action row.
        render(&ctx, key_press(Key::Tab), "42");
        assert_eq!(ctx.memory(|m| m.focused()), Some(tile));

        // Enter activates the focused tile, not the row below it.
        let pass = render(&ctx, key_press(Key::Enter), "42");
        assert!(pass.tile_clicked);
        assert!(!pass.row_clicked);

        render(&ctx, key_press(Key::Tab), "42");
        let focused = ctx.memory(|m| m.focused());
        assert!(
            focused.is_some() && focused != Some(tile),
            "focus moved to the row"
        );

        // Space activates the focused row, not the tile above it.
        let pass = render(&ctx, key_press(Key::Space), "42");
        assert!(pass.row_clicked);
        assert!(!pass.tile_clicked);
    }

    #[test]
    fn tile_keeps_its_id_and_focus_when_its_numbers_change() {
        // The tile's id comes from its stable key, not from the numbers it
        // shows, so focus survives the counts changing underneath it.
        let ctx = Context::default();
        let first = render(&ctx, Vec::new(), "42");
        let tile = first.tile_id.expect("tile has an id");
        render(&ctx, key_press(Key::Tab), "42");
        assert_eq!(ctx.memory(|m| m.focused()), Some(tile));

        let changed = render(&ctx, Vec::new(), "43");
        assert_eq!(changed.tile_id, Some(tile));
        assert_eq!(ctx.memory(|m| m.focused()), Some(tile));
    }

    #[test]
    fn tiles_still_click_with_the_mouse() {
        let ctx = Context::default();
        let first = render(&ctx, Vec::new(), "42");
        let center = first.tile_rect.expect("tile has a rect").center();
        let button = |pressed: bool| {
            vec![Event::PointerButton {
                pos: center,
                button: PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            }]
        };
        render(&ctx, button(true), "42");
        let pass = render(&ctx, button(false), "42");
        assert!(pass.tile_clicked);
        assert!(!pass.row_clicked);
    }
}
