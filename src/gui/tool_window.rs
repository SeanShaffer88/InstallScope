//! Task-specific desktop geometry and always-visible operation boundaries.
use super::app::Tab;

pub struct Design {
    pub size: [f32; 2],
    pub minimum: [f32; 2],
    pub boundary: &'static str,
}

pub fn design(tab: Tab) -> Design {
    use Tab::*;
    let (size, minimum, boundary) = match tab {
        Dashboard => ([960., 600.], [740., 300.], "Open tools independently from the Workflow Board."),
        ProjectReadiness => ([760., 360.], [560., 200.], "Read-only evidence. No build, installation or repair runs; re-inspect after changes."),
        BuildLog => ([720., 250.], [540., 180.], "Log inspection is read-only. Build/test execution requires separate review; effects cannot be automatically undone."),
        Containers => ([660., 250.], [500., 180.], "Passive host evidence; distributions, containers and services are never started."),
        Ports => ([780., 460.], [560., 200.], "Read-only listeners and file owners; no processes are terminated."),
        Environment => ([760., 500.], [560., 220.], "Persistent edits require explicit review. Recorded recovery is available within this tool."),
        Dependencies => {let mut d=design(ProjectReadiness);d.boundary="Downloads and offline bundles are explicit. Vendor installation requires separate review; recovery uses the vendor uninstaller.";return d;},
        Settings => ([580., 250.], [460., 160.], "Appearance preferences only. Existing saved data is preserved."),
        Doctor | Developer | Bundle => return design(ProjectReadiness),
    };
    Design {
        size,
        minimum,
        boundary,
    }
}

/// Measure actual scroll contents, not CentralPanel's viewport-filling rectangle.
pub fn scroll_body<R>(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash,
    contents: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let output = egui::ScrollArea::vertical().id_salt(id).show(ui, contents);
    let key = egui::Id::new(("content_height", ui.ctx().viewport_id()));
    ui.ctx()
        .data_mut(|d| d.insert_temp(key, output.inner_rect.top() + output.content_size.y + 20.));
    output.inner
}

/// Fit on open/content change; preserve deliberate manual resizing until content changes.
pub fn fit_content(ctx: &egui::Context, tab: Tab) {
    let viewport = ctx.viewport_id();
    let Some(content) =
        ctx.data(|d| d.get_temp::<f32>(egui::Id::new(("content_height", viewport))))
    else {
        return;
    };
    let desired = content
        + if tab == Tab::Dashboard {
            ctx.data(|d| d.get_temp::<f32>(egui::Id::new("board_status_height")))
                .unwrap_or(26.)
        } else {
            0.
        };
    let key = egui::Id::new(("fitted_height", viewport));
    let previous = ctx.data(|d| d.get_temp::<f32>(key));
    if previous.is_some_and(|p| (p - desired).abs() < 2.) {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(key, desired));
    let current = ctx.input(|i| {
        i.viewport()
            .inner_rect
            .map(|r| r.size())
            .unwrap_or(egui::vec2(design(tab).size[0], design(tab).size[1]))
    });
    let maximum = ctx.input(|i| {
        i.viewport()
            .monitor_size
            .map(|m| (m.y - 112.).max(160.))
            .unwrap_or(800.)
    });
    let height = desired
        .ceil()
        .clamp(design(tab).minimum[1].min(maximum), maximum);
    if (height - current.y).abs() > 2. {
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
            current.x, height,
        )));
    }
}

impl Design {
    /// Leave space for taskbar/title bar and allow smaller displays to resize.
    /// These are initial hints; user resizing remains unrestricted above minimum.
    pub fn geometry(&self, monitor: Option<egui::Vec2>) -> ([f32; 2], [f32; 2]) {
        let available = monitor
            .filter(|m| m.x.is_finite() && m.y.is_finite() && m.x > 0. && m.y > 0.)
            .map(|m| [(m.x - 48.).max(1.), (m.y - 112.).max(1.)]);
        let mut size = self.size;
        let mut minimum = self.minimum;
        if let Some(bounds) = available {
            for axis in 0..2 {
                size[axis] = size[axis].min(bounds[axis]);
                minimum[axis] = minimum[axis].min(size[axis]);
            }
        }
        (size, minimum)
    }
}

