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
/// reader users can reach and activate it, not just the mouse. `explain` is
/// the row's fuller explanation, attached with `a11y::explain` (hover text,
/// the page's focus-follows line, and the screen-reader description).
fn action_row(ui: &mut egui::Ui, icon: &str, title: &str, desc: &str, explain: &str) -> bool {
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
    let resp = super::a11y::explain(resp, explain);
    resp.clicked()
}

/// One installed program reduced to what the space-wasters panel needs:
/// its resolved size in bytes (0 = unknown) and the evidence for calling
/// it long-unused, if any.
struct WasterCandidate {
    program: installscope_core::programs::InstalledProgram,
    size: u64,
    basis: Option<super::programs::UnusedBasis>,
}

/// Which claim the space-wasters panel is making. The two kinds exist
/// because a recorded last-opened date and a mere install date are
/// different facts, and the heading must only ever claim the one the
/// listed programs actually have.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WasterKind {
    /// Every entry has a recorded last-opened date at least 180 days ago.
    NotOpened,
    /// No entry has a recorded last-opened date; entries are ranked by
    /// size among the programs installed at least 180 days ago.
    InstalledLongAgo,
}

/// What the space-wasters panel shows, chosen so the heading is true of
/// every listed program.
struct WasterPanel {
    kind: WasterKind,
    /// (program, size in bytes, days) — largest first, at most 5.
    entries: Vec<(installscope_core::programs::InstalledProgram, u64, i64)>,
    /// Old, sizeable programs left out of a `NotOpened` panel because
    /// Windows has no record of when they were last opened.
    unknown_opened: usize,
    /// Old programs left out because their size is unknown.
    unknown_size: usize,
}

/// Days without use (or since install) before a program counts as old.
const WASTER_MIN_DAYS: i64 = 180;
/// How many entries the panel lists.
const WASTER_LIMIT: usize = 5;

