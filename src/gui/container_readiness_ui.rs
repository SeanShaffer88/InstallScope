use super::{a11y, Job};
use setupscope_core::container_readiness;
#[derive(Default)]
pub struct ContainerState {
    job: Option<Job<Result<container_readiness::Report, String>>>,
    text: Option<String>,
    error: Option<String>,
}
impl Drop for ContainerState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}
impl ContainerState {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Ok(report) => {
                    self.text = Some(report.text());
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        super::widgets::scope_note(ui,"Passive host evidence only. Runtime health is unverified; no commands or system changes.",container_readiness::LIMITATIONS);
        if a11y::explain(super::widgets::action_button(ui,self.job.is_none(),"Inspect WSL/container host readiness"), "Read bounded local CLI metadata, service status and processor hints. No engine connection, command execution or system change.").clicked() {
            self.error = None;
            self.job = Some(Job::spawn(ui.ctx(), |progress| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| container_readiness::inspect(progress)))
                    .map_err(|_| "Inspection failed unexpectedly; previous report retained.".into())
            }));
        }
        if let Some(job) = &self.job {
            ui.spinner();
            ui.label(setupscope_core::i18n::t("Inspection running. Previous report below may be stale."));
            if a11y::explain(
                ui.button(setupscope_core::i18n::t("Cancel host readiness inspection")),
                "Cancel at the next checkpoint; OS metadata calls must return first.",
            )
            .clicked()
            {
                job.cancel();
            }
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(150));
        }
        if let Some(error) = &self.error {
            ui.colored_label(super::theme::BAD, setupscope_core::i18n::t(&format!("✖ {error}")));
        }
        if let Some(text) = &self.text {
            if a11y::explain(ui.button(setupscope_core::i18n::t("Copy host readiness report (contains local paths)")), "Explicit clipboard copy is unredacted. Review before sharing; diagnostic bundles do not include this report.").clicked() { ui.ctx().copy_text(text.clone()); }
            egui::ScrollArea::vertical()
                .id_salt("container_readiness_report")
                .max_height(360.0)
                .min_scrolled_height(360.0)
                .show(ui, |ui| {
                    ui.add(egui::Label::new(text.replace(container_readiness::LIMITATIONS, "Passive observations; runtime health remains unverified. Full scope is available above.")).wrap().selectable(true));
                });
        }
    }
}
