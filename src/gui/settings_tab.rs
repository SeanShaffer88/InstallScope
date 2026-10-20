//! Settings page.

use super::app::{App, Confirm};
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::i18n::{self, t, tf};
use installscope_core::noise::{self, Learned};
use installscope_core::settings::{FileRoot, Settings};

/// Keyboard shortcuts shown in Settings (keys, description).
const SHORTCUTS: &[(&str, &str)] = &[
    ("Tab / Shift+Tab", "Move between buttons, boxes and lists"),
    (
        "Enter / Space",
        "Press the focused button or tick the focused box",
    ),
    (
        "↑ ↓ Home End PgUp PgDn",
        "Move through the program or install-log list",
    ),
    (
        "Space",
        "Tick the selected program for uninstalling several at once",
    ),
    ("Delete", "Uninstall the selected program"),
    ("Esc", "Close a dialog or window, or leave a text box"),
    ("Ctrl+1 … Ctrl+9", "Go to a page"),
    ("Ctrl+Tab / Ctrl+Shift+Tab", "Next / previous page"),
    ("Ctrl+F", "Search"),
    ("Ctrl+K", "Ask InstallScope (plain-language search)"),
    ("F5", "Refresh"),
    ("F10", "Go to the menu bar (then arrow keys)"),
    ("F1", "Help for this page"),
    ("Ctrl + / Ctrl − / Ctrl+0", "Bigger / smaller / normal text"),
];

/// Seconds between the two snapshots of the background-noise check.
const LEARN_SECONDS: u64 = 60;

/// Suggestions from the background-noise check, each with a checkbox.
pub struct LearnResult {
    pub registry: Vec<(String, bool)>,
    pub files: Vec<(String, bool)>,
}

/// Text-editable copies of the list settings.
#[derive(Default)]
pub struct SettingsState {
    pub reg_roots: String,
    pub file_roots_rec: String,
    pub file_roots_top: String,
    pub reg_excl: String,
    pub file_excl: String,
    pub dirty: bool,
    pub learn_job: Option<Job<Result<Learned, String>>>,
    pub learned: Option<LearnResult>,
    /// Whether the right-click entry exists (all users, current user);
    /// read from the registry when Settings opens, not every frame.
    pub shell_registered: Option<(bool, bool)>,
    /// Name typed for "Save as profile".
    pub profile_name: String,
}

impl SettingsState {
    pub fn load(&mut self, s: &Settings) {
        self.shell_registered = None;
        self.reg_roots = s.registry_roots.join("\n");
        self.file_roots_rec = s
            .file_roots
            .iter()
            .filter(|r| r.recursive)
            .map(|r| r.path.clone())
            .collect::<Vec<_>>()
            .join("\n");
        self.file_roots_top = s
            .file_roots
            .iter()
            .filter(|r| !r.recursive)
            .map(|r| r.path.clone())
            .collect::<Vec<_>>()
            .join("\n");
        self.reg_excl = s.registry_exclusions.join("\n");
        self.file_excl = s.file_exclusions.join("\n");
        self.dirty = false;
    }

    fn apply(&self, s: &mut Settings) {
        let lines = |t: &str| {
            t.lines()
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
        };
        s.registry_roots = lines(&self.reg_roots);
        s.file_roots = lines(&self.file_roots_rec)
            .into_iter()
            .map(|p| FileRoot {
                path: p,
                recursive: true,
            })
            .chain(lines(&self.file_roots_top).into_iter().map(|p| FileRoot {
                path: p,
                recursive: false,
            }))
            .collect();
        s.registry_exclusions = lines(&self.reg_excl);
        s.file_exclusions = lines(&self.file_excl);
    }
}

fn list_editor(ui: &mut egui::Ui, label: &str, help: &str, text: &mut String, dirty: &mut bool) {
    ui.label(RichText::new(t(label)).strong());
    small_muted(ui, t(help));
    let r = ui.add(
        egui::TextEdit::multiline(text)
            .code_editor()
            .desired_rows(5)
            .desired_width(f32::INFINITY),
    );
    *dirty |= r.changed();
    ui.add_space(8.0);
}

