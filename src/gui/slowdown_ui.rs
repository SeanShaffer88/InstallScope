//! Read-only history investigation. Measurements describe current startup cost,
//! never a causal estimate of the slowdown or a before/after comparison.
use super::{
    a11y,
    app::{App, Tab},
    theme, Job,
};
use installscope_core::{
    history::{self, HistoryStore, SlowdownReport},
    i18n::t,
    time::Date,
};

struct Loaded {
    report: SlowdownReport,
    impact: Vec<Option<u64>>,
}

pub struct SlowdownState {
    from: String,
    to: String,
    job: Option<Job<Result<Loaded, String>>>,
    loaded: Option<Loaded>,
    error: Option<String>,
}

impl Default for SlowdownState {
    fn default() -> Self {
        let today = Date::today_local();
        Self {
            from: today.add_days(-90).to_string(),
            to: today.to_string(),
            job: None,
            loaded: None,
            error: None,
        }
    }
}

fn date_range(from: &str, to: &str) -> Result<(Date, Date), String> {
    let parse = |s: &str| {
        // Date::parse_ymd accepts a timestamp prefix; text fields require exactly
        // a date. Validate ASCII before its byte-indexed parser sees user text.
        if s.len() != 10 || !s.is_ascii() {
            return None;
        }
        Date::parse_ymd(s)
    };
    let from = parse(from)
        .ok_or_else(|| t("Enter the From date as YYYY-MM-DD (for example 2026-10-01)."))?;
    let to =
        parse(to).ok_or_else(|| t("Enter the To date as YYYY-MM-DD (for example 2026-10-21)."))?;
    if from > to {
        return Err(t("From must be on or before To."));
    }
    Ok((from, to))
}

fn date_fields(ui: &mut egui::Ui, state: &mut SlowdownState) {
    let from_label = ui.label(t("From"));
    a11y::explain(
        ui.add(
            egui::TextEdit::singleline(&mut state.from)
                .desired_width(110.0)
                .hint_text("YYYY-MM-DD"),
        )
        .labelled_by(from_label.id),
        &t("First day to include; enter YYYY-MM-DD, then Refresh."),
    );
    let to_label = ui.label(t("To"));
    a11y::explain(
        ui.add(
            egui::TextEdit::singleline(&mut state.to)
                .desired_width(110.0)
                .hint_text("YYYY-MM-DD"),
        )
        .labelled_by(to_label.id),
        &t("Last day to include; enter YYYY-MM-DD, then Refresh."),
    );
}

impl App {
    pub fn refresh_slowdown(&mut self, ctx: &egui::Context) {
        if self.slowdown.job.is_some() {
            return;
        }
        let (from, to) = match date_range(&self.slowdown.from, &self.slowdown.to) {
            Ok(range) => range,
            Err(e) => {
                self.slowdown.error = Some(e);
                return;
            }
        };
        self.slowdown.error = None;
        self.slowdown.loaded = None;
        let root = self.store.root.clone();
        let reg = self.reg.clone();
        self.slowdown.job = Some(Job::spawn(ctx, move |_| {
            let store = match std::fs::read(HistoryStore::path(&root)) {
                Ok(bytes) => installscope_core::json::compat::from_slice::<HistoryStore>(&bytes)
                    .map_err(|e| format!("Could not read history.json: {e}. Restore a known-good copy from your data backup."))?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => HistoryStore::default(),
                Err(e) => return Err(format!("Could not open history.json: {e}. Check access to the data folder and try Refresh.")),
            };
            let mut report = history::report(&store, from, to);
            let boot = installscope_core::boot::read();
            let usage = installscope_core::usage::UsageData::gather(reg.as_ref(), &root);
            let programs = installscope_core::programs::list_programs(reg.as_ref(), false);
            report.events.reverse();
            let impact: Vec<Option<u64>> = report
                .events
                .iter()
                .map(|event| {
                    history::boot_impact_ms(event, boot.as_ref()).or_else(|| {
                        if !event.kind.is_program() {
                            return None;
                        }
                        let mut matches = programs.iter().filter(|p| {
                            installscope_core::model::fold(&p.name)
                                == installscope_core::model::fold(&event.subject)
                        });
                        let p = matches.next()?;
                        if matches.next().is_some() {
                            return None;
                        }
                        usage.for_program(p).startup_ms
                    })
                })
                .collect();
            let mut rows: Vec<_> = report.events.into_iter().zip(impact).collect();
            rows.sort_by(|a, b| b.1.cmp(&a.1).then(b.0.ts.cmp(&a.0.ts)));
            let (events, impact) = rows.into_iter().unzip();
            report.events = events;
            Ok(Loaded { report, impact })
        }));
    }

