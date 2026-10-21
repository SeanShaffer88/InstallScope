//! Install logs page: browse monitored installations, search which program
//! created a path, export, and uninstall precisely.

use super::app::{App, Confirm};
use super::plan::RemovalFlow;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::i18n::{t, tf};
use installscope_core::model::{human_size, ChangeSet, InstallLog};
use installscope_core::report;
use installscope_core::revert::build_plan;

#[derive(Default)]
pub struct LogsState {
    pub logs: Vec<InstallLog>,
    pub selected: Option<String>,
    pub filter: String,
    /// "Which program created this?" search.
    pub path_search: String,
    pub editing: bool,
    /// "Which program created this?" results for the last query:
    /// (query, logs fingerprint, [(log id, name, match count, first match)]).
    pub search_cache: Option<(String, usize, Vec<(String, String, usize, String)>)>,
    /// Change statistics per log id, computed once when logs load.
    pub stats: std::collections::HashMap<String, installscope_core::model::ChangeStats>,
    /// Tags being edited, as typed.
    pub tags_text: String,
    /// Show only this group ("" = all).
    pub group_filter: String,
    pub loader: super::Reload<Vec<InstallLog>>,
    /// The visual map of the selected log's changes.
    pub tree: super::difftree_ui::TreeView,
    /// Show the plain list instead of the map.
    pub show_list: bool,
    /// Making a silent installer: (folder, Setup.exe or why it wasn't made).
    pub package_job:
        Option<Job<Result<(std::path::PathBuf, Result<std::path::PathBuf, String>), String>>>,
}

/// Collapsible lists of every change. With `editable`, each entry gets a ✖
/// button to drop it (used to strip noise before saving a capture).
pub fn changes_ui(ui: &mut egui::Ui, cs: &mut ChangeSet, editable: bool) {
    fn section<T>(
        ui: &mut egui::Ui,
        id: &str,
        title: String,
        hint: String,
        color: egui::Color32,
        items: &mut Vec<T>,
        editable: bool,
        render: impl Fn(&T) -> (String, String),
    ) {
        if items.is_empty() {
            return;
        }
        let mut remove: Option<usize> = None;
        let header = egui::CollapsingHeader::new(
            RichText::new(format!("{title}  ({})", items.len()))
                .color(color)
                .strong(),
        )
        .id_salt(id)
        .default_open(items.len() <= 40)
        .show(ui, |ui| {
            let row_h = 20.0;
            egui::ScrollArea::vertical()
                .id_salt(id)
                .max_height(320.0)
                .auto_shrink([false, true])
                .show_rows(ui, row_h, items.len(), |ui, range| {
                    for i in range {
                        let (main, detail) = render(&items[i]);
                        ui.horizontal(|ui| {
                            if editable
                                && super::a11y::explain(
                                    ui.small_button("✖"),
                                    &t("Not part of this program — remove it from this log. It stays on this PC, and uninstalling from this log will not remove or restore it later."),
                                )
                                .clicked()
                            {
                                remove = Some(i);
                            }
                            super::a11y::explain(
                                ui.add(
                                    egui::Label::new(RichText::new(&main).monospace())
                                        .truncate()
                                        .selectable(true),
                                ),
                                &main,
                            );
                            if !detail.is_empty() {
                                super::a11y::explain(
                                    ui.add(
                                        egui::Label::new(
                                            RichText::new(&detail).small().color(theme::MUTED),
                                        )
                                        .truncate(),
                                    ),
                                    &detail,
                                );
                            }
                        });
                    }
                });
        });
        super::a11y::explain(header.header_response, &hint);
        if let Some(i) = remove {
            items.remove(i);
        }
    }
    section(
        ui,
        "fa",
        t("Files added"),
        t("Show or hide the files the installation added."),
        theme::ADDED,
        &mut cs.files_added,
        editable,
        |f| {
            (
                f.path.clone(),
                if f.is_dir {
                    t("folder")
                } else {
                    human_size(f.size)
                },
            )
        },
    );
    section(
        ui,
        "ka",
        t("Registry keys added"),
        t("Show or hide the registry keys the installation added."),
        theme::ADDED,
        &mut cs.keys_added,
        editable,
        |k| {
            (
                k.path.clone(),
                tf("{n} values", &[("n", k.values.len().to_string())]),
            )
        },
    );
    section(
        ui,
        "va",
        t("Registry values added"),
        t("Show or hide the registry values the installation added."),
        theme::ADDED,
        &mut cs.values_added,
        editable,
        |v| {
            (
                format!("{}\\{}", v.key, v.value.display_name()),
                v.value.display_data(),
            )
        },
    );
    section(
        ui,
        "vm",
        t("Registry values changed"),
        t("Show or hide the registry values the installation changed."),
        theme::CHANGED,
        &mut cs.values_modified,
        editable,
        |v| {
            (
                format!("{}\\{}", v.key, v.after.display_name()),
                format!("{} → {}", v.before.display_data(), v.after.display_data()),
            )
        },
    );
    section(
        ui,
        "fm",
        t("Files changed"),
        t("Show or hide the files the installation changed."),
        theme::CHANGED,
        &mut cs.files_modified,
        editable,
        |f| {
            (
                f.after.path.clone(),
                format!(
                    "{} → {}",
                    human_size(f.before.size),
                    human_size(f.after.size)
                ),
            )
        },
    );
    section(
        ui,
        "vr",
        t("Registry values removed"),
        t("Show or hide the registry values the installation removed."),
        theme::REMOVED,
        &mut cs.values_removed,
        editable,
        |v| {
            (
                format!("{}\\{}", v.key, v.value.display_name()),
                v.value.display_data(),
            )
        },
    );
    section(
        ui,
        "kr",
        t("Registry keys removed"),
        t("Show or hide the registry keys the installation removed."),
        theme::REMOVED,
        &mut cs.keys_removed,
        editable,
        |k| (k.path.clone(), String::new()),
    );
    section(
        ui,
        "fr",
        t("Files removed"),
        t("Show or hide the files the installation removed."),
        theme::REMOVED,
        &mut cs.files_removed,
        editable,
        |f| (f.path.clone(), String::new()),
    );
    if cs.is_empty() {
        muted(ui, t("No changes were recorded."));
    }
}

