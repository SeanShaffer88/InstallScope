//! SetupScope's look: the Command Deck theme — a dense operational status
//! board on a pure black/red identity. Palette: window #000000, sidebar
//! #050505, cards #101010, hairline rules #242424; primary red #6B0700 is
//! reserved for the primary action, attention rules and the selected nav
//! indicator; red text #E5484D reads at ≥4.5:1 on black.

use egui::{Color32, CornerRadius, Stroke, Visuals};

/// Primary red. Reserved for the primary action of a view, tiles that need
/// attention (a 3px top rule), and the selected nav indicator.
pub const ACCENT: Color32 = Color32::from_rgb(0x6B, 0x07, 0x00); // #6B0700
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x4A, 0x05, 0x00);
/// Lighter shade of the accent for red text on the dark background, where
/// #6B0700 itself would be too dark to read. #E5484D reaches ~5.4:1 on
/// #000000 and ~4.9:1 on #101010.
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0xE5, 0x48, 0x4D);
/// Native window border colour (RGB).
pub const BORDER_RGB: (u8, u8, u8) = (0x6B, 0x07, 0x00);

pub const OK: Color32 = Color32::from_rgb(0x3F, 0xB9, 0x50); // #3FB950
pub const WARN: Color32 = Color32::from_rgb(0xD2, 0x99, 0x22); // #D29922
pub const BAD: Color32 = ACCENT_TEXT;
/// Secondary text: #A8A8A8 reaches ~8.1:1 on #101010.
pub const MUTED: Color32 = Color32::from_rgb(0xA8, 0xA8, 0xA8);
/// Faintest text for non-essential meta (footers, section labels). #6E6E6E
/// reaches only ~3.8:1 on #101010 / ~4.2:1 on #000000, so FAINT must never
/// carry meaning that MUTED or TEXT should carry.
pub const FAINT: Color32 = Color32::from_rgb(0x6E, 0x6E, 0x6E);
pub const ADDED: Color32 = OK;
pub const CHANGED: Color32 = WARN;
pub const REMOVED: Color32 = BAD;

pub const BG: Color32 = Color32::from_rgb(0x00, 0x00, 0x00); // #000000
pub const SIDEBAR: Color32 = Color32::from_rgb(0x05, 0x05, 0x05); // #050505
pub const CARD: Color32 = Color32::from_rgb(0x10, 0x10, 0x10); // #101010
const PANEL: Color32 = SIDEBAR;
const WIDGET: Color32 = Color32::from_rgb(0x1C, 0x1C, 0x1C);
const HOVER: Color32 = Color32::from_rgb(0x2A, 0x2A, 0x2A);
pub const LINE: Color32 = Color32::from_rgb(0x24, 0x24, 0x24); // #242424
pub const NAV_SELECTED: Color32 = ACCENT;
/// Primary text: #F5F5F5 reaches ~17.4:1 on #101010.
pub const TEXT: Color32 = Color32::from_rgb(0xF5, 0xF5, 0xF5);