/// Choose the space-wasters panel's contents from per-program size and
/// usage evidence.
///
/// Ranking is always by known size. Programs with a recorded last-opened
/// date form the panel whenever at least one qualifies; install-age-only
/// programs are never mixed into it (they would make its "not opened"
/// claim false) and are instead counted, or — when they are all there
/// is — listed under an install-age heading of their own. Old programs
/// whose size is unknown cannot be ranked and are counted, not dropped
/// silently.
fn space_wasters(candidates: &[WasterCandidate]) -> Option<WasterPanel> {
    let mut measured: Vec<(installscope_core::programs::InstalledProgram, u64, i64)> = Vec::new();
    let mut installed: Vec<(installscope_core::programs::InstalledProgram, u64, i64)> = Vec::new();
    let mut unknown_size = 0usize;
    for c in candidates {
        let (days, is_measured) = match c.basis {
            Some(super::programs::UnusedBasis::LastOpened(d)) => (d, true),
            Some(super::programs::UnusedBasis::Installed(d)) => (d, false),
            None => continue,
        };
        if days < WASTER_MIN_DAYS {
            continue;
        }
        if c.size == 0 {
            unknown_size += 1;
            continue;
        }
        if is_measured {
            measured.push((c.program.clone(), c.size, days));
        } else {
            installed.push((c.program.clone(), c.size, days));
        }
    }
    let largest_first =
        |a: &(installscope_core::programs::InstalledProgram, u64, i64),
         b: &(installscope_core::programs::InstalledProgram, u64, i64)| b.1.cmp(&a.1);
    if !measured.is_empty() {
        measured.sort_by(largest_first);
        measured.truncate(WASTER_LIMIT);
        Some(WasterPanel {
            kind: WasterKind::NotOpened,
            entries: measured,
            unknown_opened: installed.len(),
            unknown_size,
        })
    } else if !installed.is_empty() {
        installed.sort_by(largest_first);
        installed.truncate(WASTER_LIMIT);
        Some(WasterPanel {
            kind: WasterKind::InstalledLongAgo,
            entries: installed,
            unknown_opened: 0,
            unknown_size,
        })
    } else {
        None
    }
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
                                if super::a11y::explain(theme::primary_button(ui, t("Go to Monitor")), &t("Open the Monitor page for the installation being recorded. Finish monitoring there once the installation is complete.")).clicked() {
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
                // The page's explanation line: keyboard and touch users
                // read here what mouse users get as a tooltip (see
                // a11y::explain).
                super::a11y::focus_hint(ui, theme::MUTED);
                ui.add_space(12.0);

                let mut go: Option<(Tab, Option<super::programs::View>)> = None;
                ui.columns(4, |cols| {
                    if super::a11y::explain(stat_tile(&mut cols[0], "programs", programs.to_string(), t("Programs installed"), theme::TEXT, false), &t("Open the Programs page: browse everything installed on this PC, and uninstall programs from there.")).clicked() {
                        go = Some((Tab::Programs, Some(super::programs::View::All)));
                    }
                    if super::a11y::explain(stat_tile(&mut cols[1], "monitored", monitored.to_string(), t("Monitored"), theme::TEXT, false), &t("Open the Programs page showing only the installs InstallScope monitored. Their changes were recorded, so they can be removed precisely.")).clicked() {
                        go = Some((Tab::Programs, Some(super::programs::View::Monitored)));
                    }
                    let nc = if broken > 0 { theme::WARN } else { theme::TEXT };
                    if super::a11y::explain(stat_tile(&mut cols[2], "broken", broken.to_string(), t("Broken entries"), nc, broken > 0), &t("Open the Programs page showing entries whose program files are missing. They are usually leftovers of programs removed another way, and can be cleaned up from there.")).clicked() {
                        go = Some((Tab::Programs, Some(super::programs::View::Broken)));
                    }
                    if super::a11y::explain(stat_tile(&mut cols[3], "backups", backups.to_string(), tf("Backups ({size})", &[("size", human_size(backup_bytes))]), theme::TEXT, false), &t("Open the Backups page: the backups InstallScope took before removals, ready to restore. This is where you undo a removal.")).clicked() {
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
                if action_row(ui, "📡", "Install a program with monitoring", "Record everything it changes so it can be removed precisely later", &t("Open the Monitor page and run an installer while InstallScope watches. Everything it changes is recorded, so it can be removed precisely later.")) {
                    self.tab = Tab::Monitor;
                    self.start_install_and_monitor(ctx);
                }
                ui.add_space(6.0);
                if action_row(ui, "🗑", "Uninstall programs", "Remove programs and everything they leave behind", &t("Open the Programs page to uninstall programs. You review everything before anything is removed, and removals are backed up unless backups are off in Settings.")) {
                    self.tab = Tab::Programs;
                }
                ui.add_space(6.0);
                if action_row(ui, "🚀", "Speed up startup", "Choose which programs, services and tasks start with Windows", &t("Open the Startup page: choose which programs, services and tasks start with Windows. Anything you turn off there can be turned back on from the same page.")) {
                    self.tab = Tab::Startup;
                    self.on_tab_opened(ctx, Tab::Startup);
                }
                ui.add_space(6.0);
                if action_row(ui, "✨", "Clean junk files", "Temporary files, caches and crash dumps", &t("Open the Junk Cleaner for temporary files, caches and crash dumps. You see exactly which files will be deleted and confirm before anything is removed.")) {
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
                        &t("Scan for what programs that are already gone left behind: stale entries and registrations pointing to missing files. You review everything it finds before anything is removed."),
                    ) {
                        self.scan_orphans();
                    }
                }
                ui.add_space(6.0);
                if action_row(ui, "⏪", "System Restore", "Create restore points or roll Windows back to an earlier state", &t("Open the System Restore page: create a restore point, or roll Windows' system files and settings back to an earlier state. Your documents and photos are not part of a restore point.")) {
                    self.tab = Tab::Restore;
                }
                if action_row(ui, "🎒", "Make a portable app", "Turn a program's installer into a PortableApps.com package", &t("Open the Setup Package Creator: turn a program's installer into a PortableApps.com package that runs without being installed.")) {
                    self.tab = Tab::Builder;
                }
                if action_row(ui, "🛡", "Remove bloatware", "Trials, promotions and pre-installed apps that came with this PC", &t("Open Bloatware Shield: trials, promotions and pre-installed apps that came with this PC. You review and confirm before anything is removed.")) {
                    self.tab = Tab::Debloat;
                }

                // "What are the top 5 space-wasters I haven't opened in 6 months?"
                // Only programs with a recorded last-opened date may be
                // called "not opened"; when the evidence is only an install
                // date, the panel says so instead (see `space_wasters`).
                if !self.programs.usage.is_empty() {
                    let candidates: Vec<WasterCandidate> = self
                        .programs
                        .list
                        .iter()
                        .map(|p| {
                            let size = if p.size_kb > 0 { p.size_kb * 1024 } else { self.programs.sizes.get(&p.key).copied().unwrap_or(0) };
                            WasterCandidate {
                                program: p.clone(),
                                size,
                                basis: self.unused_basis(p),
                            }
                        })
                        .collect();
                    if let Some(panel) = space_wasters(&candidates) {
                        let wasters = &panel.entries;
                        ui.add_space(16.0);
                        let (title, subtitle, row_note) = match panel.kind {
                            WasterKind::NotOpened => (
                                t("Space-wasters you don't use"),
                                t("The biggest programs not opened in the last 6 months."),
                                "not opened for {n} months",
                            ),
                            WasterKind::InstalledLongAgo => (
                                t("Big programs installed long ago"),
                                t("Installed over 6 months ago. Windows has no record of when they were last opened, so they may still be in use."),
                                "installed {n} months ago",
                            ),
                        };
                        theme::section_label(ui, title);
                        small_muted(ui, subtitle);
                        ui.add_space(6.0);
                        let total: u64 = wasters.iter().map(|w| w.1).sum();
                        theme::deck_panel(ui, |ui| {
                            for (p, size, days) in wasters {
                                ui.horizontal(|ui| {
                                    if super::a11y::explain(ui.link(RichText::new(&p.name).strong()), &t("Open the Programs page with this program selected. Nothing is removed — uninstalling starts there, with a review.")).clicked() {
                                        self.programs.selected = Some(p.key.clone());
                                        self.programs.filter.clear();
                                        self.tab = Tab::Programs;
                                    }
                                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                        small_muted(ui, tf(row_note, &[("n", (days / 30).to_string())]));
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
                                if super::a11y::explain(ui.button(t("Review and uninstall…")), &t("Start the uninstall wizard for every program listed above. You review everything before anything is removed, and removals are backed up unless backups are off in Settings.")).clicked() {
                                    let chosen: Vec<_> = wasters.iter().map(|w| w.0.clone()).collect();
                                    self.start_uninstall(chosen, false);
                                }
                            });
                        });
                        if panel.unknown_opened > 0 {
                            small_muted(ui, tf("{n} more aren't listed: Windows has no record of when they were last opened. Find them on the Programs page.", &[("n", panel.unknown_opened.to_string())]));
                        }
                        if panel.unknown_size > 0 {
                            small_muted(ui, tf("{n} more aren't listed because their size isn't known.", &[("n", panel.unknown_size.to_string())]));
                        }
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
                                if super::a11y::explain(ui.link(RichText::new(name).strong()), &t("Open the Install Logs page with this install selected, to see everything it changed or to remove it precisely.")).clicked() {
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
                    "Open the Programs page to uninstall programs.",
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

    use super::super::programs::UnusedBasis;

    fn candidate(name: &str, size: u64, basis: Option<UnusedBasis>) -> WasterCandidate {
        let mut program = installscope_core::programs::InstalledProgram::default();
        program.key = name.into();
        program.name = name.into();
        WasterCandidate {
            program,
            size,
            basis,
        }
    }

    fn entry_names(panel: &WasterPanel) -> Vec<String> {
        panel.entries.iter().map(|e| e.0.name.clone()).collect()
    }

    #[test]
    fn wasters_rank_measured_programs_by_size() {
        // All sizes known, all ages measured: the panel is the measured
        // list, largest first, with the 180-day boundary included and
        // recently opened programs left out however big they are.
        let candidates = vec![
            candidate("alpha", 300, Some(UnusedBasis::LastOpened(400))),
            candidate("beta", 100, Some(UnusedBasis::LastOpened(200))),
            candidate("huge-but-used", 999, Some(UnusedBasis::LastOpened(10))),
            candidate("boundary", 500, Some(UnusedBasis::LastOpened(180))),
        ];
        let panel = space_wasters(&candidates).expect("panel");
        assert_eq!(panel.kind, WasterKind::NotOpened);
        assert_eq!(entry_names(&panel), ["boundary", "alpha", "beta"]);
        assert_eq!(panel.entries[0].2, 180);
        assert_eq!(panel.unknown_opened, 0);
        assert_eq!(panel.unknown_size, 0);
    }

    #[test]
    fn wasters_mixed_evidence_never_calls_install_age_not_opened() {
        // One measured program qualifies. The install-age-only program is
        // bigger, but listing it under "not opened" would be a guess, so
        // it is counted as not listed instead; the size-unknown program
        // is counted too, and too-young / no-evidence programs vanish
        // from every count.
        let candidates = vec![
            candidate("measured", 100, Some(UnusedBasis::LastOpened(300))),
            candidate("install-age-only", 900, Some(UnusedBasis::Installed(400))),
            candidate("size-unknown", 0, Some(UnusedBasis::LastOpened(300))),
            candidate("too-young", 700, Some(UnusedBasis::Installed(90))),
            candidate("no-evidence", 800, None),
        ];
        let panel = space_wasters(&candidates).expect("panel");
        assert_eq!(panel.kind, WasterKind::NotOpened);
        assert_eq!(entry_names(&panel), ["measured"]);
        assert_eq!(panel.unknown_opened, 1);
        assert_eq!(panel.unknown_size, 1);
    }

    #[test]
    fn wasters_without_any_measurement_use_install_age() {
        // Nothing has a recorded last-opened date: the panel presents the
        // list for what it is — big programs installed long ago, carrying
        // their install ages, not invented last-opened ages.
        let candidates = vec![
            candidate("older", 100, Some(UnusedBasis::Installed(500))),
            candidate("bigger", 300, Some(UnusedBasis::Installed(200))),
            candidate("recent-install", 999, Some(UnusedBasis::Installed(30))),
        ];
        let panel = space_wasters(&candidates).expect("panel");
        assert_eq!(panel.kind, WasterKind::InstalledLongAgo);
        assert_eq!(entry_names(&panel), ["bigger", "older"]);
        assert_eq!(panel.entries[0].2, 200);
        assert_eq!(panel.unknown_opened, 0);
    }

    #[test]
    fn wasters_no_known_sizes_or_no_old_programs_no_panel() {
        // Every old program's size is unknown: nothing can be ranked, so
        // there is no panel rather than a list ordered by something else.
        let candidates = vec![
            candidate("a", 0, Some(UnusedBasis::LastOpened(400))),
            candidate("b", 0, Some(UnusedBasis::Installed(400))),
        ];
        assert!(space_wasters(&candidates).is_none());
        // Nothing old enough on either basis: also no panel.
        let candidates = vec![
            candidate("a", 500, Some(UnusedBasis::LastOpened(179))),
            candidate("b", 500, None),
        ];
        assert!(space_wasters(&candidates).is_none());
        assert!(space_wasters(&[]).is_none());
    }

    #[test]
    fn wasters_list_is_capped_at_five_largest() {
        let candidates: Vec<WasterCandidate> = (1..=7u64)
            .map(|i| {
                candidate(
                    &format!("p{i}"),
                    i * 100,
                    Some(UnusedBasis::LastOpened(365)),
                )
            })
            .collect();
        let panel = space_wasters(&candidates).expect("panel");
        assert_eq!(panel.entries.len(), 5);
        assert_eq!(entry_names(&panel), ["p7", "p6", "p5", "p4", "p3"]);
    }
}
