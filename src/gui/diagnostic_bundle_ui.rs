use super::{a11y, widgets};
use setupscope_core::{
    devcheck::Report,
    diagnostic_bundle::{self, Sections},
};

pub struct BundleState {
    sections: Sections,
    preview: Option<String>,
    reviewed: bool,
    status: String,
}
impl Default for BundleState {
    fn default() -> Self {
        Self {
            sections: Sections {
                runtime: true,
                readiness: false,
                tools: false,
            },
            preview: None,
            reviewed: false,
            status: String::new(),
        }
    }
}
impl BundleState {
    pub fn ui(&mut self, ui: &mut egui::Ui, report: Option<&Report>) {
        ui.label(setupscope_core::i18n::t("Local JSON bundle, at most 32 KiB. Paths, secrets and free-form source text are excluded by allowlist. No files are collected or uploaded."));
        ui.label(setupscope_core::i18n::t("Readiness sections use the retained Toolchain / PATH / Versions evidence; its original observation date is unavailable here. Reinspect first if needed. Log findings, other workspaces and recovery data are excluded."));
        let changed = a11y::explain(ui.checkbox(&mut self.sections.runtime, setupscope_core::i18n::t("Application runtime")), "Include app version, process architecture and OS family.").changed()
            | a11y::explain(ui.checkbox(&mut self.sections.readiness, setupscope_core::i18n::t("Build readiness summary")), "Include profile, target, status counts and partial/cancelled flags only.").changed()
            | a11y::explain(ui.checkbox(&mut self.sections.tools, setupscope_core::i18n::t("Known tool candidate counts")), "Include fixed built-in tool names and candidate counts; omit paths, versions and custom names.").changed();
        if changed {
            self.preview = None;
            self.reviewed = false;
            self.status.clear();
        }
        if a11y::explain(
            ui.button(setupscope_core::i18n::t("Generate redacted preview")),
            "Build an immutable preview from the selected sections. Does not write a file.",
        )
        .clicked()
        {
            self.reviewed = false;
            match diagnostic_bundle::preview(self.sections, report) {
                Ok(text) => {
                    self.preview = Some(text);
                    self.status.clear();
                }
                Err(e) => {
                    self.preview = None;
                    self.status = e.to_string();
                }
            }
        }
        if let Some(text) = &self.preview {
            egui::ScrollArea::vertical()
                .id_salt("bundle_preview")
                .max_height(260.0)
                .min_scrolled_height(260.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(text).monospace())
                            .wrap()
                            .selectable(true),
                    );
                });
            a11y::explain(ui.checkbox(&mut self.reviewed, setupscope_core::i18n::t("I reviewed this exact preview for sharing")), "Review remains tied to this generated snapshot; changing sections or generating another preview clears it.");
            if a11y::explain(ui.add_enabled(self.reviewed, egui::Button::new(setupscope_core::i18n::t("Save new diagnostic bundle…"))), "Save exactly the preview as a new JSON file. Existing files are never overwritten; no upload occurs.").clicked() {
                if let Some(path) = widgets::save_file("Save redacted diagnostic bundle", "SetupScope-diagnostics.json", "JSON", "json") {
                    self.status = match diagnostic_bundle::export(&path, text) {
                        Ok(()) => "Saved locally. Nothing uploaded.".into(),
                        Err(e) => format!("Save failed: {e:#}. Choose a new filename and an accessible folder."),
                    };
                }
            }
        }
        ui.label(setupscope_core::i18n::t(&self.status));
    }
}
