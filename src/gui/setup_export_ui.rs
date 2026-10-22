//! Explicit local authoring, separate from read-only machine diagnostics.
use super::{
    a11y, theme,
    widgets::{pick_file, save_file},
    Job,
};
use setupscope_core::{
    i18n::t,
    setup_export::{self, Inventory},
};
use std::{collections::BTreeSet, path::PathBuf, time::Duration};

#[derive(Default)]
pub struct SetupExportState {
    inventory: Option<Inventory>,
    source: Option<PathBuf>,
    selected: BTreeSet<usize>,
    preview: Option<String>,
    reviewed: bool,
    filter: String,
    load_job: Option<Job<Result<(Inventory, PathBuf), String>>>,
    save_job: Option<Job<Result<PathBuf, String>>>,
    error: Option<String>,
    message: Option<String>,
}
impl SetupExportState {
    fn invalidate_preview(&mut self) {
        self.preview = None;
        self.reviewed = false;
        self.message = None;
    }
    fn ids(&self) -> Vec<String> {
        self.inventory
            .as_ref()
            .map(|inv| {
                self.selected
                    .iter()
                    .filter_map(|i| inv.packages.get(*i))
                    .map(|p| p.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.load_job.as_ref().and_then(Job::poll) {
            self.load_job = None;
            match result {
                Ok((inventory, path)) => {
                    self.inventory = Some(inventory);
                    self.source = Some(path);
                    self.selected.clear();
                    self.filter.clear();
                    self.invalidate_preview();
                    self.error = None;
                }
                Err(error) => {
                    self.error = Some(format!(
                        "{error}\n{}",
                        t("Any previous inventory and selection are retained.")
                    ))
                }
            }
        }
        if let Some(result) = self.save_job.as_ref().and_then(Job::poll) {
            self.save_job = None;
            match result {
                Ok(path) => {
                    self.message = Some(format!(
                        "{}: {}",
                        t("Configuration saved; nothing applied"),
                        path.display()
                    ));
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        let busy = self.load_job.is_some() || self.save_job.is_some();
        ui.heading(t("Reviewed setup export"));
        ui.label(t("Author a WinGet configuration from an existing WinGet export JSON. Nothing is scanned, executed, downloaded or applied here. Importing reads only the chosen file; saving writes only a new file you choose."));
        ui.label(t("Inventory IDs and source details are unverified declarations, not proof of what is installed on this PC. Only the declared default winget community source is supported; other sources are skipped. Verify the target PC's actual winget source before applying elsewhere."));
        ui.label(t("Applying the resulting file manually can download DSC modules and installers and change a PC. Versions, scope, channels, app settings and custom switches are not reproduced. Packages/modules are not pinned; this is not a reproducible lockfile. Review package licenses and prerequisites separately."));
        ui.label(t("Create an inventory yourself with: winget export -o inventory.json --source winget\nSetupScope never runs this command. WinGet may contact its source and ask for agreements."));
        if a11y::explain(ui.button(t("Copy inventory export command")), &t("Copy the inventory command to your local clipboard for manual use in your own terminal. This does not run it or accept source agreements.")).clicked() { ui.ctx().copy_text("winget export -o inventory.json --source winget".into()); }
        if a11y::explain(ui.add_enabled(!busy, egui::Button::new(t("Load WinGet inventory JSON…"))), &t("Choose an existing UTF-8 WinGet export, at most 2 MiB and 500 supported entries. No commands run. A successful load clears selection and preview; a failed load retains the previous inventory.")).clicked() {
            if let Some(path) = pick_file(&t("Choose existing WinGet inventory"), &t("WinGet inventory JSON"), &["json"]) {
                self.error = None;
                self.load_job = Some(Job::spawn(ui.ctx(), move |_| setup_export::load(&path).map(|inv| (inv, path)).map_err(|e| format!("{e:#}"))));
            }
        }
        if busy {
            ui.spinner();
            ui.label(t("Reading or saving the selected local file…"));
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::BAD, error);
        }
        if let Some(message) = &self.message {
            ui.add(egui::Label::new(message).wrap().selectable(true));
        }
        if let Some(source) = &self.source {
            ui.add(
                egui::Label::new(format!(
                    "{}: {}",
                    t("Loaded inventory (unverified)"),
                    source.display()
                ))
                .wrap()
                .selectable(true),
            );
        }
        let Some(inventory) = &self.inventory else {
            ui.label(t("Load an inventory, select up to 50 package IDs, then preview and review before saving. Nothing is selected automatically."));
            return;
        };
        ui.label(format!(
            "{}: {} · {}: {} · {}: {}",
            t("Supported IDs"),
            inventory.packages.len(),
            t("Skipped sources"),
            inventory.skipped_sources,
            t("Duplicate entries omitted"),
            inventory.duplicates
        ));
        ui.add_enabled_ui(!busy, |ui| {
            let label = ui.label(t("Filter package IDs"));
            a11y::explain(ui.add(egui::TextEdit::singleline(&mut self.filter).hint_text(t("e.g. Git or SDK")).char_limit(128)).labelled_by(label.id), &t("Filter imported package IDs without changing selections. Hidden selections remain included in the preview."));
        });
        let filter = self.filter.to_ascii_lowercase();
        let mut changed = false;
        ui.add_enabled_ui(!busy, |ui| {
            egui::ScrollArea::vertical().id_salt("setup_export_packages").max_height(220.0).show(ui, |ui| {
                for (index, package) in inventory.packages.iter().enumerate() {
                    if !package.id.to_ascii_lowercase().contains(&filter) { continue; }
                    let mut selected = self.selected.contains(&index);
                    let label = format!("{}{}", package.id, package.version.as_ref().map(|v| format!(" — {}: {v}", t("inventory version, not pinned"))).unwrap_or_default());
                    if a11y::explain(ui.add_enabled(selected || self.selected.len() < setup_export::MAX_SELECTED, egui::Checkbox::new(&mut selected, label)), &t("Include this package ID in the configuration preview. This never installs it. Up to 50 packages can be selected; clear another selection to add more.")).changed() {
                        if selected { self.selected.insert(index); } else { self.selected.remove(&index); }
                        changed = true;
                    }
                }
            });
        });
        if changed {
            self.invalidate_preview();
        }
        ui.label(format!(
            "{}: {} / 50",
            t("Selected package IDs"),
            self.selected.len()
        ));
        if a11y::explain(
            ui.add_enabled(
                !busy && !self.selected.is_empty(),
                egui::Button::new(t("Clear package selection")),
            ),
            &t("Untick the current package IDs and discard their preview. No file is changed."),
        )
        .clicked()
        {
            self.selected.clear();
            self.invalidate_preview();
        }
        if a11y::explain(ui.add_enabled(!busy && !self.selected.is_empty(), egui::Button::new(t("Preview setup configuration"))), &t("Generate a bounded YAML preview from the selected IDs only. Imported scripts, custom switches and arbitrary settings are never copied.")).clicked() {
            match setup_export::configuration(&self.ids()) { Ok(preview) => { self.preview = Some(preview); self.reviewed = false; self.error = None; }, Err(error) => self.error = Some(error.to_string()) }
        }
        if let Some(preview) = &self.preview {
            egui::ScrollArea::both()
                .id_salt("setup_export_preview")
                .max_height(240.0)
                .show(ui, |ui| {
                    ui.add(
                        egui::Label::new(egui::RichText::new(preview).monospace()).selectable(true),
                    );
                });
            if a11y::explain(ui.button(t("Copy configuration preview")), &t("Copy the displayed configuration, including selected package IDs, to the local clipboard. Nothing is applied or uploaded.")).clicked() { ui.ctx().copy_text(preview.clone()); }
            a11y::explain(ui.add_enabled(!busy, egui::Checkbox::new(&mut self.reviewed, t("I reviewed this preview and understand applying it can change a PC"))), &t("Acknowledge review of this exact selection before saving. Changing the selection clears this acknowledgment. This does not authorize SetupScope to run it."));
            if save_button(ui, self.reviewed, busy).clicked() {
                if let Some(path) = save_file(
                    &t("Save new setup configuration"),
                    "configuration.winget",
                    &t("WinGet configuration"),
                    "winget",
                ) {
                    let ids = self.ids();
                    self.error = None;
                    self.save_job = Some(Job::spawn(ui.ctx(), move |_| {
                        setup_export::save_new(&path, &ids)
                            .map(|_| path)
                            .map_err(|e| format!("{e:#}"))
                    }));
                }
            }
        }
    }
}
fn save_button(ui: &mut egui::Ui, reviewed: bool, busy: bool) -> egui::Response {
    a11y::explain(ui.add_enabled(reviewed && !busy, egui::Button::new(t("Save new configuration…"))), &t("Save the reviewed YAML to a new chosen file. Existing files are never overwritten, even if the file picker offers replacement. Choose a new name if it exists. No configuration is run."))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changing_selection_invalidates_review_and_preview() {
        let mut state = SetupExportState {
            reviewed: true,
            preview: Some("old".into()),
            ..Default::default()
        };
        state.invalidate_preview();
        assert!(!state.reviewed && state.preview.is_none());
    }
    #[test]
    fn saving_requires_review_and_accessible_guidance() {
        for (reviewed, busy) in [(false, false), (true, false), (true, true)] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let render = || {
                ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        save_button(ui, reviewed, busy);
                    });
                })
            };
            let _ = render();
            let output = render();
            let update = output.platform_output.accesskit_update.unwrap();
            let node = update
                .nodes
                .iter()
                .find(|(_, n)| n.role() == egui::accesskit::Role::Button)
                .unwrap();
            assert_eq!(node.1.is_disabled(), !reviewed || busy);
            assert!(node.1.description().unwrap().contains("never overwritten"));
        }
    }
    #[test]
    fn keyboard_save_activates_only_after_review() {
        for (reviewed, busy) in [(false, false), (true, false), (true, true)] {
            let ctx = egui::Context::default();
            let frame = |events| {
                let mut clicked = false;
                let _ = ctx.run(
                    egui::RawInput {
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            clicked = save_button(ui, reviewed, busy).clicked();
                        });
                    },
                );
                clicked
            };
            let key = |key| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::default(),
            };
            frame(vec![]);
            frame(vec![key(egui::Key::Tab)]);
            assert_eq!(frame(vec![key(egui::Key::Enter)]), reviewed && !busy);
        }
    }
}
