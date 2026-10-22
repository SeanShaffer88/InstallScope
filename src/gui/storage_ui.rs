//! Read-only storage survey. No deletion or automatic opening of discovered files.
use super::{
    a11y,
    app::{App, Confirm},
    theme,
    widgets::pick_folder,
    Job,
};
use egui::RichText;
use setupscope_core::{
    i18n::{t, tf},
    model::human_size,
    storage::{self, StorageReport},
    storage_baseline::{self, Baseline},
};
use std::time::Duration;

type BaselineResult = Result<(Option<Baseline>, String), String>;

fn allocation_option(ui: &mut egui::Ui, enabled: &mut bool, busy: bool) {
    a11y::explain(ui.add_enabled(!busy, egui::Checkbox::new(enabled, t("Measure allocation and hard links (slower)"))), &t("Optional Windows metadata queries for each file; no contents are read. Track up to 100,000 identities and 200,000 queries across the selected tree. Unsupported or inaccessible files stay unknown. Off by default; not saved between launches. Existing results change only after another scan."));
}

#[derive(Default)]
pub struct StorageState {
    include_allocation: bool,
    root: String,
    job: Option<Job<Result<StorageReport, String>>>,
    report: Option<StorageReport>,
    error: Option<String>,
    baseline: Option<Baseline>,
    baseline_job: Option<Job<BaselineResult>>,
    baseline_error: Option<String>,
    baseline_message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, Vec2};

    fn key(key: Key) -> Vec<Event> {
        vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]
    }

    fn frame(
        ctx: &egui::Context,
        can_save: bool,
        busy: bool,
        events: Vec<Event>,
    ) -> (egui::FullOutput, Option<BaselineAction>) {
        let mut action = None;
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 300.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = baseline_controls(ui, can_save, busy);
                });
            },
        );
        (output, action)
    }

    #[test]
    fn baseline_controls_work_by_keyboard_and_skip_unavailable_save() {
        for (can_save, tabs, expected) in [
            (true, 1, BaselineAction::Save),
            (true, 2, BaselineAction::Load),
            (true, 3, BaselineAction::Delete),
            (false, 1, BaselineAction::Load),
        ] {
            let ctx = egui::Context::default();
            frame(&ctx, can_save, false, vec![]);
            for _ in 0..tabs {
                frame(&ctx, can_save, false, key(Key::Tab));
            }
            assert_eq!(
                frame(&ctx, can_save, false, key(Key::Enter)).1,
                Some(expected)
            );
        }
        let ctx = egui::Context::default();
        frame(&ctx, true, true, vec![]);
        frame(&ctx, true, true, key(Key::Tab));
        assert_eq!(frame(&ctx, true, true, key(Key::Enter)).1, None);
    }

    #[test]
    fn baseline_controls_expose_guidance_and_busy_state_to_accesskit() {
        for busy in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            frame(&ctx, true, busy, vec![]);
            let update = frame(&ctx, true, busy, vec![])
                .0
                .platform_output
                .accesskit_update
                .unwrap();
            let buttons: Vec<_> = update
                .nodes
                .iter()
                .map(|(_, node)| node)
                .filter(|node| node.role() == egui::accesskit::Role::Button)
                .collect();
            assert_eq!(buttons.len(), 3);
            assert!(buttons.iter().all(|node| node.description().is_some()));
            assert!(buttons.iter().all(|node| node.is_disabled() == busy));
        }
    }

    #[test]
    fn size_change_text_preserves_direction_and_full_unsigned_range() {
        assert!(change_label(i128::from(u64::MAX)).starts_with("Increased"));
        assert!(change_label(-i128::from(u64::MAX)).starts_with("Decreased"));
        assert_eq!(change_label(0), "No logical size change");
    }

    #[test]
    fn accounting_option_is_opt_in_accessible_and_disabled_during_scan() {
        for busy in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut enabled = false;
            let mut render = |events| {
                ctx.run(
                    RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default()
                            .show(ctx, |ui| allocation_option(ui, &mut enabled, busy));
                    },
                )
            };
            let _ = render(vec![]);
            let output = render(vec![]);
            let update = output.platform_output.accesskit_update.unwrap();
            let checkbox = update
                .nodes
                .iter()
                .find(|(_, node)| node.role() == egui::accesskit::Role::CheckBox)
                .unwrap();
            assert_eq!(checkbox.1.is_disabled(), busy);
            assert!(checkbox
                .1
                .description()
                .unwrap()
                .contains("200,000 queries"));
            let _ = render(key(Key::Tab));
            let _ = render(key(Key::Space));
            assert_eq!(enabled, !busy);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BaselineAction {
    Save,
    Load,
    Delete,
}