pub fn apply(ctx: &egui::Context, high_contrast: bool) {
    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::Dark);
    let mut v = Visuals::dark();
    v.override_text_color = Some(if high_contrast { Color32::WHITE } else { TEXT });
    v.window_fill = PANEL;
    v.panel_fill = BG;
    v.faint_bg_color = Color32::from_rgb(0x0A, 0x0A, 0x0A);
    v.extreme_bg_color = Color32::from_rgb(0x00, 0x00, 0x00);
    v.code_bg_color = Color32::from_rgb(0x10, 0x10, 0x10);
    v.hyperlink_color = ACCENT_TEXT;
    v.selection.bg_fill = ACCENT_DIM;
    // Selected text stays white so it is readable on the red highlight.
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.window_stroke = Stroke::new(1.0_f32, ACCENT_DIM);
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(6);
    v.warn_fg_color = WARN;
    v.error_fg_color = BAD;

    // Soft shadows under windows and menus, and rounder controls.
    v.window_shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    v.popup_shadow = egui::epaint::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    let r = CornerRadius::same(6);
    v.widgets.noninteractive.bg_fill = PANEL;
    v.widgets.noninteractive.weak_bg_fill = PANEL;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = WIDGET;
    v.widgets.inactive.weak_bg_fill = WIDGET;
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT_TEXT.gamma_multiply(0.7));
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.active.weak_bg_fill = ACCENT_DIM;
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.corner_radius = r;
    v.widgets.open.bg_fill = HOVER;
    v.widgets.open.weak_bg_fill = HOVER;
    v.widgets.open.corner_radius = r;
    if high_contrast {
        // Stronger outlines and focus rings for low-vision users.
        let edge = Stroke::new(1.5_f32, Color32::from_rgb(200, 190, 195));
        v.widgets.noninteractive.bg_stroke = edge;
        v.widgets.inactive.bg_stroke = edge;
        v.widgets.hovered.bg_stroke = Stroke::new(2.0_f32, ACCENT_TEXT);
        v.widgets.active.bg_stroke = Stroke::new(2.0_f32, Color32::WHITE);
        v.selection.stroke = Stroke::new(2.0_f32, Color32::WHITE);
        v.window_stroke = Stroke::new(2.0_f32, ACCENT);
    }
    ctx.set_visuals_of(egui::Theme::Dark, v);

    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.button_padding = egui::vec2(12.0, 5.0);
        // Short, smooth transitions (collapsing sections, hover fades).
        s.animation_time = if animations_enabled() { 0.12 } else { 0.0 };
        s.scroll_animation = egui::style::ScrollAnimation::duration(0.15);
        s.text_styles
            .insert(egui::TextStyle::Body, egui::FontId::proportional(14.0));
        s.text_styles
            .insert(egui::TextStyle::Button, egui::FontId::proportional(14.0));
        s.text_styles
            .insert(egui::TextStyle::Heading, egui::FontId::proportional(18.0));
        s.text_styles.insert(
            egui::TextStyle::Name("Title".into()),
            egui::FontId::proportional(24.0),
        );
        s.text_styles
            .insert(egui::TextStyle::Small, egui::FontId::proportional(12.0));
        s.text_styles
            .insert(egui::TextStyle::Monospace, egui::FontId::monospace(13.0));
    });
}

/// A filled red pill button for the primary action of a view.
pub fn primary_button(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(text.into())
                .color(Color32::WHITE)
                .strong(),
        )
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(16))
        .min_size(egui::vec2(0.0, 30.0)),
    )
}

/// A wide pill button spanning the available width (bottom-of-page action).
pub fn wide_button(ui: &mut egui::Ui, text: impl Into<String>, enabled: bool) -> egui::Response {
    let w = ui.available_width();
    let (fill, color) = if enabled {
        (ACCENT, Color32::WHITE)
    } else {
        (WIDGET, MUTED)
    };
    let button = egui::Button::new(egui::RichText::new(text.into()).color(color).strong())
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(18));
    // add_sized centres the label inside the full-width pill.
    ui.add_enabled_ui(enabled, |ui| ui.add_sized([w, 36.0], button))
        .inner
}

/// A rounded card, like the panels in the main pages.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(CornerRadius::same(12))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A Command Deck panel: flat card with a 1px hairline border, no shadow.
/// Use for Dashboard panels; other pages keep their existing frames.
pub fn deck_panel<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(CornerRadius::same(6))
        .inner_margin(egui::Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// A Command Deck section label: small, faint, uppercase. egui has no
/// letter-spacing, so the deck's letterspacing is approximated with casing
/// alone; the underlying string is unchanged.
pub fn section_label(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.label(
        egui::RichText::new(text.into().to_uppercase())
            .color(FAINT)
            .size(11.0),
    )
}

/// A red warning banner.
pub fn warning_banner(ui: &mut egui::Ui, text: impl Into<String>) {
    egui::Frame::new()
        .fill(Color32::from_rgb(0x2C, 0x06, 0x03))
        .stroke(Stroke::new(1.0_f32, ACCENT))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(egui::RichText::new("⚠").color(ACCENT_TEXT));
                ui.label(
                    egui::RichText::new(text.into())
                        .color(Color32::from_rgb(0xF2, 0xB8, 0xAE))
                        .strong(),
                );
            });
        });
}

