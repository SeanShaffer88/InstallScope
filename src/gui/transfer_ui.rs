//! Transfer programs to another PC: export wizard and restore window.

use super::app::App;
use super::theme;
use super::widgets::*;
use super::Job;
use egui::RichText;
use installscope_core::backup::{ActionResult, BackupWriter};
use installscope_core::i18n::{t, tf};
use installscope_core::model::human_size;
use installscope_core::programs::InstalledProgram;
use installscope_core::transfer::{self, Check, Collected, Manifest};
use std::path::PathBuf;

type RestoreOutcome = Result<(Vec<ActionResult>, Option<String>), String>;

pub enum TransferUi {
    Export {
        names: Vec<String>,
        collect_job: Option<Job<Collected>>,
        collected: Option<Collected>,
        write_job: Option<Job<Result<PathBuf, String>>>,
        done: Option<Result<PathBuf, String>>,
    },
    Import {
        path: PathBuf,
        load_job: Option<Job<Result<(Manifest, Check), String>>>,
        loaded: Option<Result<(Manifest, Check), String>>,
        overwrite: bool,
        run_job: Option<Job<RestoreOutcome>>,
        done: Option<RestoreOutcome>,
    },
}

fn list_names(ui: &mut egui::Ui, names: &[String]) {
    egui::ScrollArea::vertical()
        .id_salt("transfer_names")
        .max_height(110.0)
        .show(ui, |ui| {
            for n in names {
                ui.label(format!("• {n}"));
            }
        });
}

fn warnings(ui: &mut egui::Ui, list: &[String], color: egui::Color32) {
    for w in list {
        ui.label(RichText::new(format!("⚠ {w}")).color(color).small());
    }
}

impl App {
    /// Start collecting what belongs to these programs.
    pub fn start_transfer_export(&mut self, programs: Vec<InstalledProgram>) {
        if programs.is_empty() {
            return;
        }
        let logs: Vec<_> = programs
            .iter()
            .map(|p| super::programs::log_for(&self.logs.logs, p).map(|l| l.changes.clone()))
            .collect();
        let reg = self.reg.clone();
        let names = programs.iter().map(|p| p.name.clone()).collect();
        let job = Job::spawn(&self.ctx, move |pr| {
            transfer::collect(&programs, &logs, reg.as_ref(), &|s| pr.set_phase(s))
        });
        self.transfer = Some(TransferUi::Export {
            names,
            collect_job: Some(job),
            collected: None,
            write_job: None,
            done: None,
        });
    }

    /// Ask for a package and check it against this PC.
    pub fn start_transfer_import(&mut self) {
        let Some(path) = pick_file(
            &t("Restore programs from a transfer package"),
            "InstallScope transfer package",
            &[transfer::EXTENSION],
        ) else {
            return;
        };
        let reg = self.reg.clone();
        let p = path.clone();
        let job = Job::spawn(&self.ctx, move |_| {
            let m = transfer::read_manifest(&p).map_err(|e| e.to_string())?;
            let c = transfer::check(&m, reg.as_ref(), &transfer::Vars::current());
            Ok((m, c))
        });
        self.transfer = Some(TransferUi::Import {
            path,
            load_job: Some(job),
            loaded: None,
            overwrite: false,
            run_job: None,
            done: None,
        });
    }

