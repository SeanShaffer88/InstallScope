//! Keyboard accessibility: a visible focus ring, arrow-key list navigation,
//! and screen-reader names for custom-drawn widgets.
//!
//! egui already moves focus with Tab / Shift+Tab (and arrow keys between
//! focused widgets), activates the focused button with Enter or Space, and
//! exposes the UI to screen readers (Narrator, NVDA) through AccessKit.

use egui::{Context, Id, Key, Modifiers, Response, WidgetInfo, WidgetType};

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
}