fn baseline_controls(ui: &mut egui::Ui, can_save: bool, busy: bool) -> Option<BaselineAction> {
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        if a11y::explain(ui.add_enabled(!busy && can_save, egui::Button::new(t("Save storage baseline…"))), &t("Confirm saving this complete scan locally, replacing the previous valid baseline. Finish a scan without cancellation, errors, or skipped links first. Private paths are included.")).clicked() { action = Some(BaselineAction::Save); }
        if a11y::explain(ui.add_enabled(!busy, egui::Button::new(t("Load storage baseline"))), &t("Read the one saved storage baseline. No folder is scanned and no scanned file is changed. Wait for the current storage operation to finish.")).clicked() { action = Some(BaselineAction::Load); }
        if a11y::explain(ui.add_enabled(!busy, egui::Button::new(t("Delete storage baseline…"))), &t("Confirm permanently deleting only storage-baseline.json in the data folder. Scanned files and the current scan report are not touched.")).clicked() { action = Some(BaselineAction::Delete); }
    });
    action
}

fn change_label(change: i128) -> String {
    if change > 0 {
        tf(
            "Increased by {size}",
            &[("size", human_size(change as u64))],
        )
    } else if change < 0 {
        tf(
            "Decreased by {size}",
            &[("size", human_size((-change) as u64))],
        )
    } else {
        t("No logical size change")
    }
}
impl Drop for StorageState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}
impl App {
    pub fn save_storage_baseline(&mut self, baseline: Baseline) {
        if self.storage.job.is_some() || self.storage.baseline_job.is_some() {
            return;
        }
        let root = self.store.root.clone();
        self.storage.baseline_error = None;
        self.storage.baseline_message = None;
        self.storage.baseline_job = Some(Job::spawn(&self.ctx, move |_| {
            storage_baseline::save(&root, &baseline)
                .map(|()| (Some(baseline), t("Storage baseline saved locally. The previous valid baseline was replaced.")))
                .map_err(|error| format!("{error:#}"))
        }));
    }

    pub fn delete_storage_baseline(&mut self) {
        if self.storage.job.is_some() || self.storage.baseline_job.is_some() {
            return;
        }
        let root = self.store.root.clone();
        self.storage.baseline_error = None;
        self.storage.baseline_message = None;
        self.storage.baseline_job = Some(Job::spawn(&self.ctx, move |_| {
            storage_baseline::delete(&root)
                .map(|()| (None, t("Saved storage baseline deleted. Scanned files and current scan results were not changed.")))
                .map_err(|error| format!("{error:#}"))
        }));
    }

    pub fn start_storage_scan(&mut self, ctx: &egui::Context) {
        if self.storage.job.is_some() || self.storage.baseline_job.is_some() {
            return;
        }
        if self.storage.root.trim().is_empty() {
            self.storage.error = Some(t("Choose a folder or enter a drive/folder path first."));
            return;
        }
        let root = std::path::PathBuf::from(&self.storage.root);
        let include_allocation = self.storage.include_allocation;
        self.storage.error = None;
        self.storage.job = Some(Job::spawn(ctx, move |progress| {
            storage::scan_with_allocation(&root, progress, include_allocation)
                .map_err(|e| format!("{e:#}"))
        }));
    }