fn clone_meta(l: &InstallLog) -> InstallLog {
    InstallLog {
        id: l.id.clone(),
        name: l.name.clone(),
        publisher: l.publisher.clone(),
        version: l.version.clone(),
        created: l.created.clone(),
        installer: l.installer.clone(),
        uninstall_key: l.uninstall_key.clone(),
        notes: l.notes.clone(),
        group: l.group.clone(),
        tags: l.tags.clone(),
        changes: ChangeSet::default(),
    }
}

impl App {
    pub fn delete_log(&mut self, id: &str) {
        match self.store.delete_log(id) {
            Ok(()) => {
                self.logs.selected = None;
                self.reload_logs();
                self.programs.needs_refresh = true;
            }
            Err(e) => self.set_status(e.to_string()),
        }
    }

    pub(super) fn uninstall_from_log(&mut self, log: &InstallLog) {
        let plan = build_plan(&log.changes, self.reg.as_ref());
        let mut flow = RemovalFlow::new(
            tf("Uninstall {name}", &[("name", log.name.clone())]),
            plan,
            true,
            self.settings.remove_unlogged_enabled(),
        );
        flow.logs_to_delete = vec![log.id.clone()];
        flow.names = vec![log.name.clone()];
        self.open_removal(flow);
    }

    /// Repackage a log as a silent deploy folder and one silent Setup.exe.
    fn start_silent_package(&mut self, log: InstallLog) {
        if self.logs.package_job.is_some() {
            return;
        }
        let Some(dir) = pick_folder(&t("Choose where to save the setup package")) else {
            return;
        };
        let reg = self.reg.clone();
        let root = self.store.root.clone();
        self.set_status(tf(
            "Creating a setup package for {name}…",
            &[("name", log.name.clone())],
        ));
        self.logs.package_job = Some(Job::spawn(&self.ctx, move |pr| {
            let s = installscope_core::repackage::repackage(&log, reg.as_ref(), &dir, &|p| {
                pr.set_phase(p)
            })
            .map_err(|e| e.to_string())?;
            if s.missing > 0 {
                return Ok((
                    s.folder,
                    Err(format!(
                        "{} captured file(s) are missing; no Setup.exe was created. Review the install log or make a fresh capture.",
                        s.missing
                    )),
                ));
            }
            let setup = installscope_core::patools::ensure(&root, &|p| pr.set_phase(p.to_string()))
                .and_then(|tc| {
                    installscope_core::patools::build_silent_setup(
                        &tc,
                        &s.folder,
                        &log.name,
                        &log.version,
                        &root.join("tools").join("work"),
                    )
                })
                .map_err(|e| e.to_string());
            Ok((s.folder, setup))
        }));
    }

