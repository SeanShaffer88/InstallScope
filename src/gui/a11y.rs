//! Keyboard accessibility: a visible focus ring, arrow-key list navigation,
//! screen-reader names for custom-drawn widgets, and control explanations
//! that mouse, keyboard, touch and screen-reader users can all reach.
//!
//! egui already moves focus with Tab / Shift+Tab (and arrow keys between
//! focused widgets), activates the focused button with Enter or Space, and
//! exposes the UI to screen readers (Narrator, NVDA) through AccessKit.

use egui::{
    Align2, Color32, Context, FontId, Id, Key, Modifiers, Response, Ui, WidgetInfo, WidgetType,
};

fn mode_id() -> Id {
    Id::new("installscope_keyboard_mode")
}

/// True while the user is navigating with the keyboard (the focus ring is
/// shown); a mouse click switches it off again.
pub fn keyboard_mode(ctx: &Context) -> bool {
    ctx.data(|d| d.get_temp::<bool>(mode_id()).unwrap_or(false))
}

/// Track whether the keyboard or the mouse was used last.
pub fn track_input_mode(ctx: &Context) {
    let (keys, mouse) = ctx.input(|i| {
        let keys = [
            Key::Tab,
            Key::ArrowUp,
            Key::ArrowDown,
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::Home,
            Key::End,
            Key::PageUp,
            Key::PageDown,
        ]
        .iter()
        .any(|k| i.key_pressed(*k));
        (keys, i.pointer.any_pressed())
    });
    if keys || mouse {
        ctx.data_mut(|d| d.insert_temp(mode_id(), keys && !mouse));
    }
}

/// Draw a clear outline around the focused widget while using the keyboard.
pub fn focus_ring(ctx: &Context, color: egui::Color32) {
    if !keyboard_mode(ctx) {
        return;
    }
    let Some(id) = ctx.memory(|m| m.focused()) else {
        return;
    };
    let Some(resp) = ctx.read_response(id) else {
        return;
    };
    if !resp.rect.is_positive() {
        return;
    }
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        Id::new("focus_ring"),
    ));
    painter.rect_stroke(
        resp.rect.expand(2.0),
        egui::CornerRadius::same(6),
        egui::Stroke::new(2.0_f32, color),
        egui::StrokeKind::Outside,
    );
}

/// Name a custom-drawn clickable widget for screen readers.
pub fn button(resp: &Response, label: &str) {
    let label = label.to_string();
    resp.widget_info(|| WidgetInfo::labeled(WidgetType::Button, true, &label));
}

/// Name a selectable item (navigation entry, list item) for screen readers.
pub fn selectable(resp: &Response, label: &str, selected: bool) {
    let label = label.to_string();
    resp.widget_info(|| WidgetInfo::selected(WidgetType::SelectableLabel, true, selected, &label));
}

fn focus_explanation_id() -> Id {
    Id::new("installscope_focus_explanation")
}

/// An explanation recorded by [`explain`] for the control that holds
/// keyboard focus, kept between passes so a page can show it in a fixed
/// place (see [`focus_hint`]).
#[derive(Clone, Default)]
struct FocusExplanation {
    control: Option<Id>,
    text: String,
}

/// Attach a plain-language explanation to a control so that every input
/// mode can reach it:
///
/// - **Mouse** — the explanation is the control's hover tooltip, shown
///   whether the control is enabled or disabled (a disabled control is
///   exactly where "why can't I use this?" needs an answer).
/// - **Keyboard and touch** — egui never shows tooltips for keyboard
///   focus, so while the control has focus the explanation is recorded
///   for the page's explanation line ([`focus_hint`]). A tap focuses a
///   control just as Tab does, so touch users see it there too.
/// - **Screen readers** — the explanation becomes the control's AccessKit
///   `description`, announced after its name. egui's [`WidgetInfo`] has
///   no description field, so the control's node is addressed directly
///   through [`Context::accesskit_node_builder`], alongside the name and
///   role the control already reports for itself; this is a no-op when
///   no assistive technology is attached.
///
/// Keep explanations to one short sentence: the explanation line shows a
/// single line. Essential information (consequences, requirements) must
/// also be visible page text or a confirmation — never let this helper
/// be the only carrier.
pub fn explain(resp: Response, text: &str) -> Response {
    if text.is_empty() {
        return resp;
    }
    let _ = resp.ctx.accesskit_node_builder(resp.id, |node| {
        node.set_description(text);
    });
    if resp.has_focus() {
        let recorded = FocusExplanation {
            control: Some(resp.id),
            text: text.to_string(),
        };
        resp.ctx
            .data_mut(|d| d.insert_temp(focus_explanation_id(), recorded));
    }
    resp.on_hover_text(text).on_disabled_hover_text(text)
}

/// Take the explanation recorded for the control that currently has
/// focus, if any. Taking clears the record; [`explain`] re-records it on
/// every pass in which its control keeps focus, so the explanation line
/// tracks focus with at most a pass of lag — the same trade egui's own
/// tooltips make with their previous-pass state.
pub fn take_focus_explanation(ctx: &Context) -> Option<String> {
    let recorded = ctx.data_mut(|d| d.remove_temp::<FocusExplanation>(focus_explanation_id()))?;
    if recorded.control.is_some() && recorded.control == ctx.memory(|m| m.focused()) {
        Some(recorded.text)
    } else {
        None
    }
}

