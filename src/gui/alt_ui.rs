//! Suggestions of lightweight and open-source alternatives, installable
//! through winget or Scoop with one click.

use super::app::App;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use setupsweep_core::alternatives::{self, Alternative, Managers, Via};
use setupsweep_core::i18n::{t, tf};
use std::collections::HashMap;

#[derive(Default)]
pub struct AltState {
    pub managers: Option<Managers>,
    pub detect_job: Option<Job<Managers>>,
    pub install_job: Option<Job<(String, Result<String, String>)>>,
    /// Result per alternative name: Ok(()) installed, Err(message).
    pub results: HashMap<String, Result<(), String>>,
}

impl App {
    pub fn poll_alternatives(&mut self) {
        if let Some(m) = self.alt.detect_job.as_ref().and_then(|j| j.poll()) {
            self.alt.detect_job = None;
            self.alt.managers = Some(m);
        }
        if let Some((name, r)) = self.alt.install_job.as_ref().and_then(|j| j.poll()) {
            self.alt.install_job = None;
            match &r {
                Ok(_) => self.set_status(tf("Installed {name}", &[("name", name.clone())])),
                Err(e) => self.set_status(tf(
                    "{name} wasn't installed: {error}",
                    &[("name", name.clone()), ("error", e.clone())],
                )),
            }
            self.alt.results.insert(name, r.map(|_| ()));
            self.programs.needs_refresh = true;
        }
    }

    /// Alternatives for the named programs; nothing is shown when there are none.
    pub fn alternatives_ui(&mut self, ui: &mut egui::Ui, programs: &[String]) {
        let installed: Vec<String> = self
            .programs
            .list
            .iter()
            .map(|p| p.name.to_lowercase())
            .collect();
        alternatives_widget(ui, &mut self.alt, &installed, programs);
    }
}

fn install_alternative(st: &mut AltState, ctx: &egui::Context, alt: Alternative, via: Via) {
    let name = alt.name.to_string();
    st.install_job = Some(Job::spawn(ctx, move |_| {
        let r = alternatives::install(&alt, via).map_err(|e| e.to_string());
        (name, r)
    }));
}

/// The alternatives card, usable where the App is already borrowed.
pub fn alternatives_widget(
    ui: &mut egui::Ui,
    st: &mut AltState,
    installed: &[String],
    programs: &[String],
) {
    {
        let mut alts: Vec<Alternative> = Vec::new();
        for p in programs {
            for a in alternatives::for_program(p) {
                if !alts.iter().any(|x| x.name == a.name) {
                    alts.push(a);
                }
            }
        }
        if alts.is_empty() {
            return;
        }
        if st.managers.is_none() && st.detect_job.is_none() {
            st.detect_job = Some(Job::spawn(ui.ctx(), |_| alternatives::detect()));
        }
        let m = st.managers.unwrap_or_default();
        let busy = st.install_job.is_some();
        let ctx = ui.ctx().clone();
        theme::card(ui, |ui| {
            ui.label(RichText::new(t("Lightweight alternatives")).strong());
            small_muted(ui, t("Free programs that do the same job. Install one with a click through winget or Scoop; nothing is installed unless you choose it."));
            ui.add_space(4.0);
            for a in alts {
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(a.name).strong());
                    if a.open_source {
                        theme::badge(ui, t("Open source"), theme::OK);
                    }
                });
                small_muted(ui, t(a.what));
                ui.horizontal_wrapped(|ui| {
                    {
                        let already = installed.iter().any(|n| n.contains(&a.name.to_lowercase()));
                        match st.results.get(a.name).cloned() {
                            _ if already => {
                                ui.label(RichText::new(format!("✔ {}", t("Installed"))).color(theme::OK));
                            }
                            Some(Ok(())) => {
                                ui.label(RichText::new(format!("✔ {}", t("Installed"))).color(theme::OK));
                            }
                            other => {
                                if let Some(Err(e)) = &other {
                                    // The failure reason is inline text, not
                                    // hover-only (audit P1-18): everyone can
                                    // read why the install didn't happen.
                                    ui.label(
                                        RichText::new(format!("✖ {e}")).color(theme::BAD).small(),
                                    );
                                }
                                let scoop_why = if busy {
                                    t("Unavailable while another alternative is installing.")
                                } else {
                                    t("Downloads and installs this program on this PC through Scoop.")
                                };
                                let winget_why = if busy {
                                    t("Unavailable while another alternative is installing.")
                                } else {
                                    t("Downloads and installs this program on this PC through winget.")
                                };
                                if m.scoop && a.scoop.is_some() && super::a11y::explain(ui.add_enabled(!busy, egui::Button::new(t("Install with Scoop")).small()), &scoop_why).clicked() {
                                    install_alternative(st, &ctx, a.clone(), Via::Scoop);
                                }
                                if m.winget && a.winget.is_some() && super::a11y::explain(ui.add_enabled(!busy, egui::Button::new(t("Install with winget")).small()), &winget_why).clicked() {
                                    install_alternative(st, &ctx, a.clone(), Via::Winget);
                                }
                                if a.winget.is_none() && a.scoop.is_none() {
                                    small_muted(ui, t("Get it from its website"));
                                } else if st.managers.is_some() && !m.winget && !m.scoop {
                                    small_muted(ui, t("Needs winget (App Installer from the Microsoft Store) or Scoop"));
                                }
                            }
                        }
                    }
                });
            }
            if busy {
                ui.horizontal(|ui| {
                    spinner(ui);
                    small_muted(ui, t("Installing…"));
                });
            }
        });
    }
}
