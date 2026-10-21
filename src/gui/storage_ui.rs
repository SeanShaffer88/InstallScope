//! Read-only storage survey. No deletion or automatic opening of discovered files.
use super::{a11y, app::App, theme, widgets::pick_folder, Job};
use egui::RichText;
use installscope_core::{
    i18n::t,
    model::human_size,
    storage::{self, StorageReport},
};
use std::time::Duration;

#[derive(Default)]
pub struct StorageState {
    root: String,
    job: Option<Job<Result<StorageReport, String>>>,
    report: Option<StorageReport>,
    error: Option<String>,
}
impl Drop for StorageState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}
impl App {
    pub fn start_storage_scan(&mut self, ctx: &egui::Context) {
        if self.storage.job.is_some() {
            return;
        }
        if self.storage.root.trim().is_empty() {
            self.storage.error = Some(t("Choose a folder or enter a drive/folder path first."));
            return;
        }
        let root = std::path::PathBuf::from(&self.storage.root);
        self.storage.error = None;
        self.storage.job = Some(Job::spawn(ctx, move |progress| {
            storage::scan(&root, progress).map_err(|e| format!("{e:#}"))
        }));
    }

    pub fn ui_storage(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.storage.job.as_ref().and_then(Job::poll) {
            self.storage.job = None;
            match result {
                Ok(report) => {
                    self.storage.report = Some(report);
                    self.storage.error = None;
                }
                Err(error) => self.storage.error = Some(error),
            }
        }
        let busy = self.storage.job.is_some();
        let mut start = false;
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            a11y::focus_hint(ui, theme::MUTED);
            ui.label(t("Find where space goes before deciding what to remove. This page only reads filesystem metadata; it never deletes files."));
            ui.label(t("Sizes are logical file lengths, not physical allocation or guaranteed recoverable space. Hard-linked files may count more than once. Filesystem activity can change the results during scanning."));
            ui.horizontal_wrapped(|ui| {
                ui.add_enabled_ui(!busy, |ui| {
                    let label = ui.label(t("Folder or drive"));
                    let input = ui.add(egui::TextEdit::singleline(&mut self.storage.root).desired_width(300.0)).labelled_by(label.id);
                    let input = a11y::explain(input, &t("Enter the folder or drive to scan. Only the selected tree is requested; observed links and Windows reparse entries are skipped. Press Enter to scan."));
                    start |= input.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if a11y::explain(ui.button(t("Choose folder…")), &t("Choose a folder using Windows' folder picker; choosing does not start the scan.")).clicked() {
                        if let Some(path) = pick_folder(&t("Choose storage folder")) { self.storage.root = path.to_string_lossy().into_owned(); }
                    }
                    start |= a11y::explain(ui.button(t("Scan storage")), &t("Read the selected folder recursively in the background and rank its immediate children by measured logical size. No files are changed.")).clicked();
                });
                if let Some(job) = &self.storage.job {
                    ui.spinner();
                    ui.label(format!("{}: {}", t("Entries examined"), job.progress.count()));
                    if a11y::explain(ui.button(t("Cancel scan")), &t("Request cancellation at the next filesystem checkpoint. Partial results are kept and clearly labeled; a blocked operating-system call may take time to return.")).clicked() { job.cancel(); }
                    ctx.request_repaint_after(Duration::from_millis(150));
                }
            });
            if let Some(error) = &self.storage.error { ui.colored_label(theme::BAD, error); }
            if busy { ui.label(t("Previous results remain below while the new scan runs.")); }
            if let Some(report) = &self.storage.report {
                ui.separator();
                ui.add(egui::Label::new(format!("{} {}", t("Report for:"), report.root.display())).selectable(true));
                let status = if report.cancelled { t("Cancelled — partial results") } else if report.incomplete() { t("Partial — some entries were not measured") } else { t("Scan finished — measured entries only") };
                ui.label(RichText::new(status).strong());
                ui.label(format!("{}: {} · {}: {} · {}: {} · {}: {} · {}: {}", t("Logical size measured"), human_size(report.totals.bytes), t("Files"), report.totals.files, t("Folders"), report.totals.folders, t("Skipped links / reparse points"), report.totals.skipped_links, t("Errors / limits"), report.errors));
                ui.label(format!("{} {} / {}. {}", t("Largest immediate entries shown:"), report.rows.len(), report.root_entries, t("Summary totals include all measured entries. Folders include their scanned descendants; partial rows are lower bounds, not zero-size assurances.")));
                if report.rows.is_empty() { ui.label(t("No entries measured. The folder may be empty, or the scan was cancelled or incomplete.")); }
                egui::ScrollArea::vertical().id_salt("storage_results").max_height((ui.available_height() - 100.0).max(100.0)).show(ui, |ui| {
                    for row in &report.rows {
                        ui.separator();
                        ui.label(format!("{} · {} · {}", human_size(row.totals.bytes), if row.folder { t("Folder") } else { t("Entry") }, if row.incomplete { t("Partial / not fully measured") } else { t("Measured") }));
                        ui.add(egui::Label::new(row.path.to_string_lossy()).selectable(true));
                    }
                });
                if !report.error_details.is_empty() {
                    let details = egui::CollapsingHeader::new(t("Scan errors and limits")).show(ui, |ui| {
                        ui.label(t("First 100 details shown. Summary counts include all errors. Check access, close unavailable network locations, or choose a narrower folder and retry."));
                        egui::ScrollArea::vertical().id_salt("storage_errors").max_height(150.0).show(ui, |ui| {
                            for error in &report.error_details { ui.add(egui::Label::new(error).selectable(true)); }
                        });
                    });
                    a11y::explain(details.header_response, &t("Expand specific paths and operating-system errors; partial results must not be treated as a complete scan."));
                }
            } else if !busy { ui.label(t("Choose a folder and scan to see its largest immediate files and subfolders. Nothing is scanned automatically.")); }
        });
        if start {
            self.start_storage_scan(ctx);
        }
    }
}