/// Tool-specific terminal identity; workflow cues are guidance, not progress claims.
pub fn header(ui: &mut egui::Ui, tab: Tab, title: &str) -> bool {
    use Tab::*;
    let (_mark, badge, flow) = match tab {
        ProjectReadiness | Doctor | Developer | Bundle => {
            ("./", "Read-Only", "Project > Evidence > Next Steps")
        }
        Dependencies => (
            "./",
            "Review First",
            "Select > Download / Offline Bundle > Review > Vendor UI",
        ),
        BuildLog => (
            "!>",
            "Review First",
            "Review Build / Test > Captured Log > Triage",
        ),
        Environment => (
            "$_",
            "Review First",
            "Profile > Preview > Review / Recovery",
        ),
        Ports => ("::", "Read-Only", "Listener Tables + Selected File Owners"),
        Containers => (
            "[]",
            "Passive",
            "Windows > Virtualization > Runtime Evidence",
        ),
        _ => ("#", "Appearance", "Local Interface Preferences"),
    };
    let mut board_requested = false;
    ui.horizontal_top(|ui| {
      ui.allocate_ui_with_layout(egui::vec2(ui.available_width(),32.),egui::Layout::right_to_left(egui::Align::Min), |ui| {
        board_requested=ui.small_button(setupscope_core::i18n::t("Workflow Board")).on_hover_text(setupscope_core::i18n::t("Show the main window without closing any tools or interrupting their work. Opening another tool hides the board again.")).clicked();
       ui.with_layout(egui::Layout::left_to_right(egui::Align::Center).with_main_wrap(true), |ui| {
        instrument_mark(ui, tab);
        ui.label(egui::RichText::new(setupscope_core::i18n::t(title)).monospace().size(18.).strong());
        let badge = egui::Frame::new()
            .fill(super::theme::ACCENT_DIM)
            .corner_radius(3)
            .inner_margin(egui::Margin::symmetric(6, 2))
            .show(ui, |ui| ui.small(setupscope_core::i18n::t(badge)))
            .inner;
        super::a11y::explain(badge, design(tab).boundary);
       });
      });
    });
    ui.label(
        egui::RichText::new(setupscope_core::i18n::t(flow))
            .monospace()
            .size(12.)
            .color(super::theme::MUTED),
    );
    board_requested
}

