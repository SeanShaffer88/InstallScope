//! "Ask InstallScope" (Ctrl+K): search programs and install logs in plain
//! language, e.g. "find and wipe everything related to that video converter
//! I downloaded last week". Shows how the question was understood; removal
//! still goes through the usual review.

use super::app::{App, Tab};
use super::theme;
use super::widgets::*;
use egui::RichText;
use installscope_core::i18n::{t, tf};
use installscope_core::model::human_size;
use installscope_core::nlquery::{self, Action, Query};
use installscope_core::programs::InstalledProgram;
use std::collections::HashSet;

#[derive(Default)]
pub struct AskState {
    pub open: bool,
    pub text: String,
    focus: bool,
    query: Option<Query>,
    programs: Vec<(InstalledProgram, u64)>,
    logs: Vec<(String, String, String)>,
    checked: HashSet<String>,
}

const EXAMPLES: [&str; 4] = [
    "Find and wipe everything related to that video converter I downloaded last week",
    "Top 5 space-wasters I haven't opened in 6 months",
    "Games installed this year bigger than 10 GB",
    "What did I install yesterday?",
];

impl App {
    pub fn open_ask(&mut self) {
        self.ask_ui.open = true;
        self.ask_ui.focus = true;
    }

    fn run_ask(&mut self) {
        let today = installscope_core::time::Date::today_local();
        let q = nlquery::parse(&self.ask_ui.text, today);
        let size_of = |p: &InstalledProgram| {
            if p.size_kb > 0 {
                p.size_kb * 1024
            } else {
                self.programs.sizes.get(&p.key).copied().unwrap_or(0)
            }
        };
        let mut found: Vec<(InstalledProgram, u64, u32)> = Vec::new();
        for p in &self.programs.list {
            let date = nlquery::parse_date(&p.install_date);
            if (q.since.is_some() || q.until.is_some()) && !q.date_ok(date) {
                continue;
            }
            let size = size_of(p);
            if q.min_size.is_some_and(|m| size < m) {
                continue;
            }
            if let Some(days) = q.unused_days {
                if !self.unused_days(p).is_some_and(|d| d >= days as i64) {
                    continue;
                }
            }
            let score = if q.has_terms() {
                let s = q.score_text(&format!(
                    "{} {} {} {}",
                    p.name, p.publisher, p.install_location, p.comments
                ));
                if s == 0 {
                    continue;
                }
                s
            } else {
                1
            };
            found.push((p.clone(), size, score));
        }
        if q.min_size.is_some() || q.top.is_some() || q.unused_days.is_some() {
            found.sort_by_key(|f| std::cmp::Reverse(f.1));
        } else {
            found.sort_by(|a, b| b.2.cmp(&a.2).then(b.0.install_date.cmp(&a.0.install_date)));
        }
        if let Some(n) = q.top {
            found.truncate(n);
        }
        let logs: Vec<(String, String, String)> = self
            .logs
            .logs
            .iter()
            .filter(|l| {
                // Logs have no size or usage, so a question about those
                // only lists logs that match its words.
                let program_only =
                    q.min_size.is_some() || q.unused_days.is_some() || q.top.is_some();
                if program_only && !q.has_terms() {
                    return false;
                }
                let date = nlquery::parse_date(&l.created);
                (q.since.is_none() && q.until.is_none() || q.date_ok(date))
                    && (!q.has_terms()
                        || q.score_text(&format!("{} {} {}", l.name, l.publisher, l.installer)) > 0)
            })
            .map(|l| (l.id.clone(), l.name.clone(), l.created.clone()))
            .collect();
        self.ask_ui.checked = if q.action == Action::Remove {
            found.iter().map(|f| f.0.key.clone()).collect()
        } else {
            HashSet::new()
        };
        self.ask_ui.programs = found.into_iter().map(|(p, s, _)| (p, s)).collect();
        self.ask_ui.logs = logs;
        self.ask_ui.query = Some(q);
    }