    pub fn ui_storage(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.storage.baseline_job.as_ref().and_then(Job::poll) {
            self.storage.baseline_job = None;
            match result {
                Ok((baseline, message)) => {
                    self.storage.baseline = baseline;
                    self.storage.baseline_message = Some(message);
                    self.storage.baseline_error = None;
                }
                Err(error) => self.storage.baseline_error = Some(error),
            }
        }
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
        let busy = self.storage.job.is_some() || self.storage.baseline_job.is_some();
        let mut start = false;
        let mut navigate = None;
        let mut baseline_action = None;
        let comparison = self
            .storage
            .baseline
            .as_ref()
            .zip(self.storage.report.as_ref())
            .map(|(baseline, report)| {
                baseline
                    .compare(report)
                    .map_err(|error| format!("{error:#}"))
            });
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
          egui::ScrollArea::vertical().id_salt("storage_page").show(ui, |ui| {
            a11y::focus_hint(ui, theme::MUTED);
            ui.label(t("Find where space goes before deciding what to remove. This page only reads filesystem metadata; it never deletes files."));
            ui.label(t("Sizes are logical file lengths, not physical allocation or guaranteed recoverable space. Hard-linked files may count more than once. Filesystem activity can change the results during scanning."));
            allocation_option(ui, &mut self.storage.include_allocation, busy);
            ui.label(t("Optional allocation accounting applies to the next scan and can take longer. Rankings and saved baseline comparisons always use logical lengths; allocation results are not saved in the baseline."));
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
            ui.separator();
            ui.label(t("Optional storage baseline: one local record, kept until replaced or deleted. It contains private paths and sizes, is not encrypted, and is not a backup. Saving or loading never changes scanned files."));
            let can_save = self.storage.report.as_ref().is_some_and(|report| !report.incomplete());
            baseline_action = baseline_controls(ui, can_save, busy);
            if self.storage.baseline_job.is_some() { ui.spinner(); ctx.request_repaint_after(Duration::from_millis(150)); }
            if let Some(error) = &self.storage.baseline_error { ui.colored_label(theme::BAD, error); }
            if let Some(message) = &self.storage.baseline_message { ui.label(message); }
            if let Some(baseline) = &self.storage.baseline {
                ui.label(format!("{} {} UTC", t("Loaded baseline saved:"), setupscope_core::time::DateTime::from_unix_utc(baseline.saved_at).ymd_hm()));
                ui.label(format!("{} {} UTC", t("Baseline scan started:"), setupscope_core::time::DateTime::from_unix_utc(baseline.scanned_at).ymd_hm()));
                ui.add(egui::Label::new(format!("{} {}", t("Baseline folder:"), baseline.root)).selectable(true));
            }
            if let Some(comparison) = &comparison {
                match comparison {
                    Ok(compared) => {
                        ui.label(RichText::new(format!("{}: {}", t("Logical size since baseline"), change_label(compared.total_change))).strong());
                        ui.label(t("Only matching names and entry types in both top-200 lists have row comparisons. A missing match is unknown, not proof of a new or deleted file. These are live metadata surveys, not filesystem snapshots."));
                        ui.label(tf("Baseline entries without a current match: {n}", &[("n", compared.unmatched_baseline_rows.to_string())]));
                    }
                    Err(error) => { ui.colored_label(theme::WARN, error); }
                }
            }
            if self.storage.job.is_some() { ui.label(t("Previous results remain below while the new scan runs.")); }
            if let Some(report) = &self.storage.report {
                ui.separator();
                ui.add(egui::Label::new(format!("{} {}", t("Report for:"), report.root.display())).selectable(true));
                ui.label(format!("{} {} UTC", t("Current scan started:"), setupscope_core::time::DateTime::from_unix_utc(report.scanned_at).ymd_hm()));
                ui.add_enabled_ui(!busy, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        let parent = report.root.parent().filter(|p| !p.as_os_str().is_empty());
                        if a11y::explain(ui.add_enabled(parent.is_some(), egui::Button::new(t("Scan parent folder"))), &t("Scan one level above the displayed report. This explicitly expands the scan scope and may take longer. No files are changed.")).clicked() {
                            navigate = parent.map(std::path::Path::to_path_buf);
                        }
                        if a11y::explain(ui.button(t("Rescan this folder")), &t("Refresh the displayed folder using current filesystem metadata. Previous results remain visible until the scan finishes.")).clicked() {
                            navigate = Some(report.root.clone());
                        }
                    });
                });
                let status = if report.cancelled { t("Cancelled — partial results") } else if report.incomplete() { t("Partial — some entries were not measured") } else { t("Scan finished — measured entries only") };
                ui.label(RichText::new(status).strong());
                ui.label(format!("{}: {} · {}: {} · {}: {} · {}: {} · {}: {}", t("Logical size measured"), human_size(report.totals.bytes), t("Files"), report.totals.files, t("Folders"), report.totals.folders, t("Skipped links / reparse points"), report.totals.skipped_links, t("Errors / limits"), report.errors));
                if let Some(allocation) = &report.allocation {
                    ui.separator();
                    ui.label(RichText::new(t(if report.incomplete() || allocation.partial() { "Partial allocation / hard-link accounting" } else { "Allocation / hard-link accounting for observed files" })).strong());
                    ui.label(format!("{}: {} · {}: {}", t("Unique logical bytes measured"), human_size(allocation.unique_logical_bytes), t("Filesystem-reported allocation measured"), human_size(allocation.allocated_bytes)));
                    ui.label(format!("{}: {} · {}: {} · {}: {}", t("Unique file identities"), allocation.unique_files, t("Repeated hard-link entries not counted twice"), allocation.repeated_links, t("Files with unknown accounting"), allocation.unknown_files));
                    if allocation.limit_reached { ui.colored_label(theme::WARN, t("Accounting limit reached; remaining files are unknown. Choose a smaller folder for more coverage.")); }
                    if allocation.overflow { ui.colored_label(theme::WARN, t("Accounting overflow; totals are incomplete.")); }
                    if !allocation.unknown_details.is_empty() {
                        let details = egui::CollapsingHeader::new(t("Why some allocation measurements are unknown (first 10)")).show(ui, |ui| {
                            for detail in &allocation.unknown_details { ui.add(egui::Label::new(detail).wrap().selectable(true)); }
                        });
                        a11y::explain(details.header_response, &t("Expand the first ten failed metadata queries and their paths. No files are opened or changed by expanding these details."));
                    }
                    ui.label(t("Measured subset only. Unreadable, unsupported, changing or skipped files remain unknown. Identity deduplication applies across the selected tree; folder rankings and saved baseline comparisons still use logical file lengths. Allocation excludes directory metadata, alternate streams and other volumes, and is not unique physical usage or guaranteed recoverable space. Links outside this tree, compression, sparse files, shared blocks and filesystem activity can affect interpretation."));
                }
                ui.label(format!("{} {} / {}. {}", t("Largest immediate entries shown:"), report.rows.len(), report.root_entries, t("Summary totals include all measured entries. Folders include their scanned descendants; partial rows are lower bounds, not zero-size assurances.")));
                if report.rows.is_empty() { ui.label(t("No entries measured. The folder may be empty, or the scan was cancelled or incomplete.")); }
                egui::ScrollArea::vertical().id_salt("storage_results").max_height((ui.available_height() - 100.0).max(100.0)).show(ui, |ui| {
                    for (index, row) in report.rows.iter().enumerate() {
                        ui.separator();
                        ui.label(format!("{} · {} · {}", human_size(row.totals.bytes), if row.folder { t("Folder") } else { t("Entry") }, if row.incomplete { t("Partial / not fully measured") } else { t("Measured") }));
                        ui.add(egui::Label::new(row.path.to_string_lossy()).selectable(true));
                        if let Some(Ok(compared)) = &comparison {
                            ui.label(compared.row_changes[index].map(change_label).unwrap_or_else(|| t("Previous size unknown — no matching baseline row")));
                        }
                        if row.folder && a11y::explain(ui.add_enabled(!busy, egui::Button::new(t("Scan inside folder")).small()), &tf("Scan inside {folder} to see its largest immediate entries. The folder is checked again; links and reparse points are refused.", &[("folder", row.path.to_string_lossy().into_owned())])).clicked() {
                            navigate = report.folder_at(index).map(std::path::Path::to_path_buf);
                        }
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
        });
        match baseline_action {
            Some(BaselineAction::Save) => {
                if let Some(report) = &self.storage.report {
                    match Baseline::from_report(report) {
                        Ok(baseline) => {
                            let body = tf("Save the displayed scan of {folder} to storage-baseline.json in SetupScope's data folder? This saves private paths and logical sizes and replaces the previous valid baseline, including a baseline for another folder. Damaged or unsupported saved data will not be overwritten. No scanned files are changed and no network is used.", &[("folder", baseline.root.clone())]);
                            self.ask("Save storage baseline?", body, "Save baseline", Confirm::SaveStorageBaseline(baseline));
                        }
                        Err(error) => self.storage.baseline_error = Some(format!("{error:#}")),
                    }
                }
            }
            Some(BaselineAction::Load) => {
                let root = self.store.root.clone();
                self.storage.baseline_error = None;
                self.storage.baseline_message = None;
                self.storage.baseline_job = Some(Job::spawn(ctx, move |_| {
                    storage_baseline::load(&root).map(|baseline| {
                        let message = if baseline.is_some() { t("Storage baseline loaded. Scan the same folder to compare.") } else { t("No saved storage baseline. Finish a complete scan and save it to keep a baseline between launches.") };
                        (baseline, message)
                    }).map_err(|error| format!("{error:#}"))
                }));
            }
            Some(BaselineAction::Delete) => self.ask("Delete storage baseline?", t("Permanently delete storage-baseline.json from SetupScope's data folder? This cannot be undone. Current scan results and every scanned file remain unchanged."), "Delete baseline", Confirm::DeleteStorageBaseline),
            None => {}
        }
        if let Some(path) = navigate {
            self.storage.root = path.to_string_lossy().into_owned();
            self.start_storage_scan(ctx);
        } else if start {
            self.start_storage_scan(ctx);
        }
    }
}
