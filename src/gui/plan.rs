//! Reviewing and executing a removal plan (shared by uninstall, log revert,
//! leftover scans and broken-entry cleanup).

use super::app::App;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::backup::{ActionResult, BackupWriter};
use installscope_core::i18n::{t, tf};
use installscope_core::refcount::{annotate_plan, RefLabel, ReferenceIndex};
use installscope_core::revert::{execute_plan, ExecOptions, Plan, Risk};
use std::collections::BTreeSet;

pub struct RemovalFlow {
    pub title: String,
    pub plan: Plan,
    /// Offer the "also remove files created later" option (log-based plans).
    pub offer_unlogged: bool,
    pub remove_unlogged: bool,
    pub hide_missing: bool,
    pub filter: String,
    /// Install logs to delete after a successful removal.
    pub logs_to_delete: Vec<String>,
    pub delete_logs: bool,
    /// Programs this flow is removing (for the status line).
    pub names: Vec<String>,
    /// Items hidden because Simple mode doesn't offer them (None = not yet filtered).
    pub hidden_for_safety: Option<usize>,
    /// Undoing a trial install: the exact log, never trimmed for Simple mode.
    pub trial: bool,
    pub job: Option<Job<(Vec<ActionResult>, Option<String>)>>,
    pub results: Option<(Vec<ActionResult>, Option<String>)>,
    pub open: bool,
    /// Confidence per plan item, computed once.
    pub scores: Vec<installscope_core::revert::Confidence>,
    /// Reference-count label per plan item, computed once (parallel to
    /// `plan.items`, like `scores`).
    pub ref_labels: Vec<RefLabel>,
    /// "Reference data covers X of your Y programs" for this review.
    pub ref_coverage: Option<String>,
    /// Install logs this flow removes (the owners the labels are relative
    /// to — the same logs `logs_to_delete` names).
    pub ref_log_ids: Vec<String>,
    /// The reference index this review was labelled from, kept so labels
    /// can be recomputed if the plan is filtered (Simple mode).
    pub ref_index: Option<ReferenceIndex>,
}

impl RemovalFlow {
    pub fn new(title: String, plan: Plan, offer_unlogged: bool, remove_unlogged: bool) -> Self {
        RemovalFlow {
            title,
            plan,
            offer_unlogged,
            remove_unlogged,
            hide_missing: true,
            filter: String::new(),
            logs_to_delete: Vec::new(),
            delete_logs: true,
            names: Vec::new(),
            hidden_for_safety: None,
            trial: false,
            job: None,
            results: None,
            open: true,
            scores: Vec::new(),
            ref_labels: Vec::new(),
            ref_coverage: None,
            ref_log_ids: Vec::new(),
            ref_index: None,
        }
    }
}

fn category_label(c: &str) -> String {
    t(c)
}