    pub fn poll_silent_package(&mut self) {
        let Some(r) = self.logs.package_job.as_ref().and_then(|j| j.poll()) else {
            return;
        };
        self.logs.package_job = None;
        match r {
            Ok((_, Ok(setup))) => {
                self.set_status(tf(
                    "Setup package ready: {file}",
                    &[("file", setup.display().to_string())],
                ));
                installscope_core::platform::reveal(&setup.to_string_lossy());
            }
            Ok((folder, Err(e))) => {
                self.set_status(tf(
                    "The deploy folder is ready, but Setup.exe could not be created: {error}",
                    &[("error", e)],
                ));
                installscope_core::platform::shell_open(&folder.to_string_lossy());
            }
            Err(e) => self.set_status(e),
        }
    }

    fn print_log(&mut self, log: &InstallLog) {
        let path = std::env::temp_dir().join(format!(
            "InstallScope-{}-print.html",
            installscope_core::util::safe_file_name(&log.name)
        ));
        match std::fs::write(&path, report::to_print_html(log)) {
            Ok(()) => installscope_core::platform::shell_open(&path.to_string_lossy()),
            Err(e) => self.set_status(e.to_string()),
        }
    }

    fn merge_with(&mut self, a: &str, b: &str) {
        let la = self.logs.logs.iter().find(|l| l.id == a).cloned();
        let lb = self.logs.logs.iter().find(|l| l.id == b).cloned();
        let (Some(la), Some(lb)) = (la, lb) else {
            return;
        };
        // The list holds summaries; merge the full logs from disk.
        let full = |l: &InstallLog| self.store.load_log(&l.id).unwrap_or_else(|_| l.clone());
        let (fa, fb) = (full(&la), full(&lb));
        let merged = installscope_core::merge::merge_logs(&[&fa, &fb], "");
        match self.store.save_log(&merged) {
            Ok(()) => {
                self.set_status(tf(
                    "Merged into a new log: {name}",
                    &[("name", merged.name.clone())],
                ));
                self.logs.selected = Some(merged.id.clone());
                self.reload_logs();
            }
            Err(e) => self.set_status(e.to_string()),
        }
    }

    fn export_log(&mut self, log: &InstallLog, ext: &str) {
        let base = installscope_core::util::safe_file_name(&log.name);
        let (content, filter) = match ext {
            "html" => (report::to_html(log), "HTML"),
            "reg" => (report::to_reg_undo(log), "Registry script"),
            "ps1" => (report::to_ps1(log), "PowerShell"),
            "xml" => (report::to_xml(log), "XML"),
            "ini" => (
                installscope_core::portable::launcher_ini(
                    &log.changes,
                    &log.name,
                    &installscope_core::portable::Folders::current(),
                )
                .text,
                "PortableApps.com Launcher settings",
            ),
            _ => (
                installscope_core::json::compat::to_string_pretty(log).unwrap_or_default(),
                "InstallScope log",
            ),
        };
        let default = format!("{base}.{ext}");
        let Some(path) = save_file(&t("Export install log"), &default, filter, ext) else {
            return;
        };
        // .reg files are expected to be UTF-16 with a BOM.
        let res = if ext == "reg" {
            let mut bytes = vec![0xFF, 0xFE];
            for u in content.encode_utf16() {
                bytes.extend_from_slice(&u.to_le_bytes());
            }
            std::fs::write(&path, bytes)
        } else {
            std::fs::write(&path, content)
        };
        match res {
            Ok(()) => {
                self.set_status(tf("Saved {path}", &[("path", path.display().to_string())]));
                if ext == "html" {
                    installscope_core::platform::shell_open(&path.to_string_lossy());
                }
            }
            Err(e) => self.set_status(e.to_string()),
        }
    }