/// Page title at the top of the content area.
pub fn page_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.add_space(6.0);
    ui.label(egui::RichText::new(title).text_style(egui::TextStyle::Name("Title".into())));
    if !subtitle.is_empty() {
        ui.label(egui::RichText::new(subtitle).color(MUTED));
    }
    ui.add_space(8.0);
}

/// One entry in the left navigation column.
/// Windows' "Animation effects" setting (Settings > Accessibility > Visual
/// effects); animations are skipped when it is off.
pub fn animations_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(setupscope_core::platform::animations_enabled)
}

pub fn nav_item_with_hint(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    selected: bool,
    badge: Option<String>,
    hint: Option<&str>,
) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 38.0), egui::Sense::click());
    let accessible_label =
        hint.map_or_else(|| label.to_string(), |hint| format!("{label}. {hint}"));
    super::a11y::selectable(&resp, &accessible_label, selected);
    let painter = ui.painter_at(rect);
    let pill = rect.shrink2(egui::vec2(2.0, 2.0));
    // Hover and selection fade in and out instead of switching abruptly.
    let hover =
        ui.ctx()
            .animate_bool_with_time(resp.id.with("hover"), resp.hovered() && !selected, 0.10);
    let sel = ui
        .ctx()
        .animate_bool_with_time(resp.id.with("sel"), selected, 0.14);
    if hover > 0.0 {
        painter.rect_filled(
            pill,
            CornerRadius::same(18),
            Color32::from_rgb(22, 22, 22).gamma_multiply(hover),
        );
    }
    if sel > 0.0 {
        let w = pill.width() * (0.85 + 0.15 * sel);
        let grown = egui::Rect::from_min_size(pill.min, egui::vec2(w, pill.height()));
        painter.rect_filled(
            grown,
            CornerRadius::same(18),
            NAV_SELECTED.gamma_multiply(sel),
        );
    }
    let icon_color = if selected { Color32::WHITE } else { TEXT };
    painter.text(
        egui::pos2(rect.left() + 26.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(17.0),
        icon_color,
    );
    painter.text(
        egui::pos2(rect.left() + 54.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(14.0),
        if selected { Color32::WHITE } else { TEXT },
    );
    if let Some(b) = badge {
        // On the selected (red) pill the badge flips to white so it stays visible.
        let (fill, text) = if selected {
            (Color32::WHITE, ACCENT)
        } else {
            (ACCENT, Color32::WHITE)
        };
        let galley = painter.layout_no_wrap(b, egui::FontId::proportional(11.0), text);
        let w = galley.size().x + 12.0;
        let br = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 12.0 - w / 2.0, rect.center().y),
            egui::vec2(w, 18.0),
        );
        painter.rect_filled(br, CornerRadius::same(9), fill);
        painter.galley(br.center() - galley.size() / 2.0, galley, text);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if let Some(hint) = hint {
        resp.on_hover_text(hint)
    } else {
        resp
    }
}

/// A small coloured badge.
pub fn badge(ui: &mut egui::Ui, text: impl Into<String>, color: Color32) {
    let text = text.into();
    egui::Frame::new()
        .fill(color.gamma_multiply(0.18))
        .stroke(Stroke::new(1.0_f32, color.gamma_multiply(0.6)))
        .corner_radius(CornerRadius::same(4))
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(color).small().strong());
        });
}

/// Frame for the main content area of a page.
pub fn content_frame() -> egui::Frame {
    egui::Frame::new().fill(BG).inner_margin(egui::Margin {
        left: 18,
        right: 18,
        top: 4,
        bottom: 12,
    })
}

/// Frame for a side panel inside a page (details pane).
pub fn side_frame() -> egui::Frame {
    egui::Frame::new().fill(BG).inner_margin(egui::Margin {
        left: 6,
        right: 18,
        top: 4,
        bottom: 12,
    })
}