impl App {
    fn start_learning(&mut self, ctx: &egui::Context) {
        let reg = self.reg.clone();
        let settings = self.settings.clone();
        self.settings_ui.learned = None;
        self.settings_ui.learn_job = Some(Job::spawn(ctx, move |pr| {
            use installscope_core::snapshot::{diff, take_snapshot};
            pr.set_phase(t("Taking the first snapshot…"));
            let before = take_snapshot(reg.as_ref(), &settings, pr).map_err(|e| e.to_string())?;
            for left in (1..=LEARN_SECONDS).rev() {
                if pr.cancelled() {
                    return Err(t("Cancelled"));
                }
                pr.set_phase(tf(
                    "Watching for background changes… {n} s",
                    &[("n", left.to_string())],
                ));
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            pr.set_phase(t("Taking the second snapshot…"));
            let after = take_snapshot(reg.as_ref(), &settings, pr).map_err(|e| e.to_string())?;
            let changes = diff(&before, &after);
            let roots: Vec<String> = settings.file_roots.iter().map(|r| r.path.clone()).collect();
            Ok(noise::learn(
                &changes,
                &roots,
                &settings.registry_roots,
                &settings.effective_file_exclusions(),
                &settings.effective_registry_exclusions(),
            ))
        }));
    }

    pub fn poll_learning(&mut self) {
        let Some(r) = self.settings_ui.learn_job.as_ref().and_then(|j| j.poll()) else {
            return;
        };
        self.settings_ui.learn_job = None;
        match r {
            Ok(l) if l.is_empty() => {
                self.set_status(t("No background noise found: nothing changed on its own."))
            }
            Ok(l) => {
                self.settings_ui.learned = Some(LearnResult {
                    registry: l.registry.into_iter().map(|p| (p, true)).collect(),
                    files: l.files.into_iter().map(|p| (p, true)).collect(),
                })
            }
            Err(e) => self.set_status(e),
        }
    }

    fn apply_learned(&mut self) {
        let Some(res) = self.settings_ui.learned.take() else {
            return;
        };
        let pick = |v: &[(String, bool)]| {
            v.iter()
                .filter(|(_, on)| *on)
                .map(|(p, _)| p.clone())
                .collect::<Vec<_>>()
        };
        let (regs, files) = (pick(&res.registry), pick(&res.files));
        let date = installscope_core::time::DateTime::now_local().ymd();
        noise::append_learned(&mut self.settings.registry_exclusions, &regs, &date);
        noise::append_learned(&mut self.settings.file_exclusions, &files, &date);
        self.save_settings();
        self.settings_ui.load(&self.settings);
        self.set_status(tf(
            "Added {n} exclusions.",
            &[("n", (regs.len() + files.len()).to_string())],
        ));
    }

    /// Automatic exclusions and the background-noise check (Advanced mode).
    fn ui_noise(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        if ui
            .checkbox(&mut self.settings.auto_exclusions, t("Automatically ignore temporary files, logs and caches (recommended)"))
            .on_hover_text(t("Uses InstallScope's built-in list of volatile locations in addition to yours. It is updated with InstallScope."))
            .changed()
        {
            self.save_settings();
        }
        egui::CollapsingHeader::new(t("Show the built-in list"))
            .id_salt("builtin_excl")
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for p in noise::FILE_PATTERNS.iter().chain(noise::REGISTRY_PATTERNS) {
                            ui.label(RichText::new(*p).monospace().small().color(theme::MUTED));
                        }
                    });
            });
        ui.add_space(6.0);
        if let Some(job) = &self.settings_ui.learn_job {
            ui.horizontal(|ui| {
                job_progress(ui, &job.progress.phase(), job.progress.count());
                if ui.small_button(t("Cancel")).clicked() {
                    job.progress
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
            });
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        } else if let Some(res) = &mut self.settings_ui.learned {
            ui.label(
                RichText::new(t(
                    "These changed on their own while nothing was being installed:",
                ))
                .strong(),
            );
            egui::ScrollArea::vertical()
                .id_salt("learned")
                .max_height(220.0)
                .show(ui, |ui| {
                    for (p, on) in res.files.iter_mut().chain(res.registry.iter_mut()) {
                        ui.checkbox(on, RichText::new(p.as_str()).monospace().small());
                    }
                });
            let mut apply = false;
            let mut discard = false;
            ui.horizontal(|ui| {
                apply = theme::primary_button(ui, t("Ignore selected")).clicked();
                discard = ui.button(t("Discard")).clicked();
            });
            if apply {
                self.apply_learned();
            } else if discard {
                self.settings_ui.learned = None;
            }
        } else {
            ui.horizontal(|ui| {
                if ui.button(format!("🔎 {}", t("Learn background noise"))).clicked() {
                    self.start_learning(ctx);
                }
                small_muted(ui, t("Takes two snapshots a minute apart. Don't install or change anything meanwhile."));
            });
        }
    }

    /// Choose the monitoring profile (and, in Settings, save or delete one).
    pub(super) fn profile_picker(&mut self, ui: &mut egui::Ui, manage: bool) {
        if !manage && self.settings.profiles.is_empty() {
            return;
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(t("Profile"));
            let current = if self.settings.active_profile.is_empty() {
                t("Custom scope")
            } else {
                self.settings.active_profile.clone()
            };
            let mut chosen: Option<String> = None;
            egui::ComboBox::from_id_salt(("scan_profile", manage))
                .selected_text(current)
                .show_ui(ui, |ui| {
                    for p in &self.settings.profiles {
                        if ui
                            .selectable_label(
                                self.settings.active_profile.eq_ignore_ascii_case(&p.name),
                                &p.name,
                            )
                            .clicked()
                        {
                            chosen = Some(p.name.clone());
                        }
                    }
                    if self.settings.profiles.is_empty() {
                        ui.label(t("No saved profiles yet"));
                    }
                });
            if let Some(name) = chosen {
                self.settings.use_profile(&name);
                self.settings_ui.load(&self.settings);
                self.save_settings();
                self.set_status(tf("Using profile {name}", &[("name", name)]));
            }
            if manage {
                ui.add_space(8.0);
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings_ui.profile_name)
                        .hint_text(t("Profile name"))
                        .desired_width(160.0),
                );
                let name = self.settings_ui.profile_name.trim().to_string();
                if ui
                    .add_enabled(!name.is_empty(), egui::Button::new(t("Save as profile")))
                    .on_hover_text(t(
                        "Save the scope below (including unsaved edits) under this name.",
                    ))
                    .clicked()
                {
                    self.settings_ui.apply(&mut self.settings);
                    self.settings_ui.dirty = false;
                    self.settings.save_profile(&name);
                    self.settings_ui.profile_name.clear();
                    self.save_settings();
                    self.set_status(tf("Saved profile {name}", &[("name", name)]));
                }
                let active = self.settings.active_profile.clone();
                if !active.is_empty() && ui.button(t("Delete profile")).clicked() {
                    self.settings.delete_profile(&active);
                    self.save_settings();
                }
            }
        });
        if manage {
            small_muted(ui, t("Profiles are saved scopes, e.g. a quick one for Program Files only and a deep one for the whole system. Choose one here or on the Monitor and Snapshots pages."));
        }
    }

    pub fn ui_settings(&mut self, ctx: &egui::Context) {
        if self.settings_ui.reg_roots.is_empty() && !self.settings_ui.dirty {
            self.settings_ui.load(&self.settings);
        }
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let mut changed = false;
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("General")).heading());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(t("Language"));
                        let current = self
                            .languages
                            .iter()
                            .find(|(c, _)| *c == self.settings.language)
                            .map(|(_, n)| n.clone())
                            .unwrap_or_else(|| t("Windows default"));
                        egui::ComboBox::from_id_salt("lang").selected_text(current).show_ui(ui, |ui| {
                            changed |= ui.selectable_value(&mut self.settings.language, String::new(), t("Windows default")).changed();
                            for (code, name) in &self.languages {
                                changed |= ui.selectable_value(&mut self.settings.language, code.clone(), name).changed();
                            }
                        });
                        if changed {
                            i18n::set_language(&self.settings.language);
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(t("Interface size"));
                        let r = ui.add(egui::Slider::new(&mut self.settings.zoom, 0.75..=2.0).step_by(0.05));
                        if r.changed() {
                            ctx.set_zoom_factor(self.settings.zoom);
                        }
                        changed |= r.drag_stopped() || r.lost_focus();
                    });
                    small_muted(ui, t("Translations live in the lang folder; you can add or improve languages without rebuilding."));
                    ui.add_space(8.0);
                    let was_advanced = self.settings.advanced_mode;
                    ui.horizontal(|ui| {
                        ui.label(t("Interface mode"));
                        changed |= ui
                            .radio_value(&mut self.settings.advanced_mode, false, t("Simple"))
                            .on_hover_text(t("Plain language, fewer technical details. Recommended if you're not sure."))
                            .changed();
                        changed |= ui
                            .radio_value(&mut self.settings.advanced_mode, true, t("Advanced"))
                            .on_hover_text(t("All details: registry keys, services, scheduled tasks and the Installer Lab for developers."))
                            .changed();
                    });
                    if was_advanced != self.settings.advanced_mode {
                        // System components appear or disappear with the mode.
                        self.programs.needs_refresh = true;
                    }
                    if ui.checkbox(&mut self.settings.high_contrast, t("High contrast (brighter text, stronger outlines)")).changed() {
                        changed = true;
                        super::theme::apply(ctx, self.settings.high_contrast);
                    }
                });
                ui.add_space(10.0);
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Keyboard")).heading());
                    small_muted(ui, t("Everything in InstallScope can be used without a mouse. Screen readers such as Narrator and NVDA read every control."));
                    ui.add_space(4.0);
                    egui::Grid::new("keys").num_columns(2).spacing([18.0, 4.0]).show(ui, |ui| {
                        for (keys, what) in SHORTCUTS {
                            ui.label(RichText::new(*keys).monospace().color(theme::ACCENT_TEXT));
                            ui.label(t(what));
                            ui.end_row();
                        }
                    });
                });
                ui.add_space(10.0);
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Safety")).heading());
                    ui.add_space(4.0);
                    changed |= ui.checkbox(&mut self.settings.create_restore_point, t("Create a System Restore point before uninstalling")).changed();
                    if self.settings.advanced_mode {
                        changed |= ui
                            .checkbox(
                                &mut self.settings.backup_before_removal,
                                t("Back up everything before removing it (recommended)"),
                            )
                            .on_hover_text(t("If disabled, files and registry changes made by InstallScope's removal and cleanup workflows cannot be restored from the Backups page."))
                            .changed();
                        if !self.settings.backup_before_removal {
                            theme::warning_banner(ui, t("Backups are off. InstallScope cannot restore files or registry changes made by removal and cleanup workflows."));
                        }
                        changed |= ui.checkbox(&mut self.settings.confirm_actions, t("Ask before deleting things")).changed();
                        small_muted(ui, t("When off, InstallScope deletes without asking first. Restoring your PC to a restore point, deleting a backup and deep cleaning always ask, because they can't be undone."));
                        changed |= ui
                            .checkbox(&mut self.settings.use_recycle_bin, t("Also send removed files to the Recycle Bin"))
                            .on_hover_text(t("Everything is backed up either way. This additionally keeps removed files in the Recycle Bin, using extra disk space."))
                            .changed();
                        changed |= ui
                            .checkbox(&mut self.settings.close_programs, t("Close the program if it is running"))
                            .on_hover_text(t("Running copies of the program being removed are asked to close (so they can save their work) and its files are deleted at once. Other programs are never closed."))
                            .changed();
                        changed |= ui
                            .checkbox(&mut self.settings.restart_explorer, t("Restart Windows Explorer if it is using the program's files"))
                            .on_hover_text(t("Shell extensions stay loaded in Explorer until it restarts. Restarting it during the removal deletes them at once instead of at the next restart. The taskbar disappears for a moment."))
                            .changed();
                        changed |= ui
                            .checkbox(&mut self.settings.remove_unlogged_files, t("When uninstalling a monitored program, also remove files it created later"))
                            .changed();
                    } else {
                        small_muted(ui, t("Simple mode always backs up before removing and asks before deleting. Options that could harm Windows or slow your PC are only in Advanced mode."));
                    }
                });
                if self.settings.advanced_mode {
                    ui.add_space(10.0);
                    theme::card(ui, |ui| {
                        ui.label(RichText::new(t("Installer Decrapifier")).heading());
                        small_muted(ui, t("After an installation is recorded, its services, scheduled tasks and autoruns are judged by built-in rules. For the ones the rules can't judge, you can ask a language model running on this PC (Ollama, llama.cpp, LM Studio). Only addresses on this computer are accepted, so nothing leaves it."));
                        ui.add_space(4.0);
                        egui::Grid::new("local_model").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                            ui.label(t("Model address"));
                            changed |= ui
                                .add(egui::TextEdit::singleline(&mut self.settings.local_model_url)
                                    .hint_text("http://localhost:11434/v1/chat/completions")
                                    .desired_width(360.0))
                                .changed();
                            ui.end_row();
                            ui.label(t("Model name"));
                            changed |= ui
                                .add(egui::TextEdit::singleline(&mut self.settings.local_model_name)
                                    .hint_text("llama3.2")
                                    .desired_width(200.0))
                                .changed();
                            ui.end_row();
                        });
                        if !self.settings.local_model_url.is_empty() {
                            if let Err(e) = installscope_core::llm::parse_local_url(&self.settings.local_model_url) {
                                theme::warning_banner(ui, t(&e.to_string()));
                            }
                        }
                    });
                }
                ui.add_space(10.0);
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Program list")).heading());
                    ui.add_space(4.0);
                    let mut refresh = false;
                    if self.settings.advanced_mode {
                        refresh |= ui.checkbox(&mut self.settings.show_system_components, t("Show system components and updates")).changed();
                    }
                    refresh |= ui.checkbox(&mut self.settings.show_store_apps, t("Load Microsoft Store apps at startup")).changed();
                    if refresh {
                        changed = true;
                        self.programs.needs_refresh = true;
                    }
                });
                if self.settings.advanced_mode {
                    ui.add_space(10.0);
                    theme::card(ui, |ui| {
                        ui.label(RichText::new(t("Background monitoring")).heading());
                        ui.add_space(4.0);
                        if let Some(v) = self.auto_watch_job.as_ref().and_then(|j| j.poll()) {
                            self.auto_watch_job = None;
                            self.auto_watch = Some(v);
                        }
                        if self.auto_watch.is_none() && self.auto_watch_job.is_none() {
                            self.auto_watch_job = Some(super::Job::spawn(ui.ctx(), |_| installscope_core::autowatch::is_enabled()));
                        }
                        let mut on = self.auto_watch.unwrap_or(false);
                        let r = ui.add_enabled(
                            self.elevated && self.auto_watch.is_some(),
                            egui::Checkbox::new(&mut on, t("Record installations automatically, even ones I forget to monitor")),
                        );
                        if r.changed() {
                            let res = if on { installscope_core::autowatch::enable() } else { installscope_core::autowatch::disable() };
                            match res {
                                Ok(()) => self.auto_watch = Some(on),
                                Err(e) => self.set_status(e.to_string()),
                            }
                        }
                        small_muted(ui, t("Starts with Windows at low priority, keeps a snapshot, and saves an install log (tagged \"auto\") whenever a new program appears. Taking snapshots uses some disk and memory after sign-in. Needs Administrator rights to turn on or off."));
                    });
                }
                if changed {
                    self.save_settings();
                }
                ui.add_space(10.0);
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Explorer integration")).heading());
                    ui.add_space(4.0);
                    let reg = self.reg.clone();
                    let (machine, mut user) = *self.settings_ui.shell_registered.get_or_insert_with(|| {
                        (
                            installscope_core::shell::is_registered(reg.as_ref(), true),
                            installscope_core::shell::is_registered(reg.as_ref(), false),
                        )
                    });
                    if machine {
                        ui.label(format!("✔ {}", t("\"Install with InstallScope\" is in the right-click menu for all users (added by Setup).")));
                    } else if ui
                        .checkbox(&mut user, t("Add \"Install with InstallScope\" to the right-click menu of installers"))
                        .changed()
                    {
                        let res = if user {
                            let exe = std::env::current_exe().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
                            installscope_core::shell::register(reg.as_ref(), &exe, &t("Install with InstallScope"), false)
                        } else {
                            installscope_core::shell::unregister(reg.as_ref(), false)
                        };
                        if let Err(e) = res {
                            self.set_status(e.to_string());
                        }
                        self.settings_ui.shell_registered = None;
                    }
                    small_muted(
                        ui,
                        t("Right-click any .exe or .msi installer to install it under monitoring. On Windows 11 it is under \"Show more options\"."),
                    );
                });
                ui.add_space(10.0);
                if !self.settings.advanced_mode {
                    theme::card(ui, |ui| {
                        small_muted(ui, t("Switch to Advanced mode to change what snapshots capture."));
                        if ui.button(t("Reset everything to defaults")).clicked() {
                            self.ask("Reset settings?", t("All settings except the language go back to their defaults."), "Reset", Confirm::ResetSettings);
                        }
                    });
                } else {
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Monitoring scope")).heading());
                    small_muted(ui, t("What snapshots capture. One entry per line; %VARIABLES% and * wildcards work."));
                    ui.add_space(6.0);
                    self.profile_picker(ui, true);
                    ui.add_space(6.0);
                    self.ui_noise(ui, ctx);
                    ui.add_space(6.0);
                    let st = &mut self.settings_ui;
                    let mut dirty = st.dirty;
                    list_editor(ui, "Registry keys", "Whole subtrees are captured.", &mut st.reg_roots, &mut dirty);
                    list_editor(ui, "Folders (with subfolders)", "", &mut st.file_roots_rec, &mut dirty);
                    list_editor(ui, "Folders (top level only)", "", &mut st.file_roots_top, &mut dirty);
                    list_editor(ui, "Ignored registry keys", "Background noise Windows changes on its own.", &mut st.reg_excl, &mut dirty);
                    list_editor(ui, "Ignored files and folders", "", &mut st.file_excl, &mut dirty);
                    st.dirty = dirty;
                    ui.horizontal(|ui| {
                        if ui.add_enabled(self.settings_ui.dirty, egui::Button::new(t("Save scope"))).clicked() {
                            self.settings_ui.apply(&mut self.settings);
                            self.settings_ui.dirty = false;
                            // Keep the chosen profile in step with its edits.
                            let active = self.settings.active_profile.clone();
                            if !active.is_empty() {
                                self.settings.save_profile(&active);
                            }
                            self.save_settings();
                            self.set_status(t("Settings saved."));
                        }
                        if ui.button(t("Reset everything to defaults")).clicked() {
                            self.ask("Reset settings?", t("All settings except the language go back to their defaults."), "Reset", Confirm::ResetSettings);
                        }
                    });
                });
                }
                ui.add_space(10.0);
                theme::card(ui, |ui| {
                    ui.label(RichText::new(t("Data")).heading());
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.label(t("Data folder"));
                        ui.add(egui::Label::new(RichText::new(self.store.root.to_string_lossy()).monospace()).truncate());
                        if ui.small_button(t("Open")).clicked() {
                            installscope_core::platform::shell_open(&self.store.root.to_string_lossy());
                        }
                    });
                    small_muted(
                        ui,
                        if self.store.portable {
                            t("Portable mode: everything is stored next to InstallScope.exe.")
                        } else {
                            t("Tip: put an empty file named \"portable\" next to InstallScope.exe to keep all data beside it.")
                        },
                    );
                });
            });
        });
    }
}
