//! Small shared UI pieces.

use super::theme;
use egui::RichText;
use setupscope_core::backup::ActionResult;
use setupscope_core::i18n::{t, tf};
use std::path::PathBuf;

pub fn muted(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.label(RichText::new(text.into()).color(theme::MUTED))
}

pub fn small_muted(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.label(RichText::new(text.into()).color(theme::MUTED).small())
}

/// Label + selectable value on one line of a grid.
pub fn field(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.label(RichText::new(t(label)).color(theme::MUTED));
    if value.is_empty() {
        ui.label(RichText::new("—").color(theme::MUTED));
    } else {
        ui.add(egui::Label::new(value).wrap().selectable(true));
    }
    ui.end_row();
}

/// Progress line for a running job.
pub fn job_progress(ui: &mut egui::Ui, phase: &str, count: u64) {
    ui.horizontal(|ui| {
        spinner(ui);
        ui.label(phase);
        if count > 0 {
            small_muted(ui, tf("{n} items", &[("n", count.to_string())]));
        }
    });
}

pub fn pick_file(title: &str, filter_name: &str, exts: &[&str]) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        super::filedialog::pick_file(&t(title), &t(filter_name), exts)
    }
    #[cfg(not(windows))]
    {
        // Demo/testing builds: SETUPSCOPE_DEMO_OPEN names the file to "pick".
        let _ = (title, filter_name, exts);
        std::env::var_os("SETUPSCOPE_DEMO_OPEN").map(PathBuf::from)
    }
}

pub fn pick_folder(title: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        super::filedialog::pick_folder(&t(title))
    }
    #[cfg(not(windows))]
    {
        let _ = title;
        std::env::var_os("SETUPSCOPE_DEMO").map(|_| std::env::temp_dir())
    }
}

pub fn save_file(title: &str, default_name: &str, filter_name: &str, ext: &str) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        super::filedialog::save_file(&t(title), default_name, &t(filter_name), ext)
    }
    #[cfg(not(windows))]
    {
        let _ = (title, filter_name, ext);
        Some(std::env::temp_dir().join(default_name))
    }
}

/// Summary and details of an operation's results.
pub fn results_ui(ui: &mut egui::Ui, results: &[ActionResult]) {
    let failed = results.iter().filter(|r| !r.ok).count();
    let ok = results.len() - failed;
    ui.horizontal(|ui| {
        theme::badge(ui, tf("{n} succeeded", &[("n", ok.to_string())]), theme::OK);
        if failed > 0 {
            theme::badge(
                ui,
                tf("{n} failed", &[("n", failed.to_string())]),
                theme::BAD,
            );
        }
        if ui.small_button(t("Copy report")).clicked() {
            let text: String = results
                .iter()
                .map(|r| {
                    format!(
                        "[{}] {} — {}\n",
                        if r.ok { "OK" } else { "FAILED" },
                        r.description,
                        r.message
                    )
                })
                .collect();
            ui.ctx().copy_text(text);
        }
    });
    ui.add_space(4.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .max_height(360.0)
        .show(ui, |ui| {
            let mut sorted: Vec<&ActionResult> = results.iter().collect();
            sorted.sort_by_key(|r| r.ok);
            for r in sorted {
                ui.horizontal_wrapped(|ui| {
                    let (sym, col) = if r.ok {
                        ("✔", theme::OK)
                    } else {
                        ("✖", theme::BAD)
                    };
                    ui.label(RichText::new(sym).color(col));
                    ui.label(&r.description);
                    ui.label(
                        RichText::new(&r.message)
                            .color(if r.ok { theme::MUTED } else { theme::BAD })
                            .small(),
                    );
                });
            }
        });
}

/// A search box with a clear button.
pub fn search_box(ui: &mut egui::Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(text)
                .hint_text(format!("🔍 {}", t(hint)))
                .desired_width(width),
        );
        if !text.is_empty() && ui.small_button("✖").clicked() {
            text.clear();
        }
        r
    })
    .inner
}

/// A busy indicator that redraws about 15 times a second instead of every
/// frame (egui's own spinner repaints continuously, which keeps a CPU core
/// busy on PCs without a fast GPU).
pub fn spinner(ui: &mut egui::Ui) -> egui::Response {
    let size = ui.style().spacing.interact_size.y;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    if ui.is_rect_visible(rect) {
        let time = ui.input(|i| i.time);
        let color = ui.visuals().strong_text_color();
        let radius = rect.height() / 2.0 - 2.0;
        let n = 24;
        let start = time * std::f64::consts::TAU;
        let end = start + 240f64.to_radians() * ((time + 0.5).sin() * 0.5 + 0.6);
        let pts: Vec<egui::Pos2> = (0..=n)
            .map(|i| {
                let a = start + (end - start) * i as f64 / n as f64;
                rect.center() + radius * egui::vec2(a.cos() as f32, a.sin() as f32)
            })
            .collect();
        ui.painter()
            .add(egui::Shape::line(pts, egui::Stroke::new(3.0_f32, color)));
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(66));
    }
    resp
}

/// Keep full scope guidance accessible without crowding compact tool controls.
pub fn scope_note(ui: &mut egui::Ui, summary: &str, details: &str) {
    ui.label(t(summary));
    collapsing_header(ui,t("Scope And Limits")).id_salt(summary).show(ui, |ui| {
        ui.add(egui::Label::new(t(details)).wrap());
    });
}

/// A section heading that wraps inside compact translated tool windows.
pub fn collapsing_header(ui: &egui::Ui, label: impl Into<egui::WidgetText>) -> egui::CollapsingHeader {
    // egui's stock header forces Extend even when the shared style wraps.
    let width=(ui.available_width()-ui.spacing().indent-ui.spacing().button_padding.x*2.).max(32.);
    let galley=label.into().into_galley(ui,Some(egui::TextWrapMode::Wrap),width,egui::TextStyle::Button);
    let header=egui::CollapsingHeader::new(galley);
    // Exercise every configuration section in the isolated layout subprocess.
    #[cfg(test)]
    let header=header.default_open(std::env::var_os("SETUPSCOPE_LAYOUT_LOCALE").is_some());
    header
}

/// Deliberate primary action with the shared red terminal accent.
pub fn action_button(ui: &mut egui::Ui, enabled: bool, label: &str) -> egui::Response {
    ui.add_enabled(enabled,egui::Button::new(egui::RichText::new(t(label)).strong().color(egui::Color32::WHITE)).fill(theme::ACCENT).stroke(egui::Stroke::NONE))
}
