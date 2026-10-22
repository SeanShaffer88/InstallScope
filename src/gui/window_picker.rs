use super::{a11y, Job};
use setupscope_core::i18n::t;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct WindowPicker {
    job: Option<Job<Result<String, String>>>,
    started: Option<Instant>,
    delay_index: usize,
    pub message: Option<String>,
}
impl Drop for WindowPicker {
    fn drop(&mut self) {
        self.cancel();
    }
}
impl WindowPicker {
    pub fn cancel(&mut self) {
        if let Some(job) = self.job.take() {
            job.cancel();
            self.message = Some(t("Window identification canceled."));
        }
        self.started = None;
    }
    pub fn poll(&mut self) -> Option<Result<String, String>> {
        let result = self.job.as_ref()?.poll()?;
        self.job = None;
        self.started = None;
        Some(result)
    }
    pub fn controls(&mut self, ui: &mut egui::Ui, available: bool) {
        if self.job.is_some() {
            let delay = [5, 10, 20][self.delay_index.min(2)];
            let remaining = delay as f32
                - self
                    .started
                    .map(|s| s.elapsed().as_secs_f32())
                    .unwrap_or(delay as f32);
            ui.label(format!(
                "{} ({:.0}s)",
                t("Switch to the application window now; return here after capture"),
                remaining.max(0.0).ceil()
            ));
            if a11y::explain(ui.button(t("Cancel window identification")), &t("Cancel the pending read-only window lookup. Escape also cancels while this page is active.")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) { self.cancel(); }
            ui.ctx().request_repaint_after(Duration::from_millis(100));
        } else {
            ui.horizontal_wrapped(|ui| {
            let label = ui.label(t("Window selection delay"));
            let combo = egui::ComboBox::from_id_salt("window_selection_delay").selected_text(format!("{} s", [5, 10, 20][self.delay_index.min(2)])).show_ui(ui, |ui| {
                for (index, seconds) in [5, 10, 20].iter().enumerate() {
                    a11y::explain(ui.selectable_value(&mut self.delay_index, index, format!("{seconds} s")), &t("Time to switch to the intended window before its path is captured."));
                }
            });
            a11y::explain(combo.response.labelled_by(label.id), &t("Choose 5, 10 or 20 seconds; use a longer delay if you need more time to change windows. This choice is kept for this session only."));
            if a11y::explain(ui.add_enabled(available && cfg!(windows), egui::Button::new(t("Identify window"))), &t("Start the countdown, then switch to the intended application's window using Alt+Tab or a click. Only its executable path is inspected. A unique installed-folder match opens details; nothing is closed, selected for removal or uninstalled.")).clicked() {
            let delay = [5, 10, 20][self.delay_index.min(2)];
            self.message = None;
            self.started = Some(Instant::now());
            self.job = Some(Job::spawn(ui.ctx(), move |progress| {
                let start = Instant::now();
                while start.elapsed() < Duration::from_secs(delay) {
                    if progress.cancelled() { return Err(t("Window identification canceled.")); }
                    std::thread::sleep(Duration::from_millis(25));
                }
                if progress.cancelled() { return Err(t("Window identification canceled.")); }
                setupscope_core::windowidentify::foreground_image().map_err(|e| format!("{e:#}"))
            }));
            }
            });
        }
        if let Some(message) = &self.message {
            ui.add(egui::Label::new(message).wrap().selectable(true));
            if a11y::explain(ui.button(t("Copy identification result")), &t("Copy this result, including the private executable path, to your local clipboard. Nothing is sent online.")).clicked() { ui.ctx().copy_text(message.clone()); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn controls_are_accessible_and_do_not_start_automatically() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut picker = WindowPicker::default();
        let mut frame = || {
            ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| picker.controls(ui, true));
            })
        };
        let _ = frame();
        let output = frame();
        assert!(picker.job.is_none());
        assert!(output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .any(|(_, node)| node
                .description()
                .is_some_and(|text| text.contains("Only its executable path"))));
    }
    #[test]
    fn cancel_discards_even_a_completed_pending_result() {
        let ctx = egui::Context::default();
        let mut picker = WindowPicker {
            job: Some(Job::spawn(&ctx, |_| Ok("fixture.exe".into()))),
            started: Some(Instant::now()),
            message: None,
            delay_index: 0,
        };
        picker.cancel();
        assert!(picker.poll().is_none());
        assert!(picker.started.is_none());
        assert!(picker.message.as_ref().unwrap().contains("canceled"));
    }
    #[test]
    fn keyboard_starts_then_escape_cancels_without_capturing() {
        let ctx = egui::Context::default();
        let mut picker = WindowPicker::default();
        let mut frame = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| picker.controls(ui, true));
                },
            );
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
        frame(vec![key(egui::Key::Tab)]);
        frame(vec![key(egui::Key::Enter)]);
        drop(frame);
        assert!(picker.job.is_some());
        let _ = ctx.run(
            egui::RawInput {
                events: vec![key(egui::Key::Escape)],
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| picker.controls(ui, true));
            },
        );
        assert!(picker.job.is_none());
        assert!(picker.poll().is_none());
    }
}