    pub fn ui_ask(&mut self, ctx: &egui::Context) {
        if !self.ask_ui.open {
            return;
        }
        let mut open = true;
        let mut run = false;
        let mut uninstall: Option<Vec<InstalledProgram>> = None;
        let mut goto_program: Option<String> = None;
        let mut goto_log: Option<String> = None;
        egui::Window::new(format!("✨ {}", t("Ask InstallScope")))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(640.0)
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 80.0))
            .show(ctx, |ui| {
                small_muted(ui, t("Ask in your own words. InstallScope works it out on this PC; nothing is sent anywhere."));
                ui.add_space(4.0);
                let r = super::a11y::explain(
                    ui.add(
                        egui::TextEdit::singleline(&mut self.ask_ui.text)
                            .hint_text(t("e.g. remove the video converter I downloaded last week"))
                            .desired_width(f32::INFINITY),
                    ),
                    &t("Ask in your own words. InstallScope works out what you mean on this PC and searches your programs and install logs; nothing is sent anywhere."),
                );
                if self.ask_ui.focus {
                    r.request_focus();
                    self.ask_ui.focus = false;
                }
                // Results follow the typing; the search is in memory and instant.
                if r.changed() {
                    if self.ask_ui.text.trim().is_empty() {
                        self.ask_ui.query = None;
                    } else {
                        run = true;
                    }
                }
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    run = true;
                }
                if self.ask_ui.query.is_none() {
                    ui.add_space(6.0);
                    small_muted(ui, t("Try:"));
                    for ex in EXAMPLES {
                        if super::a11y::explain(ui.link(t(ex)), &t("Use this example question."))
                            .clicked()
                        {
                            self.ask_ui.text = t(ex);
                            run = true;
                        }
                    }
                }
                let Some(q) = &self.ask_ui.query else { return };
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(t("Understood as:")).color(theme::MUTED));
                    ui.label(RichText::new(q.explain()).strong());
                });
                if q.unused_days.is_some() && self.programs.usage.is_empty() {
                    small_muted(ui, t("Still reading how programs are used; ask again in a moment."));
                }
                ui.separator();
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    if self.ask_ui.programs.is_empty() && self.ask_ui.logs.is_empty() {
                        muted(ui, t("Nothing matches."));
                    }
                    for (p, size) in &self.ask_ui.programs {
                        ui.horizontal(|ui| {
                            if q.action == Action::Remove {
                                let mut on = self.ask_ui.checked.contains(&p.key);
                                let resp = super::a11y::explain(
                                    ui.checkbox(&mut on, ""),
                                    &t("Tick to include this program in the removal review below."),
                                );
                                resp.widget_info(|| {
                                    egui::WidgetInfo::selected(
                                        egui::WidgetType::Checkbox,
                                        true,
                                        on,
                                        &p.name,
                                    )
                                });
                                if resp.changed() {
                                    if on {
                                        self.ask_ui.checked.insert(p.key.clone());
                                    } else {
                                        self.ask_ui.checked.remove(&p.key);
                                    }
                                }
                            }
                            if super::a11y::explain(
                                ui.link(RichText::new(&p.name).strong()),
                                &t("Opens this program on the Programs page."),
                            )
                            .clicked()
                            {
                                goto_program = Some(p.key.clone());
                            }
                            small_muted(ui, format!("{}  {}  {}", p.publisher, p.install_date, if *size > 0 { human_size(*size) } else { String::new() }));
                        });
                    }
                    if !self.ask_ui.logs.is_empty() {
                        ui.add_space(6.0);
                        small_muted(ui, t("Install logs"));
                        for (id, name, created) in &self.ask_ui.logs {
                            ui.horizontal(|ui| {
                                if super::a11y::explain(
                                    ui.link(name),
                                    &t("Opens this install log on the Logs page."),
                                )
                                .clicked()
                                {
                                    goto_log = Some(id.clone());
                                }
                                small_muted(ui, created);
                            });
                        }
                    }
                });
                if q.action == Action::Remove && !self.ask_ui.programs.is_empty() {
                    ui.separator();
                    let n = self.ask_ui.checked.len();
                    let review_why = if n == 0 {
                        t("Unavailable: tick at least one program above first.")
                    } else {
                        t("Opens the removal review for the ticked programs. Nothing is removed until you go through that review and confirm.")
                    };
                    ui.horizontal(|ui| {
                        if super::a11y::explain(
                            ui.add_enabled(n > 0, egui::Button::new(RichText::new(tf("Review removal of {n} programs…", &[("n", n.to_string())])).color(egui::Color32::WHITE)).fill(theme::ACCENT)),
                            &review_why,
                        )
                        .clicked()
                        {
                            uninstall = Some(
                                self.ask_ui
                                    .programs
                                    .iter()
                                    .filter(|(p, _)| self.ask_ui.checked.contains(&p.key))
                                    .map(|(p, _)| p.clone())
                                    .collect(),
                            );
                        }
                        small_muted(ui, t("You'll see every file and registry entry before anything is removed, and what InstallScope removes is backed up first, unless you have turned backups off in Settings."));
                    });
                }
            });
        if run {
            self.run_ask();
        }
        if let Some(key) = goto_program {
            self.programs.selected = Some(key);
            self.programs.filter.clear();
            self.tab = Tab::Programs;
        }
        if let Some(id) = goto_log {
            self.logs.selected = Some(id);
            self.tab = Tab::Logs;
        }
        if let Some(list) = uninstall {
            open = false;
            self.start_uninstall(list, false);
        }
        if !open || ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.ask_ui.open = false;
        }
    }
}
