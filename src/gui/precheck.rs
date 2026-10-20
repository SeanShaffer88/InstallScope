//! Before installing: screen the setup file for adware and warning signs,
//! and optionally test it in Windows Sandbox to preview everything it
//! changes without touching this PC.

use super::app::App;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::i18n::{t, tf};
use installscope_core::model::{human_size, InstallLog};
use installscope_core::pafbuild;
use installscope_core::prescreen::{self, Level, Screening};
use std::path::PathBuf;

#[derive(Default)]
pub struct PreCheck {
    pub path: String,
    pub name: String,
    job: Option<Job<Screening>>,
    pub result: Option<Screening>,
    /// Sandbox preview in progress: its work folder.
    sandbox_work: Option<PathBuf>,
    sandbox_phase: String,
    last_poll: Option<std::time::Instant>,
    pub preview: Option<InstallLog>,
    error: Option<String>,
    tree: super::difftree_ui::TreeView,
}

pub fn level_color(l: Level) -> egui::Color32 {
    match l {
        Level::Danger => theme::BAD,
        Level::Caution => theme::WARN,
        Level::Info => theme::MUTED,
    }
}

impl App {
    /// Screen an installer before monitoring it.
    pub fn precheck_installer(&mut self, ctx: &egui::Context, path: String) {
        let name = std::path::Path::new(&path)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let p = PathBuf::from(&path);
        self.monitor.precheck = Some(PreCheck {
            path,
            name,
            job: Some(Job::spawn(ctx, move |_| prescreen::screen(&p))),
            ..Default::default()
        });
        self.tab = super::app::Tab::Monitor;
    }

    pub fn poll_precheck(&mut self, ctx: &egui::Context) {
        let Some(pc) = &mut self.monitor.precheck else {
            return;
        };
        if let Some(r) = pc.job.as_ref().and_then(|j| j.poll()) {
            pc.job = None;
            pc.result = Some(r);
        }
        let Some(work) = pc.sandbox_work.clone() else {
            return;
        };
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        if pc.last_poll.is_some_and(|t| t.elapsed().as_millis() < 900) {
            return;
        }
        pc.last_poll = Some(std::time::Instant::now());
        if let Some(st) = pafbuild::read_status(&work) {
            pc.sandbox_phase = st.phase.clone();
            if st.done {
                pc.sandbox_work = None;
                if let Some(e) = st.error {
                    pc.error = Some(e);
                } else {
                    match std::fs::read(work.join("install-log.json"))
                        .ok()
                        .and_then(|b| {
                            installscope_core::json::compat::from_slice::<InstallLog>(&b).ok()
                        }) {
                        Some(mut log) => {
                            log.tags = vec!["sandbox-preview".into()];
                            pc.preview = Some(log);
                        }
                        None => pc.error = Some(t("The sandbox didn't return a result.")),
                    }
                }
            }
        }
    }

    fn start_sandbox_preview(&mut self) {
        let Some(pc) = &mut self.monitor.precheck else {
            return;
        };
        let job = pafbuild::Job {
            details: pafbuild::AppDetails {
                name: pc.name.clone(),
                ..Default::default()
            },
            installer: pc.path.clone(),
            learn_seconds: 30,
            preview_only: true,
            settings: self.settings.clone(),
            ..Default::default()
        };
        let work = self
            .store
            .root
            .join("builder")
            .join(installscope_core::util::new_id());
        pc.error = None;
        pc.sandbox_phase = t("Starting Windows Sandbox");
        match pafbuild::start_sandbox(&job, &work) {
            Ok(()) => {
                pc.sandbox_work = Some(work);
                pc.last_poll = None;
            }
            Err(e) => pc.error = Some(e.to_string()),
        }
    }