    pub fn ui_slowdown(&mut self, ctx: &egui::Context) {
        if let Some(result) = self.slowdown.job.as_ref().and_then(Job::poll) {
            self.slowdown.job = None;
            match result {
                Ok(loaded) => self.slowdown.loaded = Some(loaded),
                Err(error) => self.slowdown.error = Some(error),
            }
        }
        let mut refresh = false;
        let mut go = None;
        egui::CentralPanel::default().frame(theme::content_frame()).show(ctx, |ui| {
            a11y::focus_hint(ui, theme::MUTED);
            ui.label(t("What changed around the time your PC became slower?"));
            ui.label(t("Timing is a clue, not proof. Startup figures are current measurements, not before-and-after comparisons. Nothing is changed here."));
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                let busy = self.slowdown.job.is_some();
                ui.add_enabled_ui(!busy, |ui| {
                    date_fields(ui, &mut self.slowdown);
                    refresh = a11y::explain(ui.button(t("Refresh")), &t("Read saved history and current startup measurements for these dates; no settings or files are changed (F5).")).clicked();
                });
                if busy { ui.spinner(); ui.label(t("Reading history and Windows measurements…")); }
            });
            if let Some(error) = &self.slowdown.error { ui.colored_label(theme::BAD, error); }
            if let Some(loaded) = &self.slowdown.loaded {
                let r = &loaded.report;
                ui.label(format!("{} – {} · {} {} · {} {}", r.from, r.to, r.events.len(), t("changes in range"), r.total_events, t("recorded in total")));
                if let Some(started) = &r.history_started { ui.label(format!("{} {started}", t("Recording began:"))); }
                if r.is_empty() {
                    ui.label(if r.total_events == 0 { t("Not enough history yet. Run installscope history or installscope slowdown to capture a baseline, then run it again after changes. Refresh reads that saved history.") } else { t("No recorded changes in this range. Widen the dates and Refresh.") });
                }
                ui.label(t("Measured entries appear first, highest startup cost first; unmeasured entries follow by recency. No measurement means unknown impact, not zero. Services and scheduled-task history are not covered yet."));
                egui::ScrollArea::vertical().max_height((ui.available_height() - 48.0).max(80.0)).id_salt("slowdown_events").show(ui, |ui| {
                    for (event, impact) in r.events.iter().zip(&loaded.impact) {
                        ui.separator();
                        ui.label(format!("{} · {} · {}", event.date, t(event.kind.label()), event.subject));
                        ui.label(match impact { Some(ms) => format!("{} {ms} ms", t("Measured startup impact:")), None => t("Startup impact: no measurement") });
                        if !event.from_version.is_empty() || !event.to_version.is_empty() { ui.label(format!("{} → {}", event.from_version, event.to_version)); }
                        if !event.target.is_empty() { ui.label(&event.target); }
                        if !event.detail.is_empty() { ui.label(&event.detail); }
                    }
                });
            }
            ui.horizontal_wrapped(|ui| {
                if a11y::explain(ui.button(t("Review programs")), &t("Open Programs to investigate installed apps; opening this page never uninstalls anything.")).clicked() { go = Some(Tab::Programs); }
                if a11y::explain(ui.button(t("Review startup")), &t("Open Startup to review what runs at sign-in through its existing safety workflow.")).clicked() { go = Some(Tab::Startup); }
            });
        });
        if refresh {
            self.refresh_slowdown(ctx);
        }
        if let Some(tab) = go {
            self.tab = tab;
            self.on_tab_opened(ctx, tab);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn range_validation_handles_invalid_calendar_unicode_and_reversed_dates() {
        for from in [
            "2026-02-29",
            "2026-10-01 extra",
            "éééé-10",
            "",
            "2026-10-22",
        ] {
            assert!(date_range(from, "2026-10-21").is_err(), "{from}");
        }
        assert!(date_range("2024-02-29", "2024-02-29").is_ok());
    }
    #[test]
    fn date_fields_have_accessible_names_and_descriptions() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut state = SlowdownState::default();
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| date_fields(ui, &mut state));
            }));
        }
        let update = output.unwrap().platform_output.accesskit_update.unwrap();
        let descriptions: Vec<_> = update
            .nodes
            .iter()
            .filter_map(|(_, node)| node.description())
            .collect();
        assert!(descriptions
            .iter()
            .any(|s| s.contains("First day to include")));
        assert!(descriptions
            .iter()
            .any(|s| s.contains("Last day to include")));
        let labelled_fields = update
            .nodes
            .iter()
            .filter(|(_, node)| !node.labelled_by().is_empty())
            .count();
        assert!(labelled_fields >= 2);
    }
}
