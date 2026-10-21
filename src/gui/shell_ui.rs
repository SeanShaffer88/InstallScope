//! Right-click menu manager page: every context-menu entry with a
//! reversible switch, broken entries left by removed programs, and a deep
//! clean of orphaned shell extensions, COM classes and environment entries.

use super::app::App;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use egui_extras::{Column, TableBuilder};
use installscope_core::i18n::{t, tf};
use installscope_core::shellext::{self, Kind, MenuEntry};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ShellView {
    #[default]
    Programs,
    Broken,
    Disabled,
    All,
}

#[derive(Default)]
pub struct ShellState {
    pub loaded: bool,
    pub entries: Option<Vec<MenuEntry>>,
    pub job: Option<Job<Vec<MenuEntry>>>,
    pub deep_job: Option<Job<installscope_core::revert::Plan>>,
    pub view: ShellView,
    pub filter: String,
}

/// Leftover kinds the deep clean looks for.
const DEEP_KINDS: [&str; 7] = [
    "COM registration",
    "Type library",
    "Shell extension",
    "Context menu entry",
    "Open with entry",
    "PATH entry",
    "Environment variable",
];

impl App {
    pub fn refresh_shell(&mut self, ctx: &egui::Context) {
        self.shell.loaded = true;
        let reg = self.reg.clone();
        self.shell.job = Some(Job::spawn(ctx, move |_| shellext::scan(reg.as_ref())));
    }

    pub fn poll_shell(&mut self) {
        if let Some(list) = self.shell.job.as_ref().and_then(|j| j.poll()) {
            self.shell.job = None;
            self.shell.entries = Some(list);
        }
        if let Some(mut plan) = self.shell.deep_job.as_ref().and_then(|j| j.poll()) {
            self.shell.deep_job = None;
            plan.items
                .retain(|i| DEEP_KINDS.iter().any(|k| i.note.starts_with(k)));
            if plan.items.is_empty() {
                self.set_status(t(
                    "No orphaned shell extensions, COM classes or environment entries found.",
                ));
                return;
            }
            let mut flow =
                super::plan::RemovalFlow::new(t("Shell and COM leftovers"), plan, false, false);
            flow.plan.info.push(t(
                "These shell extensions, COM classes, \"Open with\" entries and environment entries point to programs that are gone. Windows' own entries are never included.",
            ));
            self.open_removal(flow);
        }
    }

    fn toggle_shell(&mut self, e: MenuEntry, on: bool) {
        let title = if on {
            tf(
                "Turn on right-click entry {name}",
                &[("name", e.name.clone())],
            )
        } else {
            tf(
                "Turn off right-click entry {name}",
                &[("name", e.name.clone())],
            )
        };
        let machine = self.elevated;
        let mut w = if self.settings.backup_enabled() {
            match installscope_core::backup::BackupWriter::new(&self.store.backups_dir(), &title) {
                Ok(writer) => Some(writer),
                Err(error) => {
                    self.set_status(format!(
                        "✖ {title}: could not create a backup; no change was made: {error:#}"
                    ));
                    self.reload_backups();
                    return;
                }
            }
        } else {
            None
        };
        let mut r = shellext::set_enabled(self.reg.as_ref(), &e, on, machine, w.as_mut());
        if let Some(w) = w {
            if w.is_empty() {
                w.discard();
            } else if let Err(error) = w.finish() {
                r = Err(anyhow::anyhow!(
                    "the change may have completed, but its backup could not be finalized: {error:#}"
                ));
            }
        }
        match r {
            Ok(()) => self.set_status(format!("✔ {title}")),
            Err(err) => self.set_status(format!("✖ {title}: {err:#}")),
        }
        self.reload_backups();
        if let Some(list) = &mut self.shell.entries {
            for x in list
                .iter_mut()
                .filter(|x| x.key == e.key || (x.clsid.is_some() && x.clsid == e.clsid))
            {
                x.enabled = on;
            }
        }
    }

