//! Developer scratch storage and a discoverable entry to reviewed app transfers.
use super::{
    app::{App, Tab},
    a11y, theme, Job,
};
use std::path::PathBuf;

pub struct ScratchState {
    parent: String,
    job: Option<Job<Result<PathBuf, String>>>,
    created: Vec<PathBuf>,
    error: Option<String>,
}
impl Default for ScratchState {
    fn default() -> Self {
        Self {
            parent: std::env::temp_dir().to_string_lossy().into_owned(),
            job: None,
            created: Vec::new(),
            error: None,
        }
    }
}
impl App {
    pub fn ui_scratch(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.scratch.job.as_ref().and_then(|j| j.poll()) {
            self.scratch.job = None;
            match result {
                Ok(path) => self.scratch.created.push(path),
                Err(e) => self.scratch.error = Some(e),
            }
        }
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
          egui::ScrollArea::vertical().id_salt("workbench_body").show(ui, |ui| {
            ui.heading("Disk-Backed Scratch Workspace");
            ui.label("Create a unique folder on an existing drive for temporary build output and experiments. This does not create a virtual disk, reserve space, impose a quota, or isolate programs.");
            theme::warning_banner(ui, "No automatic cleanup. Windows or another cleaner may remove folders under TEMP. Keep source, credentials and durable files elsewhere.");
            ui.horizontal_wrapped(|ui| {
                let label = ui.label("Parent folder");
                a11y::explain(ui.add(egui::TextEdit::singleline(&mut self.scratch.parent).char_limit(4096).desired_width(500.0)).labelled_by(label.id), "Existing local parent folder. A uniquely named child will be created here; existing contents are preserved.");
            });
            if a11y::explain(ui.add_enabled(self.scratch.job.is_none(), egui::Button::new("Create New Scratch Folder")), "Create one new folder without changing global TEMP or launching programs. Remove it manually when no longer needed; SetupScope never automatically deletes its contents.").clicked() {
                let parent = PathBuf::from(&self.scratch.parent);
                self.scratch.error = None;
                self.scratch.job = Some(Job::spawn(ctx, move |_| setupscope_core::scratch::create(&parent).map_err(|e| e.to_string())));
            }
            if self.scratch.job.is_some() { ui.spinner(); ctx.request_repaint_after(std::time::Duration::from_millis(150)); }
            if let Some(error) = &self.scratch.error { ui.colored_label(theme::BAD, error); }
            egui::ScrollArea::vertical().show(ui, |ui| {
                for folder in self.scratch.created.iter().rev() {
                    ui.separator();
                    ui.add(egui::Label::new(folder.display().to_string()).selectable(true));
                    ui.horizontal_wrapped(|ui| {
                        if a11y::explain(ui.button("Copy Folder Path"), "Copy this local path to the clipboard. No program starts or files move.").clicked() { ctx.copy_text(folder.display().to_string()); }
                        if a11y::explain(ui.button("Copy PowerShell TEMP / TMP Assignments"), "Copy shell-scoped assignments for review. Paste into your own PowerShell session to apply; closing that shell discards its environment changes.").clicked() { ctx.copy_text(setupscope_core::scratch::powershell_environment(folder)); }
                    });
                    ui.label("Assignments affect only the shell where you paste them and its future children. SetupScope does not execute them. Existing processes keep their environment.");
                }
            });
          });
        });
    }
    pub fn ui_migration(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
          egui::ScrollArea::vertical().id_salt("workbench_body").show(ui, |ui| {
            ui.heading("App Migration And Transfer");
            ui.label("Use the existing reviewed transfer workflow to collect supported app files and settings into a package, or check a package against this PC before restoring.");
            theme::warning_banner(ui, "Copying an installed directory is not a universal migration. Services, drivers, Store apps, machine-specific registration, activation, credentials and licensing may require a vendor reinstall. Keep the original installation and backups until the destination is verified.");
            ui.label("Export: open Programs, select the applications, then choose the transfer action. Review collected paths, exclusions and warnings before saving the package.");
            if a11y::explain(ui.button("Select Apps In Programs"), "Open the program inventory to choose apps for a separately reviewed transfer package.").clicked() { self.open_tool(ctx, Tab::Programs); }
            ui.add_space(12.0);
            ui.label("Import: select a SetupScope transfer package. Compatibility checks and a separate restore review appear in the main dashboard. Nothing is restored by opening this tool.");
            if a11y::explain(ui.add_enabled(self.transfer.is_none(), egui::Button::new("Inspect Transfer Package…")), "Choose an existing package and inspect compatibility. Restore requires a separate review in the dashboard.").clicked() {
                self.start_transfer_import();
                ctx.send_viewport_cmd_to(egui::ViewportId::ROOT, egui::ViewportCommand::Focus);
            }
            if self.transfer.is_some() { ui.label("A transfer review is already open in the main dashboard."); }
          });
        });
    }
}