    pub fn ui_logs(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("logs_list")
            .frame(theme::side_frame().inner_margin(egui::Margin {
                left: 18,
                right: 8,
                top: 4,
                bottom: 12,
            }))
            .show_separator_line(false)
            .resizable(true)
            .default_width(300.0)
            .show(ctx, |ui| {
                let r = super::a11y::explain(
                    search_box(
                        ui,
                        &mut self.logs.filter,
                        "Search logs",
                        ui.available_width() - 30.0,
                    ),
                    &t("Search by log name, publisher, group or tag. The list below filters as you type."),
                );
                if self.focus_search {
                    r.request_focus();
                    self.focus_search = false;
                }
                let mut groups: Vec<String> = self
                    .logs
                    .logs
                    .iter()
                    .map(|l| l.group.clone())
                    .filter(|g| !g.is_empty())
                    .collect();
                groups.sort_by_key(|g| g.to_lowercase());
                groups.dedup();
                if !groups.is_empty() {
                    ui.add_space(4.0);
                    let combo = egui::ComboBox::from_id_salt("log_group_filter")
                        .selected_text(if self.logs.group_filter.is_empty() {
                            t("All groups")
                        } else {
                            self.logs.group_filter.clone()
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(
                                &mut self.logs.group_filter,
                                String::new(),
                                t("All groups"),
                            );
                            for g in &groups {
                                ui.selectable_value(&mut self.logs.group_filter, g.clone(), g);
                            }
                        });
                    super::a11y::explain(
                        combo.response,
                        &t("Show only the logs in one group, or all groups."),
                    );
                } else {
                    self.logs.group_filter.clear();
                }
                ui.add_space(6.0);
                let f = self.logs.filter.to_lowercase();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        if self.logs.logs.is_empty() && self.logs.loader.busy() {
                            ui.add_space(20.0);
                            job_progress(ui, &t("Loading…"), 0);
                        } else if self.logs.logs.is_empty() {
                            ui.add_space(20.0);
                            muted(
                                ui,
                                t("No monitored installations yet. Use Monitor to record one."),
                            );
                        }
                        // Keyboard navigation over the logs that match the filter.
                        let shown: Vec<usize> = self
                            .logs
                            .logs
                            .iter()
                            .enumerate()
                            .filter(|(_, l)| {
                                l.matches(&f)
                                    && (self.logs.group_filter.is_empty()
                                        || l.group == self.logs.group_filter)
                            })
                            .map(|(n, _)| n)
                            .collect();
                        // Grouped logs first, by group name; ungrouped last.
                        let mut shown = shown;
                        shown.sort_by_key(|&n| {
                            let g = &self.logs.logs[n].group;
                            (g.is_empty(), g.to_lowercase())
                        });
                        let cur =
                            self.logs.selected.as_ref().and_then(|id| {
                                shown.iter().position(|&n| &self.logs.logs[n].id == id)
                            });
                        let blocked = self.removal.is_some() || self.confirm.is_some();
                        let nav = super::a11y::list_nav(ui.ctx(), shown.len(), cur, blocked)
                            .map(|k| shown[k]);
                        if let Some(n) = nav {
                            self.logs.selected = Some(self.logs.logs[n].id.clone());
                            self.logs.editing = false;
                        }
                        let any_group = self.logs.logs.iter().any(|l| !l.group.is_empty());
                        let mut last_group: Option<&str> = None;
                        for &n in &shown {
                            let log = &self.logs.logs[n];
                            if any_group && last_group != Some(log.group.as_str()) {
                                last_group = Some(log.group.as_str());
                                ui.add_space(4.0);
                                let title = if log.group.is_empty() {
                                    t("Not in a group")
                                } else {
                                    log.group.clone()
                                };
                                ui.label(
                                    RichText::new(title.to_uppercase())
                                        .small()
                                        .color(theme::MUTED),
                                );
                            }
                            let sel = self.logs.selected.as_deref() == Some(&log.id);
                            let fill = if sel {
                                theme::NAV_SELECTED
                            } else {
                                theme::CARD
                            };
                            let resp = egui::Frame::new()
                                .fill(fill)
                                .corner_radius(10)
                                .inner_margin(egui::Margin::symmetric(12, 8))
                                .show(ui, |ui| {
                                    ui.set_width(ui.available_width());
                                    ui.label(RichText::new(&log.name).strong());
                                    if !log.tags.is_empty() {
                                        ui.horizontal_wrapped(|ui| {
                                            for tag in &log.tags {
                                                theme::badge(ui, tag, theme::MUTED);
                                            }
                                        });
                                    }
                                    small_muted(
                                        ui,
                                        format!(
                                            "{}  ·  {}",
                                            log.created,
                                            human_size(
                                                self.logs
                                                    .stats
                                                    .get(&log.id)
                                                    .map(|s| s.bytes_added)
                                                    .unwrap_or(0)
                                            )
                                        ),
                                    );
                                })
                                .response
                                .interact(egui::Sense::click());
                            let resp = super::a11y::explain(
                                resp,
                                &t("Open this install log to see what it recorded and what you can do with it."),
                            );
                            super::a11y::selectable(&resp, &log.name, sel);
                            if nav == Some(n) {
                                resp.scroll_to_me(None);
                            }
                            if resp.clicked() {
                                self.logs.selected = Some(log.id.clone());
                                self.logs.editing = false;
                            }
                            ui.add_space(4.0);
                        }
                    });
            });

        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            // The focused control's explanation line, for keyboard
            // and touch users (see a11y::explain).
            super::a11y::focus_hint(ui, theme::MUTED);
            theme::card(ui, |ui| {
                ui.label(RichText::new(t("Which program created this?")).strong());
                small_muted(ui, t("Type part of a file path or registry key to find the monitored program it belongs to."));
                super::a11y::explain(
                    search_box(ui, &mut self.logs.path_search, "e.g. AppData\\Roaming\\Acme or HKCU\\Software\\Acme", 420.0),
                    &t("Search every install log for part of a file path or registry key."),
                );
                let fp = self.logs.logs.as_ptr() as usize ^ self.logs.logs.len();
                let fresh = matches!(&self.logs.search_cache, Some((q, f, _)) if *q == self.logs.path_search && *f == fp);
                if !fresh {
                    let hits: Vec<(String, String, usize, String)> = report::search_logs(&self.logs.logs, &self.logs.path_search)
                        .into_iter()
                        .take(10)
                        .map(|(log, paths)| (log.id.clone(), log.name.clone(), paths.len(), paths[0].clone()))
                        .collect();
                    self.logs.search_cache = Some((self.logs.path_search.clone(), fp, hits));
                }
                let hits = self.logs.search_cache.as_ref().map(|c| c.2.clone()).unwrap_or_default();
                let mut jump = None;
                for (id, name, n, first) in &hits {
                    ui.horizontal(|ui| {
                        if super::a11y::explain(
                            ui.link(RichText::new(name).strong()),
                            &t("Open this install log."),
                        )
                        .clicked()
                        {
                            jump = Some(id.clone());
                        }
                        small_muted(ui, tf("{n} matches, e.g. {path}", &[("n", n.to_string()), ("path", first.clone())]));
                    });
                }
                if !self.logs.path_search.trim().is_empty() && hits.is_empty() {
                    small_muted(ui, t("No monitored program created anything matching that."));
                }
                if let Some(id) = jump {
                    self.logs.selected = Some(id);
                }
            });
            ui.add_space(10.0);

            let Some(idx) = self.logs.selected.as_ref().and_then(|id| self.logs.logs.iter().position(|l| &l.id == id)) else {
                ui.add_space(20.0);
                ui.vertical_centered(|ui| muted(ui, t("Select an install log on the left.")));
                return;
            };
            // Clone only the metadata; the change lists can be huge.
            let log = {
                let l = &self.logs.logs[idx];
                clone_meta(l)
            };
            let st = self.logs.stats.get(&self.logs.logs[idx].id).cloned().unwrap_or_default();
            theme::card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.heading(RichText::new(&log.name).strong());
                    if !log.version.is_empty() {
                        theme::badge(ui, &log.version, theme::MUTED);
                    }
                });
                small_muted(
                    ui,
                    format!(
                        "{}{}{}",
                        log.publisher,
                        if log.publisher.is_empty() { "" } else { "  ·  " },
                        tf("monitored {name}", &[("name", log.created.clone())])
                    ),
                );
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    theme::badge(ui, tf("{n} files", &[("n", st.files_added.to_string())]), theme::ADDED);
                    theme::badge(ui, tf("{n} folders", &[("n", st.dirs_added.to_string())]), theme::ADDED);
                    theme::badge(ui, tf("{n} keys", &[("n", st.keys_added.to_string())]), theme::ADDED);
                    theme::badge(ui, tf("{n} values", &[("n", st.values_added.to_string())]), theme::ADDED);
                    theme::badge(ui, tf("{n} changed", &[("n", (st.values_modified + st.files_modified).to_string())]), theme::CHANGED);
                    theme::badge(ui, human_size(st.bytes_added), theme::MUTED);
                });
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    if super::a11y::explain(
                        theme::primary_button(ui, format!("🗑 {}", t("Uninstall using this log"))),
                        &t("Uninstall this program using exactly what this log recorded. You review the removal first, and the log is deleted when the uninstall completes."),
                    )
                    .clicked()
                    {
                        let full = self.logs.logs[idx].clone();
                        self.uninstall_from_log(&full);
                    }
                    if super::a11y::explain(
                        ui.add_enabled(
                            self.logs.package_job.is_none(),
                            egui::Button::new(t("Create Setup Package…")),
                        ),
                        &t("Build a silent Setup.exe and deploy folder from this recorded installation."),
                    )
                    .clicked()
                    {
                        let id = self.logs.logs[idx].id.clone();
                        match self.store.load_log(&id) {
                            Ok(full) => self.start_silent_package(full),
                            Err(error) => self.set_status(tf(
                                "Could not load this install log: {error}",
                                &[("error", error.to_string())],
                            )),
                        }
                    }
                    let export_menu = ui.menu_button(t("Export"), |ui| {
                        for (ext, label) in [
                            ("html", "HTML report"),
                            ("reg", "Registry undo script (.reg)"),
                            ("ps1", "PowerShell removal script"),
                            ("json", "Install log (.json)"),
                            ("xml", "XML (.xml)"),
                            ("ini", "PortableApps.com Launcher settings (.ini)"),
                        ] {
                            if ui.button(t(label)).clicked() {
                                ui.close();
                                let full = self.logs.logs[idx].clone();
                                self.export_log(&full, ext);
                            }
                        }
                    });
                    super::a11y::explain(
                        export_menu.response,
                        &t("Save this log in another format, such as an HTML report or a removal script."),
                    );
                    if super::a11y::explain(
                        ui.button(format!("🖨 {}", t("Print"))),
                        &t("Open the report in your browser and print it."),
                    )
                    .clicked()
                    {
                        let full = self.logs.logs[idx].clone();
                        self.print_log(&full);
                    }
                    let others: Vec<(String, String)> = self
                        .logs
                        .logs
                        .iter()
                        .filter(|l| l.id != log.id)
                        .map(|l| (l.id.clone(), format!("{}  ({})", l.name, l.created)))
                        .collect();
                    if !others.is_empty() {
                        let merge_menu = ui.menu_button(t("Merge with"), |ui| {
                            small_muted(ui, t("Combine two logs into one, e.g. a program and its update. The originals are kept."));
                            egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                                for (id, label) in &others {
                                    if ui.button(label).clicked() {
                                        ui.close();
                                        self.merge_with(&log.id.clone(), id);
                                    }
                                }
                            });
                        });
                        super::a11y::explain(
                            merge_menu.response,
                            &t("Combine this log with another into a new log. The originals are kept."),
                        );
                    }
                    if super::a11y::explain(
                        ui.button(t("Edit details")),
                        &t("Edit this log's name, publisher, version, group, tags and notes."),
                    )
                    .clicked()
                    {
                        self.logs.editing = !self.logs.editing;
                        self.logs.tags_text = log.tags.join(", ");
                    }
                    if super::a11y::explain(
                        ui.button(t("Delete log")),
                        &t("Delete this install log. You confirm first, and the program itself is not touched."),
                    )
                    .clicked()
                    {
                        self.ask(
                            "Delete install log?",
                            tf("The record of {name} will be deleted. The program itself is not touched.", &[("name", log.name.clone())]),
                            "Delete",
                            Confirm::DeleteLog(log.id.clone()),
                        );
                    }
                });
                if !log.notes.is_empty() && !self.logs.editing {
                    ui.add_space(4.0);
                    small_muted(ui, &log.notes);
                }
            });
            if self.logs.editing {
                ui.add_space(8.0);
                let mut save = false;
                theme::card(ui, |ui| {
                    let l = &mut self.logs.logs[idx];
                    egui::Grid::new("edit_log").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        let name_label = ui.label(t("Name"));
                        super::a11y::explain(
                            ui.text_edit_singleline(&mut l.name).labelled_by(name_label.id),
                            &t("The name this install log is saved under. Editing it changes only the log, not the installed program."),
                        );
                        ui.end_row();
                        let publisher_label = ui.label(t("Publisher"));
                        super::a11y::explain(
                            ui.text_edit_singleline(&mut l.publisher).labelled_by(publisher_label.id),
                            &t("The publisher recorded in this install log. Editing it changes only the log, not the installed program."),
                        );
                        ui.end_row();
                        let version_label = ui.label(t("Version"));
                        super::a11y::explain(
                            ui.text_edit_singleline(&mut l.version).labelled_by(version_label.id),
                            &t("The version recorded in this install log. Editing it changes only the log, not the installed program."),
                        );
                        ui.end_row();
                        let group_label = ui.label(t("Group"));
                        super::a11y::explain(
                            ui.add(egui::TextEdit::singleline(&mut l.group).hint_text(t("e.g. Work tools"))).labelled_by(group_label.id),
                            &t("The group this log is filed under in the list. Groups only organize your logs."),
                        );
                        ui.end_row();
                        let tags_label = ui.label(t("Tags"));
                        super::a11y::explain(
                            ui.add(egui::TextEdit::singleline(&mut self.logs.tags_text).hint_text(t("Separate tags with commas"))).labelled_by(tags_label.id),
                            &t("Words for finding this log later, separated by commas. Tags only organize your logs."),
                        );
                        ui.end_row();
                        let notes_label = ui.label(t("Notes"));
                        super::a11y::explain(
                            ui.text_edit_multiline(&mut l.notes).labelled_by(notes_label.id),
                            &t("Your own notes about this installation, saved with the log."),
                        );
                        ui.end_row();
                    });
                    if super::a11y::explain(
                        theme::primary_button(ui, t("Save")),
                        &t("Save the changes to this log's details."),
                    )
                    .clicked()
                    {
                        save = true;
                    }
                });
                if save {
                    let tags = installscope_core::model::parse_tags(&self.logs.tags_text);
                    let group = self.logs.logs[idx].group.trim().to_string();
                    self.logs.logs[idx].tags = tags;
                    self.logs.logs[idx].group = group;
                    let l = self.logs.logs[idx].clone();
                    // Names and groups feed the cached program views.
                    self.programs.visible_cache = None;
                    self.programs.monitored_cache = None;
                    if let Err(e) = self.store.save_log(&l) {
                        self.set_status(e.to_string());
                    }
                    self.logs.editing = false;
                }
            }
            ui.add_space(8.0);
            let LogsState { logs, tree, show_list, .. } = &mut self.logs;
            let cs = &mut logs[idx].changes;
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                theme::card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(t("Where it put things")).strong());
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if super::a11y::explain(
                                ui.selectable_label(*show_list, t("List")),
                                &t("Show the changes as a plain list, grouped by the kind of change."),
                            )
                            .clicked()
                            {
                                *show_list = true;
                            }
                            if super::a11y::explain(
                                ui.selectable_label(!*show_list, t("Map")),
                                &t("Show the changes as a colored map, grouped by area of the PC."),
                            )
                            .clicked()
                            {
                                *show_list = false;
                            }
                        });
                    });
                    ui.add_space(4.0);
                    if *show_list {
                        changes_ui(ui, cs, false);
                    } else {
                        super::difftree_ui::diff_tree_ui(ui, tree, cs);
                    }
                });
            });
        });
    }
}