    pub fn ui_shell(&mut self, ctx: &egui::Context) {
        if !self.shell.loaded {
            self.refresh_shell(ctx);
        }
        let advanced = self.settings.advanced_mode;
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            // The page's explanation line (see a11y::explain): the focused
            // control's explanation, for keyboard and touch users who
            // never see a hover tooltip.
            super::a11y::focus_hint(ui, theme::MUTED);
            let Some(all) = self.shell.entries.clone() else {
                job_progress(ui, &t("Reading the right-click menu…"), 0);
                return;
            };
            let broken = all.iter().filter(|e| e.orphaned && !e.windows).count();
            let off = all.iter().filter(|e| !e.enabled).count();
            let added = all.iter().filter(|e| !e.windows).count();
            ui.horizontal_wrapped(|ui| {
                theme::badge(ui, tf("{n} added by programs", &[("n", added.to_string())]), theme::MUTED);
                theme::badge(ui, tf("{n} broken", &[("n", broken.to_string())]), if broken > 0 { theme::BAD } else { theme::OK });
                theme::badge(ui, tf("{n} turned off", &[("n", off.to_string())]), theme::MUTED);
            });
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                super::a11y::explain(
                    search_box(ui, &mut self.shell.filter, "Search the menu", 220.0),
                    &t("Show only entries whose name or target contains this text."),
                );
                for (v, label) in [
                    (ShellView::Programs, t("Added by programs")),
                    (ShellView::Broken, t("Broken")),
                    (ShellView::Disabled, t("Turned off")),
                    (ShellView::All, t("All, including Windows")),
                ] {
                    if v == ShellView::All && !advanced {
                        continue;
                    }
                    let why = match v {
                        ShellView::Programs => {
                            t("Show only entries added by programs, not Windows' own entries.")
                        }
                        ShellView::Broken => {
                            t("Show only entries whose program is gone, so they no longer work.")
                        }
                        ShellView::Disabled => t("Show only entries that are currently turned off."),
                        ShellView::All => t("Show every entry, including Windows' own entries."),
                    };
                    if super::a11y::explain(ui.selectable_label(self.shell.view == v, label), &why)
                        .clicked()
                    {
                        self.shell.view = v;
                    }
                }
            });
            ui.add_space(4.0);
            let refresh_why = if self.shell.job.is_some() {
                t("Unavailable while the right-click menu is being read.")
            } else {
                t("Reads the right-click menu again, to pick up entries added or removed since.")
            };
            ui.horizontal_wrapped(|ui| {
                {
                    if super::a11y::explain(
                        ui.add_enabled(self.shell.job.is_none(), egui::Button::new(format!("⟳ {}", t("Refresh")))),
                        &refresh_why,
                    )
                    .clicked()
                    {
                        self.refresh_shell(ctx);
                    }
                    if super::a11y::explain(
                        ui.add_enabled(self.shell.deep_job.is_none(), egui::Button::new(t("Deep clean…"))),
                        &t("Find shell extensions, COM classes, \"Open with\" entries and PATH or environment entries left by removed programs, anywhere in Windows."),
                    )
                    .clicked()
                    {
                        let reg = self.reg.clone();
                        self.shell.deep_job = Some(Job::spawn(ctx, move |_| installscope_core::orphans::scan_system(reg.as_ref())));
                    }
                    if broken > 0 && super::a11y::explain(
                        theme::primary_button(ui, tf("Remove {n} broken…", &[("n", broken.to_string())])),
                        &t("Opens a review of all the broken entries. Nothing is removed until you confirm there, and what is removed is backed up first, unless you have turned backups off in Settings."),
                    )
                    .clicked()
                    {
                        let plan = shellext::orphan_plan(&all);
                        let mut flow = super::plan::RemovalFlow::new(t("Broken right-click entries"), plan, false, false);
                        flow.plan.info.push(t("These entries belong to programs that are no longer installed; Explorer can slow down or show errors trying to load them."));
                        self.open_removal(flow);
                        self.shell.loaded = false;
                    }
                }
            });
            if self.shell.deep_job.is_some() {
                job_progress(ui, &t("Looking for leftovers of removed programs…"), 0);
            }
            ui.add_space(6.0);
            let f = self.shell.filter.to_lowercase();
            let rows: Vec<MenuEntry> = all
                .into_iter()
                .filter(|e| match self.shell.view {
                    ShellView::Programs => !e.windows,
                    ShellView::Broken => e.orphaned && !e.windows,
                    ShellView::Disabled => !e.enabled,
                    ShellView::All => true,
                })
                .filter(|e| f.is_empty() || e.name.to_lowercase().contains(&f) || e.target.to_lowercase().contains(&f))
                .collect();
            if rows.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| muted(ui, t("Nothing to show here.")));
                return;
            }
            let mut toggle: Option<(MenuEntry, bool)> = None;
            TableBuilder::new(ui)
                .striped(true)
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(34.0))
                .column(Column::initial(230.0).clip(true))
                .column(Column::initial(150.0).clip(true))
                .column(Column::initial(110.0))
                .column(Column::remainder().clip(true))
                .header(24.0, |mut h| {
                    for l in ["On", "Entry", "Shown on", "Type", "Provided by"] {
                        h.col(|ui| {
                            ui.strong(t(l));
                        });
                    }
                })
                .body(|body| {
                    body.rows(26.0, rows.len(), |mut row| {
                        let e = &rows[row.index()];
                        row.col(|ui| {
                            let mut on = e.enabled;
                            let protected = e.windows && !advanced;
                            let why = if protected {
                                t("This is one of Windows' own entries, so it can't be changed unless Advanced mode is on in Settings.")
                            } else {
                                t("Turn this entry on or off; nothing is deleted.")
                            };
                            let resp = super::a11y::explain(
                                ui.add_enabled(!protected, egui::Checkbox::without_text(&mut on)),
                                &why,
                            );
                            resp.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Checkbox,
                                    true,
                                    on,
                                    &e.name,
                                )
                            });
                            if resp.changed() {
                                toggle = Some((e.clone(), on));
                            }
                        });
                        row.col(|ui| {
                            super::a11y::explain(
                                ui.add(
                                    egui::Label::new(RichText::new(&e.name).strong())
                                        .truncate()
                                        .selectable(true),
                                ),
                                &e.name,
                            );
                            if e.orphaned && !e.windows {
                                theme::badge(ui, t("Broken"), theme::BAD);
                            }
                        });
                        row.col(|ui| {
                            ui.label(t(e.scope));
                        });
                        row.col(|ui| {
                            ui.label(match e.kind {
                                Kind::Handler => t("Shell extension"),
                                Kind::Command => t("Command"),
                            });
                        });
                        row.col(|ui| {
                            let shown = if e.target.is_empty() { t("(not registered)") } else { e.target.clone() };
                            super::a11y::explain(
                                ui.add(
                                    egui::Label::new(RichText::new(shown).color(theme::MUTED))
                                        .truncate()
                                        .selectable(true),
                                ),
                                &e.key,
                            );
                        });
                    });
                });
            if let Some((e, on)) = toggle {
                self.toggle_shell(e, on);
            }
            ui.add_space(6.0);
            small_muted(ui, t("Changes apply to new File Explorer windows. Turning entries off is reversible and backed up first, unless you have turned backups off in Settings."));
        });
    }
}
