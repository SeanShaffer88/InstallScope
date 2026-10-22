//! Startup page: Run entries, Startup folders, services, scheduled tasks.

use super::a11y;
use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use egui_extras::{Column, TableBuilder};
use setupscope_core::backup::BackupWriter;
use setupscope_core::i18n::{t, tf};
use setupscope_core::startup::{self, ServiceEntry, StartupEntry, TaskEntry};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Sub {
    #[default]
    Entries,
    Services,
    Tasks,
    /// Winlogon, Active Setup, IFEO, AppInit, Explorer extensions, WMI…
    Other,
}

type StartupLists = (Vec<StartupEntry>, Vec<ServiceEntry>, Vec<TaskEntry>);

#[derive(Default)]
pub struct StartupState {
    pub loaded: bool,
    pub sub: Sub,
    pub entries: Vec<StartupEntry>,
    pub services: Vec<ServiceEntry>,
    pub tasks: Vec<TaskEntry>,
    pub job: Option<Job<StartupLists>>,
    pub show_windows_services: bool,
    pub show_microsoft_tasks: bool,
    pub filter: String,
    /// Other autostart locations, with "target file is missing" flags.
    pub other: Option<Vec<(setupscope_core::autoruns::AutoEntry, bool)>>,
    pub other_job: Option<Job<Vec<(setupscope_core::autoruns::AutoEntry, bool)>>>,
    pub other_checked: std::collections::BTreeSet<usize>,
    pub show_windows_other: bool,
    /// Boot diagnostics (None inside = not readable) and delayed programs.
    pub boot: Option<Option<setupscope_core::boot::BootInfo>>,
    pub delayed: Vec<String>,
    pub boot_job: Option<Job<(Option<setupscope_core::boot::BootInfo>, Vec<String>)>>,
}

impl App {
    pub fn refresh_startup(&mut self, ctx: &egui::Context) {
        let reg = self.reg.clone();
        self.startup.loaded = true;
        self.startup.job = Some(Job::spawn(ctx, move |_| {
            (
                startup::list_startup(reg.as_ref()),
                startup::list_services(reg.as_ref()),
                startup::list_tasks(),
            )
        }));
        self.startup.boot_job = Some(Job::spawn(ctx, |_| {
            (
                setupscope_core::boot::read(),
                setupscope_core::boot::delayed(),
            )
        }));
    }

    pub fn poll_startup(&mut self) {
        if let Some((e, s, tk)) = self.startup.job.as_ref().and_then(|j| j.poll()) {
            self.startup.job = None;
            self.startup.entries = e;
            self.startup.services = s;
            self.startup.tasks = tk;
        }
        if let Some((b, d)) = self.startup.boot_job.as_ref().and_then(|j| j.poll()) {
            self.startup.boot_job = None;
            self.startup.boot = Some(b);
            self.startup.delayed = d;
        }
    }