/// Decorative, task-specific instrument diagrams; the adjacent title supplies semantics.
fn instrument_mark(ui: &mut egui::Ui, tab: Tab) {
    use Tab::*;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(32., 32.), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    let point = |x: f32, y: f32| rect.min + egui::vec2(x, y);
    let red = egui::Stroke::new(1.6_f32, super::theme::ACCENT_TEXT);
    let dim = egui::Stroke::new(1.0_f32, super::theme::MUTED);
    let line = |a: (f32, f32), b: (f32, f32), s: egui::Stroke| {
        painter.line_segment([point(a.0, a.1), point(b.0, b.1)], s);
    };
    let box_at = |x: f32, y: f32, w: f32, h: f32, s: egui::Stroke| {
        painter.rect_stroke(
            egui::Rect::from_min_size(point(x, y), egui::vec2(w, h)),
            2.,
            s,
            egui::StrokeKind::Inside,
        );
    };
    match tab {
        ProjectReadiness | Doctor | Developer | Bundle => {
            box_at(3., 8., 22., 19., dim);
            line((4., 8.), (4., 4.), red);
            line((4., 4.), (13., 4.), red);
            for y in [12., 16., 20.] {
                line((7., y), (15., y), dim);
            }
            painter.circle_stroke(point(23., 22.), 5., red);
            line((26., 26.), (30., 30.), red);
        }
        BuildLog => {
            box_at(2., 3., 25., 25., dim);
            for y in [8., 13., 23.] {
                line((7., y), (22., y), dim);
            }
            line((8., 18.), (12., 18.), red);
            line((12., 18.), (15., 10.), red);
            line((15., 10.), (20., 25.), red);
            line((20., 25.), (24., 18.), red);
            line((24., 18.), (31., 18.), red);
        }
        Environment => {
            for (x, y) in [(7., 16.), (25., 6.), (25., 26.)] {
                painter.circle_stroke(point(x, y), 4., red);
            }
            line((11., 16.), (16., 16.), dim);
            line((16., 16.), (16., 6.), dim);
            line((16., 6.), (21., 6.), red);
            line((16., 16.), (16., 26.), dim);
            line((16., 26.), (21., 26.), red);
        }
        Ports => {
            box_at(2., 10., 8., 12., red);
            line((10., 16.), (16., 16.), dim);
            line((16., 5.), (16., 27.), dim);
            for y in [5., 16., 27.] {
                line((16., y), (24., y), red);
                box_at(24., y - 3., 6., 6., red);
            }
        }
        Containers => {
            for (x, y) in [(3., 3.), (7., 11.), (11., 19.)] {
                box_at(x, y, 18., 9., if y == 19. { red } else { dim });
                line((x + 4., y + 4.), (x + 13., y + 4.), red);
            }
        }
        Dependencies => {
            box_at(4., 13., 24., 16., dim);
            line((4., 18.), (28., 18.), red);
            line((16., 2.), (16., 14.), red);
            line((11., 9.), (16., 14.), red);
            line((16., 14.), (21., 9.), red);
            line((16., 18.), (16., 28.), dim);
        }
        _ => {
            painter.circle_stroke(point(16., 16.), 10., red);
            painter.circle_stroke(point(16., 16.), 4., dim);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn board_button_stays_at_top_right_for_every_tool() {
        for tab in [
            Tab::ProjectReadiness,
            Tab::Dependencies,
            Tab::BuildLog,
            Tab::Environment,
            Tab::Ports,
            Tab::Containers,
            Tab::Settings,
        ] {
            let width = design(tab).minimum[0];
            let ctx = egui::Context::default();
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        header(ui, tab, "Tool Title");
                    });
                },
            );
            let text = output
                .shapes
                .iter()
                .find_map(|s| {
                    if let egui::epaint::Shape::Text(t) = &s.shape {
                        (t.galley.text() == "Workflow Board").then_some(t)
                    } else {
                        None
                    }
                })
                .unwrap();
            assert!(
                text.pos.x > width - 150. && text.pos.x + text.galley.size().x <= width,
                "{tab:?}"
            );
            assert!(text.pos.y < 40., "{tab:?}");
        }
    }
    #[test]
    fn measured_contents_resize_without_filling_blank_panel_space() {
        let ctx = egui::Context::default();
        let render = |height: f32| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(720., 600.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        scroll_body(ui, "fit-test", |ui| {
                            ui.allocate_space(egui::vec2(200., height));
                        });
                    });
                    fit_content(ctx, Tab::BuildLog);
                },
            )
        };
        let small = render(100.);
        let commands = &small.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands
            .iter()
            .any(|c| matches!(c,egui::ViewportCommand::InnerSize(s) if s.y<300.)));
        let large = render(420.);
        let commands = &large.viewport_output[&egui::ViewportId::ROOT].commands;
        assert!(commands
            .iter()
            .any(|c| matches!(c,egui::ViewportCommand::InnerSize(s) if s.y>400. && s.y<600.)));
        let unchanged = render(420.);
        assert!(!unchanged.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::InnerSize(_))));
    }
    #[test]
    fn every_tool_starts_compact_and_remains_resizable() {
        for &(tab, _, _, _) in super::super::app::NAVIGATION {
            let d = design(tab);
            assert!(
                d.size[0] <= 800. && d.size[1] <= 600.,
                "{tab:?} exceeds compact window budget"
            );
            assert!(d.minimum[0] < d.size[0] && d.minimum[1] < d.size[1]);
        }
    }
    #[test]
    fn tool_geometry_fits_small_displays_and_keeps_resize_possible() {
        for &(tab, _, _, _) in super::super::app::NAVIGATION {
            let d = design(tab);
            let (size, min) = d.geometry(Some(egui::vec2(800., 600.)));
            assert!(size[0] <= 752. && size[1] <= 488.);
            for axis in 0..2 {
                assert!(min[axis] > 0. && min[axis] <= size[axis]);
            }
            assert!(!d.boundary.is_empty());
        }
    }
}