    /// The pre-install card. Returns true while it is shown.
    pub fn ui_precheck(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) -> bool {
        if self.monitor.precheck.is_none() {
            return false;
        }
        let sandbox = pafbuild::sandbox_available();
        let mut install = false;
        let mut cancel = false;
        let mut preview = false;
        let mut trial = false;
        let pc = self.monitor.precheck.as_mut().unwrap();
        theme::card(ui, |ui| {
            ui.label(
                RichText::new(tf("Before installing {name}", &[("name", pc.name.clone())]))
                    .heading(),
            );
            small_muted(ui, &pc.path);
            ui.add_space(6.0);
            match &pc.result {
                None => {
                    ui.horizontal(|ui| {
                        spinner(ui);
                        ui.label(t(
                            "Checking the setup file for adware and other warning signs…",
                        ));
                    });
                }
                Some(s) => {
                    ui.horizontal_wrapped(|ui| {
                        let (text, col) = match s.worst() {
                            Level::Danger => (t("Warning signs found"), theme::BAD),
                            Level::Caution => {
                                (t("Read the installer's pages carefully"), theme::WARN)
                            }
                            Level::Info => (t("No warning signs found"), theme::OK),
                        };
                        theme::badge(ui, text, col);
                        small_muted(
                            ui,
                            tf("Installer type: {kind}", &[("kind", t(&s.installer))]),
                        );
                        if let Some(sig) = &s.signer {
                            small_muted(ui, tf("Signed by {name}", &[("name", sig.clone())]));
                        }
                    });
                    if let Some(src) = &s.source {
                        small_muted(ui, tf("Downloaded from {url}", &[("url", src.clone())]));
                    }
                    ui.add_space(4.0);
                    for f in &s.findings {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(
                                RichText::new(t(&f.title))
                                    .strong()
                                    .color(level_color(f.level)),
                            );
                        });
                        small_muted(ui, t(&f.detail));
                    }
                }
            }
            if let Some(e) = &pc.error {
                ui.add_space(4.0);
                theme::warning_banner(ui, e.clone());
            }
            if pc.sandbox_work.is_some() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    spinner(ui);
                    ui.label(RichText::new(t(&pc.sandbox_phase)).strong());
                });
                small_muted(ui, t("Install it in the Windows Sandbox window as you normally would, start it once if you like, then click OK there. Nothing happens to this PC."));
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                let busy = pc.sandbox_work.is_some();
                if theme::primary_button(ui, format!("⏺ {}", t("Install with monitoring"))).clicked() {
                    install = true;
                }
                if sandbox
                    && ui
                        .add_enabled(!busy, egui::Button::new(format!("🧪 {}", t("Test in Windows Sandbox first"))))
                        .on_hover_text(t("Runs the installer in a throwaway copy of Windows and shows everything it changes. Your PC is untouched."))
                        .clicked()
                {
                    preview = true;
                }
                if !sandbox
                    && ui
                        .button(format!("🧪 {}", t("Try it, then undo")))
                        .on_hover_text(t("Installs it on this PC while recording every change, shows you what it did, then removes all of it. InstallScope tries to make a restore point first, as an extra safety net."))
                        .clicked()
                {
                    trial = true;
                }
                if ui.button(t("Cancel")).clicked() {
                    cancel = true;
                }
            });
            if !sandbox {
                small_muted(ui, t("Testing in Windows Sandbox needs Windows 11 Pro, Enterprise or Education with the Windows Sandbox feature turned on."));
            }
        });
        if let Some(log) = pc.preview.as_ref() {
            ui.add_space(8.0);
            let st = log.changes.stats();
            theme::card(ui, |ui| {
                ui.label(RichText::new(t("What it did in Windows Sandbox")).heading());
                small_muted(
                    ui,
                    tf(
                        "{files} files and {keys} registry keys added ({size}). This PC was not changed.",
                        &[
                            ("files", st.files_added.to_string()),
                            ("keys", st.keys_added.to_string()),
                            ("size", human_size(st.bytes_added)),
                        ],
                    ),
                );
                ui.add_space(6.0);
                super::difftree_ui::diff_tree_ui(ui, &mut pc.tree, &log.changes);
            });
        }
        if preview {
            self.start_sandbox_preview();
        }
        if install || trial {
            let pc = self.monitor.precheck.take().unwrap();
            self.begin_monitoring(ctx, pc.name, pc.path, trial);
        }
        if cancel {
            self.monitor.precheck = None;
        }
        true
    }
}