/// A page's explanation line: a fixed-height slot, placed near the top of
/// a page, showing the explanation of the focused control (see
/// [`explain`]). The height is reserved whether or not an explanation is
/// showing, so focus moving never shifts the page layout.
pub fn focus_hint(ui: &mut Ui, color: Color32) {
    let (_id, rect) = ui.allocate_space(egui::vec2(ui.available_width(), 17.0));
    if let Some(text) = take_focus_explanation(ui.ctx()) {
        ui.painter_at(rect).text(
            rect.left_top() + egui::vec2(0.0, 1.0),
            Align2::LEFT_TOP,
            text,
            FontId::proportional(12.0),
            color,
        );
    }
}

/// Arrow-key navigation for a list: Up/Down, Page Up/Down, Home/End.
/// Only acts when no widget has keyboard focus (press Esc to leave a text
/// box first) and no dialog is open. Returns the new index if it changed.
pub fn list_nav(ctx: &Context, len: usize, current: Option<usize>, blocked: bool) -> Option<usize> {
    if len == 0 || blocked || ctx.memory(|m| m.focused().is_some()) {
        return None;
    }
    let last = len - 1;
    let cur = current.map(|c| c.min(last));
    let step = |delta: isize| -> usize {
        match cur {
            None => {
                if delta < 0 {
                    last
                } else {
                    0
                }
            }
            Some(c) => (c as isize + delta).clamp(0, last as isize) as usize,
        }
    };
    let next = ctx.input_mut(|i| {
        if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
            Some(step(1))
        } else if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
            Some(step(-1))
        } else if i.consume_key(Modifiers::NONE, Key::PageDown) {
            Some(step(10))
        } else if i.consume_key(Modifiers::NONE, Key::PageUp) {
            Some(step(-10))
        } else if i.consume_key(Modifiers::NONE, Key::Home) {
            Some(0)
        } else if i.consume_key(Modifiers::NONE, Key::End) {
            Some(last)
        } else {
            None
        }
    })?;
    ctx.data_mut(|d| d.insert_temp(mode_id(), true));
    (Some(next) != current).then_some(next)
}

/// Space pressed with nothing focused (toggles the checkbox of the selected
/// list item).
pub fn space_pressed(ctx: &Context, blocked: bool) -> bool {
    !blocked
        && ctx.memory(|m| m.focused().is_none())
        && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Space))
}

/// Escape pressed, for closing windows that egui doesn't close by itself.
/// Ignored while a text box has focus (Esc first leaves the text box).
pub fn escape_pressed(ctx: &Context) -> bool {
    !ctx.wants_keyboard_input() && ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape))
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Event, Pos2, RawInput, Rect, Vec2};

    fn press(ctx: &Context, key: Key) -> Option<usize> {
        let mut out = None;
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            out = list_nav(ctx, 5, Some(2), false);
        });
        out
    }

    #[test]
    fn arrows_move_and_clamp() {
        let ctx = Context::default();
        assert_eq!(press(&ctx, Key::ArrowDown), Some(3));
        assert_eq!(press(&ctx, Key::ArrowUp), Some(1));
        assert_eq!(press(&ctx, Key::End), Some(4));
        assert_eq!(press(&ctx, Key::Home), Some(0));
        assert_eq!(press(&ctx, Key::PageDown), Some(4));
        assert_eq!(press(&ctx, Key::Enter), None);
    }

    fn frame_input(events: Vec<Event>) -> RawInput {
        RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 700.0))),
            events,
            ..Default::default()
        }
    }

    fn tab() -> Vec<Event> {
        vec![Event::Key {
            key: Key::Tab,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        }]
    }

    /// One pass with a single explained button, the way a page uses it.
    fn explained_button(ctx: &Context, events: Vec<Event>) -> egui::FullOutput {
        ctx.run(frame_input(events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let resp = ui.add(egui::Button::new("Create"));
                explain(resp, "Create a restore point of how Windows is now.");
            });
        })
    }

    #[test]
    fn explain_sets_the_accesskit_description() {
        let ctx = Context::default();
        ctx.enable_accesskit();
        // Two passes: registration, then the steady state a screen reader
        // would be listening to.
        explained_button(&ctx, Vec::new());
        let out = explained_button(&ctx, Vec::new());
        let update = out
            .platform_output
            .accesskit_update
            .expect("a tree update while AccessKit is enabled");
        let button = update
            .nodes
            .iter()
            .map(|(_, node)| node)
            .find(|node| node.role() == egui::accesskit::Role::Button)
            .expect("the button is in the accessibility tree");
        assert_eq!(
            button.description(),
            Some("Create a restore point of how Windows is now.")
        );
    }

    /// Three buttons in a column: two explained, one plain.
    fn three_buttons(ctx: &Context, events: Vec<Event>) {
        let _ = ctx.run(frame_input(events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let first = ui.add(egui::Button::new("First"));
                explain(first, "First explanation.");
                let second = ui.add(egui::Button::new("Second"));
                explain(second, "Second explanation.");
                ui.add(egui::Button::new("Plain"));
            });
        });
    }

    #[test]
    fn focus_explanation_follows_focus() {
        let ctx = Context::default();
        three_buttons(&ctx, Vec::new());
        // Nothing focused yet: no explanation to show.
        assert_eq!(take_focus_explanation(&ctx), None);

        three_buttons(&ctx, tab());
        three_buttons(&ctx, Vec::new());
        assert_eq!(
            take_focus_explanation(&ctx).as_deref(),
            Some("First explanation.")
        );

        three_buttons(&ctx, tab());
        three_buttons(&ctx, Vec::new());
        assert_eq!(
            take_focus_explanation(&ctx).as_deref(),
            Some("Second explanation.")
        );

        // Focus moves on to a control with no explanation: the line clears
        // instead of showing the previous control's text.
        three_buttons(&ctx, tab());
        three_buttons(&ctx, Vec::new());
        assert_eq!(take_focus_explanation(&ctx), None);
    }
}