    /// Run a small change with a backup, reporting the outcome in the status bar.
    fn with_backup(
        &mut self,
        title: &str,
        f: impl FnOnce(
            &dyn setupscope_core::platform::Registry,
            Option<&mut BackupWriter>,
        ) -> anyhow::Result<()>,
    ) {
        let mut w = if self.settings.backup_enabled() {
            match BackupWriter::new(&self.store.backups_dir(), title) {
                Ok(writer) => Some(writer),
                Err(error) => {
                    self.set_status(format!(
                        "✖ {title}: could not create a backup; no change was made: {error:#}"
                    ));
                    let ctx = self.ctx.clone();
                    self.refresh_startup(&ctx);
                    self.reload_backups();
                    return;
                }
            }
        } else {
            None
        };
        let mut r = f(self.reg.as_ref(), w.as_mut());
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
            Err(e) => self.set_status(format!("✖ {title}: {e:#}")),
        }
        let ctx = self.ctx.clone();
        self.refresh_startup(&ctx);
        self.reload_backups();
    }

    pub fn delete_startup(&mut self, i: usize) {
        let Some(e) = self.startup.entries.get(i).cloned() else {
            return;
        };
        self.with_backup(
            &tf("Delete startup entry {name}", &[("name", e.name.clone())]),
            |reg, b| startup::delete_entry(reg, &e, b),
        );
    }

    pub fn delete_task(&mut self, i: usize) {
        let Some(task) = self.startup.tasks.get(i).cloned() else {
            return;
        };
        self.with_backup(
            &tf(
                "Delete scheduled task {name}",
                &[("name", task.name.clone())],
            ),
            |_, b| startup::delete_task(&task, b),
        );
    }

    pub fn ui_startup(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(theme::content_frame())
            .show(ctx, |ui| {
                if !self.settings.advanced_mode {
                    self.startup.sub = Sub::Entries;
                }
                // The focused control's explanation line, for keyboard
                // and touch users (see a11y::explain).
                a11y::focus_hint(ui, theme::MUTED);
                ui.horizontal(|ui| {
                    if self.settings.advanced_mode {
                        for (s, label, why) in [
                            (
                                Sub::Entries,
                                t("Startup programs"),
                                t("Programs that start with Windows, in the Run registry keys and the Startup folders."),
                            ),
                            (
                                Sub::Services,
                                t("Services"),
                                t("Background services and when each one starts."),
                            ),
                            (
                                Sub::Tasks,
                                t("Scheduled tasks"),
                                t("Tasks Windows runs on a schedule or at sign-in."),
                            ),
                            (
                                Sub::Other,
                                t("Other autostarts"),
                                t("Other places programs use to start or load automatically, such as Winlogon and Explorer extensions."),
                            ),
                        ] {
                            if a11y::explain(ui.selectable_label(self.startup.sub == s, label), &why)
                                .clicked()
                            {
                                self.startup.sub = s;
                            }
                        }
                        ui.separator();
                    }
                    a11y::explain(
                        search_box(ui, &mut self.startup.filter, "Filter", 200.0),
                        &t("Filter the list below by name or command. The list updates as you type."),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if a11y::explain(
                            ui.button(format!("⟳ {}", t("Refresh"))),
                            &t("Reload the startup lists from Windows."),
                        )
                        .clicked()
                        {
                            self.refresh_startup(ctx);
                            self.startup.other = None;
                        }
                    });
                });
                ui.add_space(6.0);
                if self.startup.job.is_some() {
                    job_progress(ui, &t("Loading…"), 0);
                    return;
                }
                match self.startup.sub {
                    Sub::Entries => self.ui_startup_entries(ui),
                    Sub::Services => self.ui_services(ui),
                    Sub::Tasks => self.ui_tasks(ui),
                    Sub::Other => self.ui_other_autostarts(ui),
                }
            });
    }

    fn ui_other_autostarts(&mut self, ui: &mut egui::Ui) {
        if let Some(r) = self.startup.other_job.as_ref().and_then(|j| j.poll()) {
            self.startup.other_job = None;
            self.startup.other = Some(r);
            self.startup.other_checked.clear();
        }
        if self.startup.other.is_none() && self.startup.other_job.is_none() {
            let reg = self.reg.clone();
            self.startup.other_job = Some(Job::spawn(ui.ctx(), move |_| {
                let ix = setupscope_core::leftovers::DeepIndex::build(reg.as_ref(), None);
                let mut list = setupscope_core::autoruns::scan(reg.as_ref(), &ix);
                list.extend(setupscope_core::autoruns::wmi_consumers());
                list.into_iter()
                    .map(|e| {
                        let missing = setupscope_core::orphans::target_path(&e.target)
                            .is_some_and(|p| !std::path::Path::new(&p).exists());
                        (e, missing)
                    })
                    .collect()
            }));
        }
        let Some(list) = self.startup.other.clone() else {
            job_progress(ui, &t("Loading…"), 0);
            return;
        };
        ui.horizontal(|ui| {
            a11y::explain(
                ui.checkbox(
                    &mut self.startup.show_windows_other,
                    t("Show Windows' own entries"),
                ),
                &t("Also list the autostart entries that are part of Windows itself, not just ones added by programs."),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let n = self.startup.other_checked.len();
                if n > 0
                    && a11y::explain(
                        theme::primary_button(ui, tf("Remove {n} selected", &[("n", n.to_string())])),
                        &t("Open a review of the ticked entries. Nothing is removed until you confirm there; removals are backed up first unless backups are off in Settings."),
                    )
                    .clicked()
                {
                    let items: Vec<setupscope_core::revert::PlanItem> = self
                        .startup
                        .other_checked
                        .iter()
                        .filter_map(|&i| list.get(i))
                        .filter_map(|(e, _)| e.removal.clone().map(|a| (a, e.kind)))
                        .map(|(action, kind)| setupscope_core::revert::PlanItem {
                            action,
                            selected: true,
                            exists: true,
                            note: kind.into(),
                        })
                        .collect();
                    let plan = setupscope_core::revert::Plan {
                        items,
                        ..Default::default()
                    };
                    self.open_removal(super::plan::RemovalFlow::new(
                        t("Remove autostart entries"),
                        plan,
                        false,
                        false,
                    ));
                    self.startup.other = None;
                }
            });
        });
        small_muted(ui, t("Places programs use to start or load automatically, beyond the usual startup list: Winlogon, Active Setup, program replacements, AppInit DLLs, Explorer extensions, icon overlays, browser helpers, boot programs and WMI subscriptions."));
        ui.add_space(4.0);
        let f = self.startup.filter.to_lowercase();
        let rows: Vec<usize> = (0..list.len())
            .filter(|&i| {
                let (e, _) = &list[i];
                (self.startup.show_windows_other || !e.windows)
                    && (f.is_empty()
                        || e.name.to_lowercase().contains(&f)
                        || e.target.to_lowercase().contains(&f)
                        || e.kind.to_lowercase().contains(&f))
            })
            .collect();
        if rows.is_empty() {
            ui.add_space(10.0);
            muted(ui, t("Nothing found."));
            return;
        }
        TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(22.0))
            .column(
                Column::initial(170.0)
                    .at_least(90.0)
                    .clip(true)
                    .resizable(true),
            )
            .column(
                Column::initial(170.0)
                    .at_least(90.0)
                    .clip(true)
                    .resizable(true),
            )
            .column(Column::remainder().at_least(150.0).clip(true))
            .header(24.0, |mut h| {
                h.col(|_| {});
                for l in ["Kind", "Name", "Runs or loads"] {
                    h.col(|ui| {
                        ui.strong(t(l));
                    });
                }
            })
            .body(|body| {
                body.rows(26.0, rows.len(), |mut row| {
                    let i = rows[row.index()];
                    let (e, missing) = &list[i];
                    row.col(|ui| {
                        if e.removal.is_some() {
                            let mut c = self.startup.other_checked.contains(&i);
                            let name = e.name.clone();
                            let r = ui.checkbox(&mut c, "");
                            r.widget_info(|| {
                                egui::WidgetInfo::selected(
                                    egui::WidgetType::Checkbox,
                                    true,
                                    c,
                                    &name,
                                )
                            });
                            let r = a11y::explain(
                                r,
                                &tf(
                                    "Tick to mark {name} for removal. Remove selected above opens a review before anything is removed.",
                                    &[("name", name)],
                                ),
                            );
                            if r.changed() {
                                if c {
                                    self.startup.other_checked.insert(i);
                                } else {
                                    self.startup.other_checked.remove(&i);
                                }
                            }
                        }
                    });
                    row.col(|ui| {
                        ui.add(egui::Label::new(t(e.kind)).truncate());
                    });
                    row.col(|ui| {
                        if *missing {
                            a11y::explain(
                                ui.label(RichText::new("⚠").color(theme::WARN)),
                                &t("The file it runs no longer exists"),
                            );
                        }
                        ui.add(egui::Label::new(&e.name).truncate());
                    });
                    row.col(|ui| {
                        a11y::explain(
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&e.target).monospace().color(theme::MUTED),
                                )
                                .truncate(),
                            ),
                            &format!("{}\n{}", e.target, e.location),
                        );
                    });
                });
            });
    }

    fn ui_startup_entries(&mut self, ui: &mut egui::Ui) {
        small_muted(ui, t("Disabling works like Task Manager: the entry stays but doesn't run. Deleting removes it (backed up first, unless backups are off in Settings)."));
        ui.add_space(4.0);
        let f = self.startup.filter.to_lowercase();
        let rows: Vec<usize> = (0..self.startup.entries.len())
            .filter(|&i| {
                let e = &self.startup.entries[i];
                f.is_empty()
                    || e.name.to_lowercase().contains(&f)
                    || e.command.to_lowercase().contains(&f)
            })
            .collect();
        match &self.startup.boot {
            Some(Some(b)) if b.last_boot_ms.is_some() => {
                let secs = |ms: u64| format!("{:.0}", ms as f64 / 1000.0);
                small_muted(ui, tf(
                    "The last Windows startup took {last} s (average {avg} s). Impact shows how much each program slowed it down, measured by Windows.",
                    &[("last", secs(b.last_boot_ms.unwrap_or(0))), ("avg", secs(b.avg_boot_ms.unwrap_or(0)))],
                ));
            }
            Some(_) if !self.elevated => {
                small_muted(
                    ui,
                    t("Run as Administrator to see how much each program slows down startup."),
                );
            }
            _ => {}
        }
        ui.add_space(4.0);
        let advanced = self.settings.advanced_mode;
        let mut toggle: Option<(usize, bool)> = None;
        let mut delete: Option<usize> = None;
        let mut delay_op: Option<(usize, bool)> = None;
        let boot = self.startup.boot.clone().flatten();
        let delayed = self.startup.delayed.clone();
        TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(70.0))
            .column(
                Column::initial(200.0)
                    .at_least(100.0)
                    .clip(true)
                    .resizable(true),
            )
            .column(Column::remainder().at_least(200.0).clip(true))
            .column(Column::initial(80.0))
            .column(Column::initial(110.0))
            .column(Column::exact(170.0))
            .header(24.0, |mut h| {
                for l in ["Enabled", "Name", "Command", "Impact", "Scope", ""] {
                    h.col(|ui| {
                        ui.strong(t(l));
                    });
                }
            })
            .body(|body| {
                body.rows(26.0, rows.len(), |mut row| {
                    let i = rows[row.index()];
                    let e = &self.startup.entries[i];
                    let locked = !advanced && startup::is_essential(e);
                    row.col(|ui| {
                        let mut on = e.enabled;
                        let why = if locked {
                            t("Needed by Windows, your security software or your hardware. Advanced mode can change it.")
                        } else if e.approved.is_none() {
                            t("This entry can only be deleted, not disabled: Windows keeps no on/off switch for it.")
                        } else {
                            tf(
                                "Start {name} with Windows. Unticking stops it starting but keeps the entry, like Task Manager, so you can tick it again to undo.",
                                &[("name", e.name.clone())],
                            )
                        };
                        let resp = a11y::explain(
                            ui.add_enabled(
                                e.approved.is_some() && !locked,
                                egui::Checkbox::new(&mut on, ""),
                            ),
                            &why,
                        );
                        let name = e.name.clone();
                        resp.widget_info(|| {
                            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, &name)
                        });
                        if resp.changed() {
                            toggle = Some((i, on));
                        }
                    });
                    row.col(|ui| {
                        if e.missing {
                            a11y::explain(
                                ui.label(RichText::new("⚠").color(theme::WARN)),
                                &t("The program this entry starts no longer exists"),
                            );
                        }
                        ui.add(egui::Label::new(&e.name).truncate());
                        if delayed.iter().any(|d| d == &e.name) {
                            theme::badge(ui, t("Delayed"), theme::OK);
                        }
                    });
                    row.col(|ui| {
                        a11y::explain(
                            ui.add(
                                egui::Label::new(
                                    RichText::new(&e.command).color(theme::MUTED).monospace(),
                                )
                                .truncate(),
                            ),
                            &e.location,
                        );
                    });
                    row.col(|ui| match boot.as_ref().and_then(|b| b.impact_of(&e.command)) {
                        Some(imp) => {
                            let color = match imp.level() {
                                setupscope_core::boot::Level::High => theme::BAD,
                                setupscope_core::boot::Level::Medium => theme::WARN,
                                setupscope_core::boot::Level::Low => theme::MUTED,
                            };
                            a11y::explain(
                                ui.label(RichText::new(t(imp.level().label())).color(color)),
                                &tf(
                                    "Slowed startup by {ms} ms on average ({n} recorded startups).",
                                    &[("ms", imp.avg_ms.to_string()), ("n", imp.boots.to_string())],
                                ),
                            );
                        }
                        None => {
                            a11y::explain(
                                ui.label(RichText::new("—").color(theme::MUTED)),
                                &t("Windows hasn't recorded this program slowing down startup."),
                            );
                        }
                    });
                    row.col(|ui| {
                        ui.label(if e.all_users {
                            t("All users")
                        } else {
                            t("Current user")
                        });
                    });
                    row.col(|ui| {
                        let is_delayed = delayed.iter().any(|d| d == &e.name);
                        if is_delayed {
                            if a11y::explain(
                                ui.small_button(t("Start normally")),
                                &t("Undo the delayed start: remove the delayed task and start this program with Windows again."),
                            )
                            .clicked()
                            {
                                delay_op = Some((i, false));
                            }
                        } else if e.enabled && !e.all_users && e.approved.is_some() && !locked && !e.missing
                            && a11y::explain(
                                ui.small_button(t("Delay")),
                                &t("Start this program 2 minutes after you sign in, so Windows is ready sooner."),
                            )
                            .clicked()
                        {
                            delay_op = Some((i, true));
                        }
                        // Simple mode only deletes broken entries; others can be disabled instead.
                        if (advanced || (e.missing && !locked))
                            && a11y::explain(
                                ui.small_button(t("Delete")),
                                &t("Ask to delete this entry so the program no longer starts with Windows. You confirm first, and a backup is kept unless backups are off in Settings."),
                            )
                            .clicked()
                        {
                            delete = Some(i);
                        }
                    });
                });
            });
        if let Some((i, on)) = toggle {
            let e = self.startup.entries[i].clone();
            let title = if on {
                tf("Enable {name}", &[("name", e.name.clone())])
            } else {
                tf("Disable {name}", &[("name", e.name.clone())])
            };
            self.with_backup(&title, |reg, b| startup::set_enabled(reg, &e, on, b));
        }
        if let Some((i, on)) = delay_op {
            let e = self.startup.entries[i].clone();
            if on {
                let title = tf("Delay {name}", &[("name", e.name.clone())]);
                self.with_backup(&title, |reg, b| {
                    setupscope_core::boot::delay(reg, &e, 120, b)
                });
            } else {
                let title = tf("Start {name} normally", &[("name", e.name.clone())]);
                self.with_backup(&title, |reg, _| setupscope_core::boot::undelay(reg, &e));
            }
        }
        if let Some(i) = delete {
            let name = self.startup.entries[i].name.clone();
            self.ask(
                "Delete startup entry?",
                tf(
                    "{name} will no longer start with Windows. A backup is kept.",
                    &[("name", name)],
                ),
                "Delete",
                Confirm::DeleteStartup(i),
            );
        }
    }

    fn ui_services(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            a11y::explain(
                ui.checkbox(
                    &mut self.startup.show_windows_services,
                    t("Show Windows services"),
                ),
                &t("Also list the services that are part of Windows itself, not just ones added by programs."),
            );
            small_muted(ui, t("Start type changes apply at the next restart."));
        });
        ui.add_space(4.0);
        let f = self.startup.filter.to_lowercase();
        let rows: Vec<usize> = (0..self.startup.services.len())
            .filter(|&i| {
                let s = &self.startup.services[i];
                (self.startup.show_windows_services || !s.windows)
                    && !s.is_driver
                    && (f.is_empty()
                        || s.display_name.to_lowercase().contains(&f)
                        || s.name.to_lowercase().contains(&f))
            })
            .collect();
        let mut change: Option<(usize, u32, bool)> = None;
        TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(
                Column::initial(260.0)
                    .at_least(120.0)
                    .clip(true)
                    .resizable(true),
            )
            .column(Column::initial(160.0).clip(true).resizable(true))
            .column(Column::remainder().at_least(160.0).clip(true))
            .header(24.0, |mut h| {
                for l in ["Service", "Start type", "Program"] {
                    h.col(|ui| {
                        ui.strong(t(l));
                    });
                }
            })
            .body(|body| {
                body.rows(26.0, rows.len(), |mut row| {
                    let i = rows[row.index()];
                    let s = &self.startup.services[i];
                    row.col(|ui| {
                        a11y::explain(
                            ui.add(egui::Label::new(&s.display_name).truncate()),
                            &s.name,
                        );
                    });
                    row.col(|ui| {
                        let label = |k: &str| {
                            t(match k {
                                "automatic" => "Automatic",
                                "auto_delayed" => "Automatic (delayed)",
                                "manual" => "Manual",
                                "disabled" => "Disabled",
                                "boot" => "Boot",
                                "system" => "System",
                                _ => "Unknown",
                            })
                        };
                        let combo = egui::ComboBox::from_id_salt(("svc", i))
                            .width(150.0)
                            .selected_text(label(s.start_label()))
                            .show_ui(ui, |ui| {
                                for (start, delayed, key, why) in [
                                    (2, false, "automatic", t("Start automatically when Windows starts.")),
                                    (2, true, "auto_delayed", t("Start automatically, shortly after Windows has finished starting.")),
                                    (3, false, "manual", t("Start only when Windows or a program asks for it.")),
                                    (4, false, "disabled", t("Never start until the start type is changed back.")),
                                ] {
                                    if a11y::explain(
                                        ui.selectable_label(s.start_label() == key, label(key)),
                                        &why,
                                    )
                                    .clicked()
                                    {
                                        change = Some((i, start, delayed));
                                    }
                                }
                            });
                        a11y::explain(
                            combo.response,
                            &t("When this service starts. The change is backed up first and applies at the next restart."),
                        );
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&s.image_path).color(theme::MUTED).monospace(),
                            )
                            .truncate(),
                        );
                    });
                });
            });
        if let Some((i, start, delayed)) = change {
            let name = self.startup.services[i].name.clone();
            self.with_backup(
                &tf("Change start type of {name}", &[("name", name.clone())]),
                |reg, b| startup::set_service_start(reg, &name, start, delayed, b),
            );
        }
    }

    fn ui_tasks(&mut self, ui: &mut egui::Ui) {
        a11y::explain(
            ui.checkbox(
                &mut self.startup.show_microsoft_tasks,
                t("Show Microsoft tasks"),
            ),
            &t("Also list the scheduled tasks that are part of Windows itself, not just ones added by programs."),
        );
        ui.add_space(4.0);
        let f = self.startup.filter.to_lowercase();
        let rows: Vec<usize> = (0..self.startup.tasks.len())
            .filter(|&i| {
                let tk = &self.startup.tasks[i];
                (self.startup.show_microsoft_tasks || !tk.microsoft)
                    && (f.is_empty()
                        || tk.name.to_lowercase().contains(&f)
                        || tk.command.to_lowercase().contains(&f))
            })
            .collect();
        if rows.is_empty() && !self.elevated {
            theme::warning_banner(
                ui,
                t("Scheduled tasks can only be read when SetupScope runs as Administrator."),
            );
        }
        let mut toggle: Option<(usize, bool)> = None;
        let mut delete: Option<usize> = None;
        TableBuilder::new(ui)
            .striped(true)
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::exact(70.0))
            .column(
                Column::initial(260.0)
                    .at_least(120.0)
                    .clip(true)
                    .resizable(true),
            )
            .column(Column::remainder().at_least(160.0).clip(true))
            .column(Column::exact(80.0))
            .header(24.0, |mut h| {
                for l in ["Enabled", "Task", "Runs", ""] {
                    h.col(|ui| {
                        ui.strong(t(l));
                    });
                }
            })
            .body(|body| {
                body.rows(26.0, rows.len(), |mut row| {
                    let i = rows[row.index()];
                    let tk = &self.startup.tasks[i];
                    row.col(|ui| {
                        let mut on = tk.enabled;
                        let resp = a11y::explain(
                            ui.checkbox(&mut on, ""),
                            &tf(
                                "Let {name} run on its schedule. Unticking turns the task off without deleting it, so you can tick it again to undo.",
                                &[("name", tk.name.clone())],
                            ),
                        );
                        let name = tk.name.clone();
                        resp.widget_info(|| {
                            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, true, on, &name)
                        });
                        if resp.changed() {
                            toggle = Some((i, on));
                        }
                    });
                    row.col(|ui| {
                        a11y::explain(
                            ui.add(egui::Label::new(&tk.name).truncate()),
                            &tk.author,
                        );
                    });
                    row.col(|ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&tk.command).color(theme::MUTED).monospace(),
                            )
                            .truncate(),
                        );
                    });
                    row.col(|ui| {
                        if a11y::explain(
                            ui.small_button(t("Delete")),
                            &t("Ask to delete this scheduled task. You confirm first, and a backup is kept unless backups are off in Settings."),
                        )
                        .clicked()
                        {
                            delete = Some(i);
                        }
                    });
                });
            });
        if let Some((i, on)) = toggle {
            let name = self.startup.tasks[i].name.clone();
            match startup::set_task_enabled(&name, on) {
                Ok(()) => self.startup.tasks[i].enabled = on,
                Err(e) => self.set_status(format!("✖ {e:#}")),
            }
        }
        if let Some(i) = delete {
            let name = self.startup.tasks[i].name.clone();
            self.ask(
                "Delete scheduled task?",
                tf(
                    "{name} will be deleted. A backup is kept.",
                    &[("name", name)],
                ),
                "Delete",
                Confirm::DeleteTask(i),
            );
        }
    }
}
