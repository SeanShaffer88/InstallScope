//! Read-only local endpoints and explicitly selected file ownership.
use super::{a11y, theme, widgets::pick_file, Job};
use setupscope_core::{
    devinspect::{self, Filter, LockReport, Page, PortReport},
    i18n::t,
};
use std::{path::PathBuf, time::Duration};
enum Update {
    Ports(PortReport),
    Locks(LockReport),
}
#[derive(Default)]
pub struct InspectorState {
    include_udp: bool,
    file: String,
    port_filter: String,
    pid_filter: String,
    port_page: usize,
    lock_page: usize,
    one_row: bool,
    ports: Option<PortReport>,
    locks: Option<LockReport>,
    job: Option<Job<Result<Update, String>>>,
    error: Option<String>,
}
impl Drop for InspectorState {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel();
        }
    }
}
#[derive(Default)]
struct Actions {
    ports: bool,
    browse: bool,
    locks: bool,
}
fn controls(ui: &mut egui::Ui, state: &mut InspectorState) -> Actions {
    let mut action = Actions::default();
    ui.add_enabled_ui(state.job.is_none(),|ui|{
        ui.horizontal_wrapped(|ui|{
            a11y::explain(ui.checkbox(&mut state.include_udp,t("Include UDP bound endpoints")),&t("TCP listener tables are always queried for IPv4 and IPv6. Optionally include UDP bindings; a bound UDP socket is not proof of a listening service or remote reachability."));
            action.ports=a11y::explain(super::widgets::action_button(ui,true,&t("Refresh local ports (read-only)")),&t("Read local IP Helper owner-PID tables and bounded current process metadata. No DNS lookup, socket connection, process termination or firewall change occurs. This is a dated snapshot, not a port reservation.")).clicked();
        });
        ui.separator();let label=ui.label(t("Explicit local file to inspect"));
        a11y::explain(ui.add(egui::TextEdit::singleline(&mut state.file).char_limit(32766)).labelled_by(label.id),&t("Literal absolute local drive file path without surrounding quotes. Directories, network/mapped drives, device paths and observed reparse links are rejected. No content is read, unlocked or deleted."));
        ui.horizontal_wrapped(|ui|{
            action.browse=a11y::explain(ui.button(t("Choose file…")),&t("Choose one file. Choosing alone never queries owners; explicitly Inspect selected file afterwards.")).clicked();
            action.locks=a11y::explain(super::widgets::action_button(ui,!state.file.is_empty(),&t("Inspect selected file owners")),&t("Register only this local file with a temporary Restart Manager query and compare reported PID/start times with current read-only process metadata. Never requests shutdown, restart, unlock or termination. Zero owners does not prove absence of locks.")).clicked();
        });
    });
    action
}
impl InspectorState {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        if let Some(result) = self.job.as_ref().and_then(Job::poll) {
            self.job = None;
            match result {
                Ok(Update::Ports(report)) => {
                    self.ports = Some(report);
                    self.port_page = 0;
                    self.error = None;
                }
                Ok(Update::Locks(report)) => {
                    self.locks = Some(report);
                    self.lock_page = 0;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
        super::widgets::scope_note(ui, "Explicit read-only queries. No process control or unlocking; ownership can be incomplete.", "Read-only local ports and selected-file owners. Scans start only when requested. No process control, unlocking, firewall edits, network service probes or automatic saving. Kernel/permission limits and PID reuse remain explicit. TCP/UDP tables and process metadata are read at different times. Current PID image metadata is not proof that the same process owned an endpoint earlier. Restart Manager covers only supported resource-owner cases.");

        let action = controls(ui, self);
        if action.browse {
            if let Some(path) = pick_file(
                &t("Choose a local file for read-only owner inspection"),
                "All files",
                &["*"],
            ) {
                if let Some(value) = path.to_str() {
                    self.file = value.into();
                } else {
                    self.error=Some(t("Selected file path contains unsupported Unicode; previous evidence retained."));
                }
            }
        } else if action.ports {
            let include_udp = self.include_udp;
            self.spawn(ui, move |progress| {
                devinspect::inspect_ports(include_udp, progress).map(Update::Ports)
            });
        } else if action.locks {
            let file = PathBuf::from(&self.file);
            self.spawn(ui, move |progress| {
                devinspect::inspect_file(&file, progress).map(Update::Locks)
            });
        }
        if let Some(job) = &self.job {
            ui.spinner();
            ui.label(setupscope_core::i18n::t(&format!(
                "{} · {} checkpoints",
                job.progress.phase(),
                job.progress.count()
            )));
            if a11y::explain(ui.button(t("Cancel ports/locks inspection")),&t("Stop at the next table/metadata checkpoint. An operating-system call may take time to return. Partial/cancelled results cannot prove a free port or unlocked file.")).clicked(){job.cancel();}
            ui.ctx().request_repaint_after(Duration::from_millis(150));
        }
        if let Some(error) = &self.error {
            ui.colored_label(theme::WARN, error);
            ui.label(t("Query failed or target unsupported. Any displayed previous report retains its original target and observation time; it is not a successful empty result for this request."));
        }
        if a11y::explain(ui.checkbox(&mut self.one_row,t("One row per page (for long paths)")),&t("Reduce report pages to one row instead of 50. Clipboard/display pages are bounded to 128 KiB and explicitly mark truncation. Paths are not redacted.")).changed(){self.port_page=0;self.lock_page=0;}
        if let Some(report) = &self.ports {
            ui.separator();
            ui.heading(t("Observed endpoint tables"));
            ui.label(t("Display filters do not refresh evidence. Changing Include UDP affects only the next explicit scan; the report lists the queried sources and times."));
            ui.horizontal_wrapped(|ui|{
                let before=(self.port_filter.clone(),self.pid_filter.clone());
                let label=ui.label(t("Exact port (optional)"));a11y::explain(ui.add(egui::TextEdit::singleline(&mut self.port_filter).char_limit(5).desired_width(90.0)).labelled_by(label.id),&t("Filter existing rows by decimal port 0–65535; blank shows all. Does not probe or reserve that port."));
                let label=ui.label(t("Exact PID (optional)"));a11y::explain(ui.add(egui::TextEdit::singleline(&mut self.pid_filter).char_limit(10).desired_width(110.0)).labelled_by(label.id),&t("Filter existing rows by numeric PID. PIDs can be reused; this is not a trusted process identity."));
                if before!=(self.port_filter.clone(),self.pid_filter.clone()){self.port_page=0;}
            });
            match Filter::parse(&self.port_filter, &self.pid_filter) {
                Ok(filter) => {
                    let page = report.page(filter, self.port_page, self.one_row);
                    self.port_page = page.page;
                    page_ui(ui, "ports", &mut self.port_page, page);
                }
                Err(error) => {
                    ui.colored_label(theme::WARN, error.to_string());
                }
            }
        }
        if let Some(report) = &self.locks {
            ui.separator();
            ui.heading(t("Selected-file owner observation"));
            if report.path.to_str() != Some(self.file.as_str()) {
                ui.colored_label(theme::WARN,t("Displayed evidence belongs to the earlier selected file. Inspect the new path to refresh; choosing alone did not query it."));
            }
            let page = report.page(self.lock_page, self.one_row);
            self.lock_page = page.page;
            page_ui(ui, "locks", &mut self.lock_page, page);
        }
    }
    fn spawn(
        &mut self,
        ui: &egui::Ui,
        operation: impl FnOnce(&setupscope_core::snapshot::Progress) -> anyhow::Result<Update>
            + Send
            + 'static,
    ) {
        self.error = None;
        self.job = Some(Job::spawn(ui.ctx(), move |progress| {
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(progress)))
                .map_err(|_| {
                    "Read-only inspection failed unexpectedly; previous evidence retained".into()
                })
                .and_then(|result| result.map_err(|e| format!("{e:#}")))
        }));
    }
}
fn page_ui(ui: &mut egui::Ui, salt: &str, index: &mut usize, page: Page) {
    ui.push_id(salt,|ui|{
    ui.horizontal_wrapped(|ui|{
        if a11y::explain(ui.add_enabled(*index>0,egui::Button::new(t("Previous observation page"))),&t("Review the preceding rows without refreshing or changing the system.")).clicked(){*index-=1;}
        ui.label(setupscope_core::i18n::t(&format!("{} rows · page {}/{}",page.count,page.page+1,page.pages)));
        if a11y::explain(ui.add_enabled(*index+1<page.pages,egui::Button::new(t("Next observation page"))),&t("Review the next rows from the same dated observation.")).clicked(){*index+=1;}
        if a11y::explain(ui.button(t("Copy this page (contains local evidence)")),&t("Explicitly copy the displayed bounded page to the clipboard. Includes local addresses, PIDs and paths; not a redacted diagnostic bundle. No upload or file export occurs.")).clicked(){ui.ctx().copy_text(page.text.clone());}
    });ui.add(egui::Label::new(page.text).selectable(true));
});
}
#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, Vec2};
    fn frame(
        ctx: &egui::Context,
        state: &mut InspectorState,
        events: Vec<Event>,
    ) -> (egui::FullOutput, Actions) {
        let mut action = Actions::default();
        let output = ctx.run(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1100.0, 700.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    action = controls(ui, state);
                });
            },
        );
        (output, action)
    }
    fn key(key: Key) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }
    }
    #[test]
    fn explicit_controls_explain_read_only_limits_and_do_not_start_automatically() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = InspectorState::default();
        frame(&ctx, &mut state, vec![]);
        let (output, action) = frame(&ctx, &mut state, vec![]);
        assert!(!action.ports && !action.locks && !action.browse);
        assert!(state.job.is_none());
        let update = output.platform_output.accesskit_update.unwrap();
        for phrase in [
            "No DNS lookup",
            "Choosing alone never queries",
            "Zero owners does not prove",
        ] {
            assert!(update
                .nodes
                .iter()
                .any(|(_, n)| n.description().is_some_and(|d| d.contains(phrase))));
        }
    }
    #[test]
    fn keyboard_requests_ports_and_selected_file_without_dispatching_side_effects() {
        for (tabs, locks) in [(2, false), (5, true)] {
            let ctx = egui::Context::default();
            let mut state = InspectorState::default();
            state.file = "C:\\fixture.txt".into();
            frame(&ctx, &mut state, vec![]);
            for _ in 0..tabs {
                frame(&ctx, &mut state, vec![key(Key::Tab)]);
            }
            let action = frame(&ctx, &mut state, vec![key(Key::Enter)]).1;
            assert_eq!(action.ports, !locks);
            assert_eq!(action.locks, locks);
            assert!(state.job.is_none());
        }
    }
    #[test]
    fn missing_file_and_busy_state_disable_file_query() {
        for busy in [false, true] {
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut state = InspectorState::default();
            if busy {
                state.file = "C:\\fixture.txt".into();
                state.job = Some(Job::spawn(&ctx, |_| Err("fixture".into())));
            }
            frame(&ctx, &mut state, vec![]);
            let (output, _) = frame(&ctx, &mut state, vec![]);
            let update = output.platform_output.accesskit_update.unwrap();
            let button = update
                .nodes
                .iter()
                .find(|(_, n)| {
                    n.role() == egui::accesskit::Role::Button
                        && n.description()
                            .is_some_and(|d| d.contains("Register only this local file"))
                })
                .unwrap();
            assert!(button.1.is_disabled());
        }
    }
}