/// Checkbox list of plan items grouped by category.
pub fn plan_ui(ui: &mut egui::Ui, flow: &mut RemovalFlow) {
    if flow.scores.len() != flow.plan.items.len() {
        flow.scores = flow
            .plan
            .items
            .iter()
            .map(installscope_core::revert::confidence)
            .collect();
    }
    // Recompute reference labels the same way (e.g. after Simple mode
    // filtered the plan).
    if flow.ref_index.is_some() && flow.ref_labels.len() != flow.plan.items.len() {
        let removing: BTreeSet<String> = flow.ref_log_ids.iter().cloned().collect();
        let new_labels = flow
            .ref_index
            .as_ref()
            .map(|index| index.labels(&flow.plan, &removing));
        if let Some(labels) = new_labels {
            flow.ref_labels = labels;
        }
    }
    let (mut green, mut yellow, mut red) = (0, 0, 0);
    for (i, c) in flow.plan.items.iter().zip(&flow.scores) {
        if !i.exists {
            continue;
        }
        match c.band() {
            Risk::Safe => green += 1,
            Risk::Review => yellow += 1,
            Risk::Careful => red += 1,
        }
    }
    ui.horizontal_wrapped(|ui| {
        theme::badge(ui, tf("{n} safe", &[("n", green.to_string())]), theme::OK);
        theme::badge(
            ui,
            tf("{n} to review", &[("n", yellow.to_string())]),
            theme::WARN,
        );
        theme::badge(ui, tf("{n} careful", &[("n", red.to_string())]), theme::BAD);
        if ui
            .small_button(t("Select only green"))
            .on_hover_text(t(
                "Tick only the items InstallScope is confident are safe to remove.",
            ))
            .clicked()
        {
            let allow: Vec<bool> = (0..flow.plan.items.len())
                .map(|n| ref_auto_select(flow, n))
                .collect();
            for ((i, c), a) in flow.plan.items.iter_mut().zip(&flow.scores).zip(&allow) {
                i.selected = i.exists && c.band() == Risk::Safe && *a;
            }
        }
    });
    if let Some(coverage) = &flow.ref_coverage {
        ui.add_space(2.0);
        small_muted(ui, coverage);
    }
    ui.add_space(4.0);
    let missing = flow.plan.items.iter().filter(|i| !i.exists).count();
    ui.horizontal(|ui| {
        search_box(ui, &mut flow.filter, "Filter items", 240.0);
        if missing > 0 {
            ui.checkbox(
                &mut flow.hide_missing,
                tf(
                    "Hide {n} items already removed",
                    &[("n", missing.to_string())],
                ),
            );
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button(t("Select none")).clicked() {
                flow.plan.items.iter_mut().for_each(|i| i.selected = false);
            }
            if ui.small_button(t("Select all")).clicked() {
                let allow: Vec<bool> = (0..flow.plan.items.len())
                    .map(|n| ref_auto_select(flow, n))
                    .collect();
                for (n, i) in flow.plan.items.iter_mut().enumerate() {
                    if i.exists && i.note.is_empty() && allow[n] {
                        i.selected = true;
                    }
                }
            }
        });
    });
    ui.separator();
    let filter = flow.filter.to_lowercase();
    let mut cats: Vec<&'static str> = flow
        .plan
        .items
        .iter()
        .map(|i| i.action.category())
        .collect();
    cats.dedup();
    cats.sort();
    cats.dedup();
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(ui.available_height() - 90.0)
        .show(ui, |ui| {
            if flow.plan.items.is_empty() {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("✔").size(32.0).color(theme::OK));
                    ui.label(t("Nothing left behind. The system is clean."));
                });
            }
            for cat in cats {
                let idx: Vec<usize> = flow
                    .plan
                    .items
                    .iter()
                    .enumerate()
                    .filter(|(_, i)| i.action.category() == cat)
                    .filter(|(_, i)| !(flow.hide_missing && !i.exists))
                    .filter(|(_, i)| {
                        filter.is_empty() || i.action.describe().to_lowercase().contains(&filter)
                    })
                    .map(|(n, _)| n)
                    .collect();
                if idx.is_empty() {
                    continue;
                }
                let sel = idx.iter().filter(|&&n| flow.plan.items[n].selected).count();
                egui::CollapsingHeader::new(
                    RichText::new(format!("{}  ({sel}/{})", category_label(cat), idx.len()))
                        .strong(),
                )
                .id_salt(cat)
                .default_open(idx.len() <= 300)
                .show(ui, |ui| {
                    for n in idx {
                        let item = &mut flow.plan.items[n];
                        ui.horizontal(|ui| {
                            ui.add_enabled_ui(item.exists, |ui| {
                                ui.checkbox(&mut item.selected, "")
                            });
                            let text = item.action.describe();
                            let rt = if item.exists {
                                RichText::new(text)
                            } else {
                                RichText::new(text).color(theme::MUTED).strikethrough()
                            };
                            let conf = &flow.scores[n];
                            let risk = conf.band();
                            let rc = match risk {
                                Risk::Safe => theme::OK,
                                Risk::Review => theme::WARN,
                                Risk::Careful => theme::BAD,
                            };
                            let tip = std::iter::once(t(risk.explanation()))
                                .chain(conf.reasons.iter().map(|r| format!("• {}", t(r))))
                                .collect::<Vec<_>>()
                                .join("\n");
                            egui::Frame::new()
                                .fill(rc.gamma_multiply(0.18))
                                .corner_radius(8.0)
                                .inner_margin(egui::Margin::symmetric(6, 0))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{} {}%",
                                            t(risk.label()),
                                            conf.score
                                        ))
                                        .small()
                                        .strong()
                                        .color(rc),
                                    );
                                })
                                .response
                                .on_hover_text(tip);
                            ui.add(egui::Label::new(rt).truncate())
                                .on_hover_text(item.action.describe());
                            if !item.note.is_empty() {
                                theme::badge(ui, t(&item.note), theme::WARN);
                            }
                            if let Some(label) = flow.ref_labels.get(n) {
                                let color = match label {
                                    RefLabel::OnlyThisProgram => theme::OK,
                                    RefLabel::Shared { .. } => theme::BAD,
                                    RefLabel::NotInAnyLog => theme::MUTED,
                                };
                                theme::badge(ui, label.text(), color);
                            }
                        });
                    }
                });
            }
            if !flow.plan.info.is_empty() {
                egui::CollapsingHeader::new(
                    RichText::new(tf("{n} notes", &[("n", flow.plan.info.len().to_string())]))
                        .color(theme::MUTED),
                )
                .id_salt("info")
                .show(ui, |ui| {
                    for i in &flow.plan.info {
                        small_muted(ui, i);
                    }
                });
            }
        });
}

