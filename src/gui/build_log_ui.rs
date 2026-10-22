use super::{a11y, widgets, Job};
use setupscope_core::build_log;
#[derive(Default)]
pub struct BuildLogState {
    path: String,
    job: Option<Job<Result<String, String>>>,
    report: Option<String>,
    report_path: Option<String>,
    error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_reports_expand_a_compact_window_instead_of_collapsing_the_preview() {
        for height in [220., 360.] {
            let ctx = egui::Context::default();
            super::super::theme::apply(&ctx, false);
            let mut state = BuildLogState::default();
            state.report = Some("Read-only diagnostic evidence\n".repeat(40));
            state.report_path = Some(String::new());
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(720., height),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        super::super::tool_window::scroll_body(ui, "report-growth", |ui| {
                            state.ui(ui)
                        })
                    });
                    super::super::tool_window::fit_content(ctx, super::super::app::Tab::BuildLog);
                },
            );
            assert!(
                output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .iter()
                    .any(|c| matches!(c,egui::ViewportCommand::InnerSize(size) if size.y>=450.)),
                "Long evidence was constrained by the old compact height"
            );
        }
    }
}
impl Drop for BuildLogState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}
impl BuildLogState {
    pub(super) fn select_path(&mut self, path: std::path::PathBuf) {
        if self.job.is_none() {
            self.path = path.display().to_string();
        }
    }
    pub(super) fn accept_evidence(&mut self, report: &build_log::Report) -> bool {
        if self.job.is_some() {
            return false;
        }
        self.path = report.path.display().to_string();
        self.report = Some(report.text());
        self.report_path = Some(self.path.clone());
        self.error = None;
        true
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Ok(text) => {
                    self.report = Some(text);
                    self.report_path = Some(self.path.clone());
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        super::widgets::scope_note(ui, "Inspect one saved log. Matches are hints; nothing runs or changes.", "Select one saved local build log. Up to 2 MiB, 20,000 lines and 8 KiB per line; UTF-8 or BOM-marked UTF-16. Oversized files are rejected; long/excess lines are explicitly skipped. No build, script or suggested command runs. Nothing is modified or uploaded. Offline pattern hints for Rust, MSVC/MSBuild, .NET/NuGet, TypeScript/npm, Python, CMake and common storage/access failures. Quoted text can match; findings are not confirmed causes. Raw log lines are omitted; use the source line numbers in your editor. This report is not included in diagnostic bundles.");

        ui.add_enabled_ui(self.job.is_none(), |ui| {
            let label = ui.label(setupscope_core::i18n::t("Saved build-log path"));
            a11y::explain(ui.add(egui::TextEdit::singleline(&mut self.path).char_limit(4096).desired_width(ui.available_width().min(600.0))).labelled_by(label.id), "Absolute local regular text file; network and reparse paths are skipped. Selecting a new path does not alter previous dated evidence.");
            ui.horizontal_wrapped(|ui| {
            if a11y::explain(ui.button(setupscope_core::i18n::t("Choose build log…")), "Select one existing text log; selection alone does not read it.").clicked() {
                if let Some(path) = widgets::pick_file("Choose saved build log", "Build logs", &["log","txt","out"]) {
                    if let Some(path) = path.to_str() { self.path = path.to_owned(); }
                    else { self.error = Some("The selected path is not representable as Unicode.".into()); }
                }
            }
            if a11y::explain(widgets::action_button(ui,!self.path.trim().is_empty(),"Triage saved build log"), "Read bounded log bytes in the background, group recognized diagnostic hints and show original line numbers. No commands execute.").clicked() {
                let path = std::path::PathBuf::from(&self.path);
                self.error = None;
                self.job = Some(Job::spawn(ui.ctx(), move |progress| {
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| build_log::inspect(&path, progress).map(|r| r.text()).map_err(|e|format!("{e:#}"))))
                        .unwrap_or_else(|_| Err("Log triage failed unexpectedly. Previous evidence retained.".into()))
                }));
            }
            });
        });
        if let Some(job) = &self.job {
            ui.spinner();
            ui.label(setupscope_core::i18n::t(&format!(
                "{} lines inspected. Previous report may be stale.",
                job.progress.count()
            )));
            if a11y::explain(
                ui.button(setupscope_core::i18n::t("Cancel log triage")),
                "Cancel at the next line checkpoint; a filesystem read must return first.",
            )
            .clicked()
            {
                job.cancel();
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
        }
        if let Some(error) = &self.error {
            ui.label(setupscope_core::i18n::t(&format!(
                "{error} Previous report, if any, is retained."
            )));
        }
        if let Some(report) = &self.report {
            if self.report_path.as_deref() != Some(self.path.as_str()) {
                super::theme::warning_banner(ui, "Input changed: the retained report is for the earlier source path. Triage the selected log to refresh it.");
            }
            if a11y::explain(ui.button(setupscope_core::i18n::t("Copy triage report (contains source path)")), "Copy grouped hints and line numbers. No raw log lines are included, but the source path is unredacted; review before sharing.").clicked() { ui.ctx().copy_text(report.clone()); }
            egui::ScrollArea::vertical()
                .id_salt("build_log_report")
                .max_height(300.0)
                .min_scrolled_height(300.0)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(report).selectable(true));
                });
        }
    }
}