    pub fn ui_transfer(&mut self, ctx: &egui::Context) {
        let Some(tr) = &mut self.transfer else { return };
        let mut close = false;
        let mut open = true;
        let busy = match tr {
            TransferUi::Export {
                collect_job,
                write_job,
                ..
            } => collect_job.is_some() || write_job.is_some(),
            TransferUi::Import {
                load_job, run_job, ..
            } => load_job.is_some() || run_job.is_some(),
        };
        let title = match tr {
            TransferUi::Export { .. } => t("Transfer to another PC"),
            TransferUi::Import { .. } => t("Restore transferred programs"),
        };
        let elevated = self.elevated;
        let backups_dir = self.store.backups_dir();
        let reg = self.reg.clone();
        let mut refresh_after = false;
        egui::Window::new(title)
            .id(egui::Id::new("transfer"))
            .collapsible(false)
            .resizable(true)
            .default_width(620.0)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| match tr {
                TransferUi::Export { names, collect_job, collected, write_job, done } => {
                    if let Some(c) = collect_job.as_ref().and_then(|j| j.poll()) {
                        *collect_job = None;
                        *collected = Some(c);
                    }
                    if let Some(r) = write_job.as_ref().and_then(|j| j.poll()) {
                        *write_job = None;
                        *done = Some(r);
                    }
                    if let Some(job) = collect_job {
                        job_progress(ui, &tf("Finding everything that belongs to {name}…", &[("name", job.progress.phase())]), 0);
                        return;
                    }
                    if let Some(job) = write_job {
                        job_progress(ui, &tf("Packing… {name}", &[("name", job.progress.phase())]), 0);
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                        return;
                    }
                    if let Some(r) = done {
                        match r {
                            Ok(path) => {
                                ui.label(RichText::new(format!("✔ {}", t("Package saved."))).color(theme::OK).strong());
                                small_muted(ui, path.display().to_string());
                                ui.add_space(6.0);
                                small_muted(ui, t("Copy it to the other PC, open InstallScope there and choose File > Restore programs from a package."));
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if super::a11y::explain(
                                        ui.button(t("Show in folder")),
                                        &t("Opens File Explorer at the saved package."),
                                    )
                                    .clicked()
                                    {
                                        installscope_core::platform::reveal(&path.to_string_lossy());
                                    }
                                    if super::a11y::explain(
                                        theme::primary_button(ui, t("Close")),
                                        &t("Closes this window."),
                                    )
                                    .clicked()
                                    {
                                        close = true;
                                    }
                                });
                            }
                            Err(e) => {
                                theme::warning_banner(ui, e.clone());
                                if super::a11y::explain(
                                    ui.button(t("Close")),
                                    &t("Closes this window."),
                                )
                                .clicked()
                                {
                                    close = true;
                                }
                            }
                        }
                        return;
                    }
                    let Some(c) = collected else { return };
                    ui.label(t("These programs will be packed with their files, settings and registry entries:"));
                    list_names(ui, names);
                    ui.add_space(6.0);
                    egui::Grid::new("transfer_summary").num_columns(2).spacing([16.0, 4.0]).show(ui, |ui| {
                        ui.label(t("Files"));
                        ui.label(format!("{} ({})", c.files.len(), human_size(c.bytes)));
                        ui.end_row();
                        ui.label(t("Registry keys"));
                        ui.label(format!("{}", c.keys.len() + c.values.len()));
                        ui.end_row();
                        if !c.services.is_empty() {
                            ui.label(t("Services"));
                            ui.label(c.services.len().to_string());
                            ui.end_row();
                        }
                        if !c.tasks.is_empty() {
                            ui.label(t("Scheduled tasks"));
                            ui.label(c.tasks.len().to_string());
                            ui.end_row();
                        }
                    });
                    if !c.warnings.is_empty() {
                        ui.add_space(6.0);
                        warnings(ui, &c.warnings, theme::WARN);
                    }
                    if c.files.is_empty() && c.keys.is_empty() {
                        ui.add_space(6.0);
                        theme::warning_banner(ui, t("Nothing was found to transfer."));
                    }
                    ui.add_space(8.0);
                    small_muted(ui, t("Tip: close the programs first, so their files aren't in use. Licences tied to this PC may need to be activated again."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        let can = !(c.files.is_empty() && c.keys.is_empty());
                        let save_why = if can {
                            t("Saves the files, settings and registry entries listed above as one transfer package, in a file you choose.")
                        } else {
                            t("Unavailable: nothing was found to pack for these programs.")
                        };
                        if super::a11y::explain(
                            ui.add_enabled(can, egui::Button::new(format!("💾 {}", t("Save package…")))),
                            &save_why,
                        )
                        .clicked()
                        {
                            let default = format!(
                                "{}.{}",
                                installscope_core::util::safe_file_name(&if names.len() == 1 { names[0].clone() } else { tf("{n} programs", &[("n", names.len().to_string())]) }),
                                transfer::EXTENSION
                            );
                            if let Some(path) = save_file(&t("Save transfer package"), &default, "InstallScope transfer package", transfer::EXTENSION) {
                                let c2 = c.clone();
                                let reg2 = reg.clone();
                                *write_job = Some(Job::spawn(ui.ctx(), move |pr| {
                                    transfer::export(&c2, reg2.as_ref(), &path, &|s| pr.set_phase(s))
                                        .map(|_| path.clone())
                                        .map_err(|e| e.to_string())
                                }));
                            }
                        }
                        if super::a11y::explain(
                            ui.button(t("Cancel")),
                            &t("Closes this window without saving a package."),
                        )
                        .clicked()
                        {
                            close = true;
                        }
                    });
                }
                TransferUi::Import { path, load_job, loaded, overwrite, run_job, done } => {
                    if let Some(r) = load_job.as_ref().and_then(|j| j.poll()) {
                        *load_job = None;
                        *loaded = Some(r);
                    }
                    if let Some(r) = run_job.as_ref().and_then(|j| j.poll()) {
                        *run_job = None;
                        *done = Some(r);
                        refresh_after = true;
                    }
                    if load_job.is_some() {
                        job_progress(ui, &t("Reading the package…"), 0);
                        return;
                    }
                    if let Some(job) = run_job {
                        job_progress(ui, &tf("Restoring… {name}", &[("name", job.progress.phase())]), 0);
                        ui.ctx().request_repaint_after(std::time::Duration::from_millis(300));
                        return;
                    }
                    if let Some(r) = done {
                        match r {
                            Ok((results, backup)) => {
                                let failed: Vec<&ActionResult> = results.iter().filter(|r| !r.ok).collect();
                                if failed.is_empty() {
                                    ui.label(RichText::new(format!("✔ {}", t("The programs were restored."))).color(theme::OK).strong());
                                } else {
                                    ui.label(RichText::new(tf("Restored with {n} problems.", &[("n", failed.len().to_string())])).color(theme::WARN).strong());
                                }
                                for r in results.iter().filter(|r| r.ok && !r.description.starts_with("File ")) {
                                    small_muted(ui, format!("{} — {}", r.description, r.message));
                                }
                                egui::ScrollArea::vertical().id_salt("transfer_fail").max_height(150.0).show(ui, |ui| {
                                    for r in failed {
                                        ui.label(RichText::new(format!("✖ {} — {}", r.description, r.message)).color(theme::BAD).small());
                                    }
                                });
                                if backup.is_some() {
                                    ui.add_space(6.0);
                                    small_muted(ui, t("Changed your mind? The whole transfer can be undone from the Backups page."));
                                }
                            }
                            Err(e) => theme::warning_banner(ui, e.clone()),
                        }
                        ui.add_space(8.0);
                        if super::a11y::explain(
                            theme::primary_button(ui, t("Close")),
                            &t("Closes this window."),
                        )
                        .clicked()
                        {
                            close = true;
                        }
                        return;
                    }
                    let Some(r) = loaded else { return };
                    let (m, c) = match r {
                        Ok(x) => x,
                        Err(e) => {
                            theme::warning_banner(ui, e.clone());
                            if super::a11y::explain(
                                ui.button(t("Close")),
                                &t("Closes this window."),
                            )
                            .clicked()
                            {
                                close = true;
                            }
                            return;
                        }
                    };
                    small_muted(ui, tf("From {computer} ({user}), {date}", &[("computer", m.computer.clone()), ("user", m.user.clone()), ("date", m.created.clone())]));
                    ui.add_space(4.0);
                    let names: Vec<String> = m.programs.iter().map(|p| if p.version.is_empty() { p.name.clone() } else { format!("{} {}", p.name, p.version) }).collect();
                    list_names(ui, &names);
                    ui.add_space(6.0);
                    ui.label(tf("{files} files ({size}), {keys} registry entries", &[
                        ("files", m.files.len().to_string()),
                        ("size", human_size(m.bytes())),
                        ("keys", (m.keys.len() + m.values.len()).to_string()),
                    ]));
                    if !c.problems.is_empty() {
                        ui.add_space(6.0);
                        warnings(ui, &c.problems, theme::BAD);
                    }
                    if !c.warnings.is_empty() {
                        ui.add_space(4.0);
                        warnings(ui, &c.warnings, theme::WARN);
                    }
                    if !c.conflicts.is_empty() {
                        ui.add_space(6.0);
                        // The conflicting files are listed as visible text
                        // under the checkbox, not only in a hover: the
                        // explanation stays a one-line summary.
                        super::a11y::explain(
                            ui.checkbox(overwrite, tf("Replace {n} files that already exist on this PC", &[("n", c.conflicts.len().to_string())])),
                            &t("Replace the files listed below, which already exist on this PC."),
                        );
                        for p in c.conflicts.iter().take(15) {
                            small_muted(ui, p.clone());
                        }
                        if c.conflicts.len() > 15 {
                            small_muted(
                                ui,
                                tf(
                                    "…and {n} more",
                                    &[("n", (c.conflicts.len() - 15).to_string())],
                                ),
                            );
                        }
                    }
                    let blocked = !c.problems.is_empty() || (c.needs_admin && !elevated);
                    let restore_why = if !c.problems.is_empty() {
                        t("Unavailable: the problems listed above must be sorted out first; InstallScope won't restore a package that failed its checks.")
                    } else if c.needs_admin && !elevated {
                        t("Unavailable: run InstallScope as Administrator to restore these programs.")
                    } else {
                        t("Writes the programs' files, settings and registry entries onto this PC. Everything written is recorded, so the transfer can be undone from the Backups page.")
                    };
                    if c.needs_admin && !elevated {
                        ui.add_space(6.0);
                        theme::warning_banner(ui, t("Run InstallScope as Administrator to restore these programs (they go into Program Files and the machine-wide registry)."));
                    }
                    ui.add_space(6.0);
                    small_muted(ui, t("Everything InstallScope writes is recorded, so the transfer can be undone from the Backups page."));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if super::a11y::explain(
                            ui.add_enabled(!blocked, egui::Button::new(format!("📥 {}", t("Restore")))),
                            &restore_why,
                        )
                        .clicked()
                        {
                            let pkg = path.clone();
                            let ow = *overwrite;
                            let reg2 = reg.clone();
                            let root = backups_dir.clone();
                            let title = tf("Transfer: {names}", &[("names", m.programs.iter().map(|p| p.name.clone()).collect::<Vec<_>>().join(", "))]);
                            *run_job = Some(Job::spawn(ui.ctx(), move |pr| {
                                let mut bw = BackupWriter::new(&root, &title).map_err(|e| e.to_string())?;
                                let res = transfer::restore(&pkg, reg2.as_ref(), &mut bw, ow, &|s| pr.set_phase(s)).map_err(|e| e.to_string());
                                let id = if bw.is_empty() {
                                    bw.discard();
                                    None
                                } else {
                                    match bw.finish() {
                                        Ok(backup) => Some(backup.id),
                                        Err(error) => {
                                            return Err(format!(
                                                "The restore may have completed, but its backup could not be finalized: {error:#}"
                                            ));
                                        }
                                    }
                                };
                                res.map(|r| (r, id))
                            }));
                        }
                        if super::a11y::explain(
                            ui.button(t("Cancel")),
                            &t("Closes this window without restoring anything."),
                        )
                        .clicked()
                        {
                            close = true;
                        }
                    });
                }
            });
        if refresh_after {
            self.reload_backups();
            self.programs.needs_refresh = true;
        }
        if (!open || close || (!busy && super::a11y::escape_pressed(ctx) && self.confirm.is_none()))
            && !busy
        {
            self.transfer = None;
        }
    }
}