/// Whether a bulk "select" button (Select all, Select only green) may tick
/// item `n`: reference labels, if present, keep shared and unlogged items
/// out of auto-selection. Flows that never ran annotation have no labels
/// and keep their previous behavior.
fn ref_auto_select(flow: &RemovalFlow, n: usize) -> bool {
    flow.ref_labels.len() != flow.plan.items.len()
        || matches!(
            flow.ref_labels.get(n),
            Some(RefLabel::OnlyThisProgram) | None
        )
}

impl App {
    pub fn open_removal(&mut self, flow: RemovalFlow) {
        let mut flow = flow;
        // Label every item from the cross-program reference index: the
        // logs this flow removes own the removal, everything else counts
        // as "shared with others".
        let removing: BTreeSet<String> = flow.logs_to_delete.iter().cloned().collect();
        let index = ReferenceIndex::build(&self.logs.logs);
        flow.ref_labels = annotate_plan(&mut flow.plan, &index, &removing);
        flow.ref_log_ids = flow.logs_to_delete.clone();
        flow.ref_coverage = index.coverage_line(self.programs.list.len());
        flow.ref_index = Some(index);
        self.removal = Some(flow);
    }

    pub fn ui_removal(&mut self, ctx: &egui::Context) {
        let simple = !self.settings.advanced_mode;
        let installed: Vec<String> = self
            .programs
            .list
            .iter()
            .map(|p| p.name.to_lowercase())
            .collect();
        let alt = &mut self.alt;
        let Some(flow) = &mut self.removal else {
            return;
        };
        if simple
            && !flow.trial
            && flow.hidden_for_safety.is_none()
            && flow.job.is_none()
            && flow.results.is_none()
        {
            flow.hidden_for_safety = Some(installscope_core::revert::simplify(&mut flow.plan));
            flow.offer_unlogged = false;
            flow.remove_unlogged = false;
        }
        let mut start = false;
        let mut close = false;
        let mut open = flow.open;
        egui::Window::new(flow.title.clone())
            .id(egui::Id::new("removal"))
            .collapsible(false)
            .resizable(true)
            .default_size([780.0, 560.0])
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                if let Some(job) = &flow.job {
                    job_progress(ui, &t("Removing… everything is backed up first."), job.progress.count());
                    return;
                }
                if let Some((results, backup)) = &flow.results {
                    ui.heading(t("Done"));
                    if let Some(id) = backup {
                        small_muted(ui, tf("Backup {name} can undo all of this from the Backups tab.", &[("name", id.clone())]));
                    }
                    ui.add_space(6.0);
                    results_ui(ui, results);
                    ui.add_space(8.0);
                    super::alt_ui::alternatives_widget(ui, alt, &installed, &flow.names);
                    ui.add_space(8.0);
                    if theme::primary_button(ui, t("Close")).clicked() {
                        close = true;
                    }
                    return;
                }
                plan_ui(ui, flow);
                if let Some(n) = flow.hidden_for_safety.filter(|n| *n > 0) {
                    small_muted(
                        ui,
                        tf(
                            "{n} items that need expert judgment (Windows folders, services, shared settings or uncertain matches) were left alone. Advanced mode shows them.",
                            &[("n", n.to_string())],
                        ),
                    );
                }
                ui.separator();
                ui.horizontal_wrapped(|ui| {
                    if flow.offer_unlogged {
                        ui.checkbox(&mut flow.remove_unlogged, t("Also remove files the program created after installation"))
                            .on_hover_text(t("Settings, caches and logs it wrote into its own folders. They are still backed up."));
                    }
                    if !flow.logs_to_delete.is_empty() {
                        ui.checkbox(&mut flow.delete_logs, t("Delete the install log afterwards"));
                    }
                });
                ui.horizontal(|ui| {
                    let n = flow.plan.selected();
                    ui.add_enabled_ui(n > 0, |ui| {
                        if theme::primary_button(ui, tf("Remove {n} selected items", &[("n", n.to_string())])).clicked() {
                            start = true;
                        }
                    });
                    if ui.button(t("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
        if !open {
            close = true;
        }
        if flow.job.is_none() && self.confirm.is_none() && super::a11y::escape_pressed(ctx) {
            close = true;
        }
        if start {
            let plan = flow.plan.clone();
            let reg = self.reg.clone();
            let backup_root = self.store.backups_dir();
            let backup_on = self.settings.backup_enabled();
            let title = flow.title.clone();
            let opts = ExecOptions {
                remove_unlogged: flow.remove_unlogged,
                dry_run: false,
                use_recycle_bin: self.settings.use_recycle_bin && !simple,
                restart_explorer: self.settings.restart_explorer,
                close_programs: self.settings.close_programs,
            };
            flow.job = Some(Job::spawn(ctx, move |_| {
                let mut writer = if backup_on {
                    match BackupWriter::new(&backup_root, &title) {
                        Ok(writer) => Some(writer),
                        Err(error) => {
                            return (
                                vec![ActionResult::err(
                                    "Backup",
                                    tf(
                                        "Could not create a backup; no changes were made: {error}",
                                        &[("error", format!("{error:#}"))],
                                    ),
                                )],
                                None,
                            );
                        }
                    }
                } else {
                    None
                };
                let mut results = execute_plan(&plan, reg.as_ref(), writer.as_mut(), &opts);
                let id = match writer {
                    Some(w) if w.is_empty() => {
                        w.discard();
                        None
                    }
                    Some(w) => match w.finish() {
                        Ok(backup) => Some(backup.id),
                        Err(error) => {
                            results.push(ActionResult::err(
                                "Backup",
                                format!(
                                    "Changes may have completed, but the backup could not be finalized: {error:#}"
                                ),
                            ));
                            None
                        }
                    },
                    None => None,
                };
                // Canary: post-operation health verification (read-only).
                // Appends its verdict rows to this same results surface; the
                // backup above (or its absence) remains the Undo.
                {
                    use installscope_core::canary;
                    use installscope_core::leftovers::DeepIndex;
                    use installscope_core::platform::expand_env;
                    use installscope_core::util::protected_dirs;
                    let ix = DeepIndex::build(reg.as_ref(), None);
                    let exists: &dyn Fn(&str) -> bool = &|p| std::path::Path::new(p).exists();
                    let verdict = canary::run_from_plan(
                        reg.as_ref(),
                        &ix,
                        &plan,
                        exists,
                        &expand_env("%SystemRoot%"),
                        &protected_dirs(),
                    );
                    results.extend(verdict.to_action_results());
                }
                (results, id)
            }));
        }
        if close {
            self.removal = None;
        }
    }

    pub fn poll_removal(&mut self) {
        let Some(flow) = &mut self.removal else {
            return;
        };
        let Some(job) = &flow.job else { return };
        let Some(res) = job.poll() else { return };
        flow.job = None;
        let failed = res.0.iter().filter(|r| !r.ok).count();
        let names = flow.names.join(", ");
        let logs: Vec<String> = if flow.delete_logs && failed == 0 {
            flow.logs_to_delete.clone()
        } else {
            Vec::new()
        };
        flow.results = Some(res);
        for id in logs {
            let _ = self.store.delete_log(&id);
        }
        self.reload_logs();
        self.reload_backups();
        self.programs.needs_refresh = true;
        self.status = if failed == 0 {
            tf("Removed {name}.", &[("name", names)])
        } else {
            tf("Finished with {n} problems.", &[("n", failed.to_string())])
        };
    }
}
