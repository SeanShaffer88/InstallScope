//! The visual diff map: a colored bar of where changes landed, area chips to
//! filter by, and an expandable tree per area.

use super::theme;
use super::widgets::*;
use egui::{Color32, RichText};
use installscope_core::difftree::{self, Area, Change, Counts, DiffTree, Node};
use installscope_core::i18n::{t, tf};
use installscope_core::model::{human_size, ChangeSet};
use std::rc::Rc;

pub fn area_color(a: Area) -> Color32 {
    match a {
        Area::Startup => Color32::from_rgb(0xE5, 0x48, 0x4D),
        Area::Services => Color32::from_rgb(0xF7, 0x6B, 0x15),
        Area::Tasks => Color32::from_rgb(0xFF, 0xC5, 0x3D),
        Area::Shell => Color32::from_rgb(0xBF, 0x7A, 0xF0),
        Area::Windows => Color32::from_rgb(0xF0, 0x6A, 0x8A),
        Area::ProgramFiles => Color32::from_rgb(0x5B, 0x8D, 0xEF),
        Area::AppDataRoaming => Color32::from_rgb(0x12, 0xC2, 0xA9),
        Area::AppDataLocal => Color32::from_rgb(0x3F, 0xB0, 0xE0),
        Area::ProgramData => Color32::from_rgb(0x9E, 0x8C, 0xFC),
        Area::Shortcuts => Color32::from_rgb(0x4C, 0xC9, 0x80),
        Area::UserFolders => Color32::from_rgb(0x8E, 0xC0, 0x5A),
        Area::Temp => Color32::from_rgb(0x8B, 0x8D, 0x98),
        Area::MachineRegistry => Color32::from_rgb(0xE0, 0x6C, 0xB8),
        Area::UserRegistry => Color32::from_rgb(0xA3, 0x8C, 0xE8),
        Area::Other => Color32::from_rgb(0x9A, 0x92, 0x96),
    }
}

fn area_icon(a: Area) -> &'static str {
    match a {
        Area::Startup => "⚡",
        Area::Services => "⚙",
        Area::Tasks => "⏰",
        Area::Shell => "🖱",
        Area::Windows => "🖥",
        Area::ProgramFiles => "📦",
        Area::AppDataRoaming | Area::AppDataLocal => "👁",
        Area::ProgramData => "👥",
        Area::Shortcuts => "🔗",
        Area::UserFolders => "📁",
        Area::Temp => "🗑",
        Area::MachineRegistry | Area::UserRegistry => "🔑",
        Area::Other => "•",
    }
}

#[derive(Default)]
pub struct TreeView {
    pub filter: Option<Area>,
    pub search: String,
    /// The tree for the change set it was built from (by address and size),
    /// built once rather than every frame.
    cache: Option<(usize, usize, Rc<DiffTree>)>,
    /// The search-filtered tree for (search text, tree).
    filtered: Option<(String, Rc<DiffTree>)>,
}

impl TreeView {
    fn tree(&mut self, cs: &ChangeSet) -> Rc<DiffTree> {
        let sig = (cs as *const ChangeSet as usize, cs.total());
        match &self.cache {
            Some((a, n, t)) if (*a, *n) == sig => t.clone(),
            _ => {
                let t = Rc::new(difftree::build(cs, &difftree::Places::current()));
                self.cache = Some((sig.0, sig.1, t.clone()));
                self.filtered = None;
                t
            }
        }
    }
}

/// Keep only nodes whose path contains `needle` (and the branches to them).
fn prune(n: &Node, needle: &str) -> Option<Node> {
    if n.full.to_lowercase().contains(needle) && n.children.is_empty() {
        return Some(n.clone());
    }
    let children: Vec<Node> = n.children.iter().filter_map(|c| prune(c, needle)).collect();
    if children.is_empty() && !n.full.to_lowercase().contains(needle) {
        return None;
    }
    let mut out = n.clone();
    if !children.is_empty() {
        out.children = children;
    }
    Some(out)
}

fn counts_text(c: &Counts) -> String {
    let mut parts = Vec::new();
    if c.added > 0 {
        parts.push(format!("+{}", c.added));
    }
    if c.changed > 0 {
        parts.push(format!("~{}", c.changed));
    }
    if c.removed > 0 {
        parts.push(format!("−{}", c.removed));
    }
    if c.bytes > 0 {
        parts.push(human_size(c.bytes));
    }
    parts.join("  ")
}

fn change_color(c: Option<Change>) -> Color32 {
    match c {
        Some(Change::Added) => theme::ADDED,
        Some(Change::Changed) => theme::CHANGED,
        Some(Change::Removed) => theme::REMOVED,
        None => theme::MUTED,
    }
}

/// The colored overview bar; clicking a segment filters to that area.
fn overview_bar(ui: &mut egui::Ui, tree: &DiffTree, filter: &mut Option<Area>) {
    let total = tree.total.total().max(1) as f32;
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 14.0), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 7.0, theme::LINE);
    let mut x = rect.left();
    let min_w = 6.0;
    let n = tree.areas.len() as f32;
    let spare = (width - min_w * n).max(0.0);
    for a in &tree.areas {
        let Some(area) = a.area else { continue };
        let share = a.root.counts.total() as f32 / total;
        let w = min_w + spare * share;
        let seg =
            egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(w, rect.height()));
        let id = ui.id().with(("seg", area as u8));
        let r = ui.interact(seg, id, egui::Sense::click());
        let dim = filter.is_some_and(|f| f != area);
        let mut col = area_color(area);
        if dim {
            col = col.gamma_multiply(0.35);
        }
        if r.hovered() {
            col = col.gamma_multiply(1.15);
        }
        painter.rect_filled(seg.shrink2(egui::vec2(0.5, 0.0)), 3.0, col);
        if r.clicked() {
            *filter = if *filter == Some(area) {
                None
            } else {
                Some(area)
            };
        }
        super::a11y::explain(
            r,
            &format!("{} — {}", t(area.label()), counts_text(&a.root.counts)),
        );
        x += w;
    }
}

fn node_ui(ui: &mut egui::Ui, n: &Node, color: Color32, depth: usize, open_all: bool, icon: &str) {
    const MAX_CHILDREN: usize = 300;
    if n.children.is_empty() {
        ui.horizontal(|ui| {
            ui.label(RichText::new("⏺").small().color(change_color(n.change)));
            super::a11y::explain(
                ui.add(
                    egui::Label::new(RichText::new(&n.name).color(theme::TEXT))
                        .truncate()
                        .selectable(true),
                ),
                &n.full,
            );
            if n.counts.bytes > 0 {
                small_muted(ui, human_size(n.counts.bytes));
            }
        });
        return;
    }
    let header = RichText::new(format!("{icon}{}   {}", n.name, counts_text(&n.counts)))
        .color(if depth == 0 { color } else { theme::TEXT });
    let resp = egui::CollapsingHeader::new(header)
        .id_salt(("dt", &n.full, depth))
        .default_open(open_all || depth == 0 && n.counts.total() <= 40)
        .show(ui, |ui| {
            for c in n.children.iter().take(MAX_CHILDREN) {
                node_ui(ui, c, color, depth + 1, open_all, "");
            }
            if n.children.len() > MAX_CHILDREN {
                small_muted(
                    ui,
                    tf(
                        "… and {n} more",
                        &[("n", (n.children.len() - MAX_CHILDREN).to_string())],
                    ),
                );
            }
        });
    super::a11y::explain(resp.header_response, &n.full);
}

/// The whole map for `cs`.
pub fn diff_tree_ui(ui: &mut egui::Ui, view: &mut TreeView, cs: &ChangeSet) {
    let tree = view.tree(cs);
    if tree.areas.is_empty() {
        muted(ui, t("Nothing changed."));
        return;
    }
    overview_bar(ui, &tree, &mut view.filter);
    ui.add_space(6.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        for a in &tree.areas {
            let Some(area) = a.area else { continue };
            let on = view.filter == Some(area);
            let col = area_color(area);
            let text = RichText::new(format!(
                "{} {}  {}",
                area_icon(area),
                t(area.label()),
                a.root.counts.total()
            ))
            .color(if on { Color32::BLACK } else { col });
            let b = egui::Button::new(text)
                .fill(if on { col } else { col.gamma_multiply(0.12) })
                .stroke(egui::Stroke::new(1.0_f32, col.gamma_multiply(0.6)))
                .corner_radius(10.0);
            if super::a11y::explain(ui.add(b), &t(area.hint())).clicked() {
                view.filter = if on { None } else { Some(area) };
            }
        }
    });
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        super::a11y::explain(
            search_box(ui, &mut view.search, "Find in changes", 260.0),
            &t("Show only the changes whose path contains this text."),
        );
        ui.label(RichText::new("⏺").color(theme::ADDED));
        small_muted(ui, t("added"));
        ui.label(RichText::new("⏺").color(theme::CHANGED));
        small_muted(ui, t("changed"));
        ui.label(RichText::new("⏺").color(theme::REMOVED));
        small_muted(ui, t("removed"));
    });
    ui.add_space(4.0);
    let needle = view.search.trim().to_lowercase();
    let shown: Rc<DiffTree> = if needle.is_empty() {
        tree
    } else {
        match &view.filtered {
            Some((s, f)) if *s == needle => f.clone(),
            _ => {
                let areas = tree
                    .areas
                    .iter()
                    .filter_map(|a| {
                        prune(&a.root, &needle)
                            .map(|root| difftree::AreaTree { area: a.area, root })
                    })
                    .collect();
                let f = Rc::new(DiffTree {
                    areas,
                    total: tree.total,
                });
                view.filtered = Some((needle.clone(), f.clone()));
                f
            }
        }
    };
    for a in &shown.areas {
        let Some(area) = a.area else { continue };
        if view.filter.is_some_and(|f| f != area) {
            continue;
        }
        let col = area_color(area);
        egui::Frame::new()
            .fill(col.gamma_multiply(0.06))
            .stroke(egui::Stroke::new(1.0_f32, col.gamma_multiply(0.35)))
            .corner_radius(8.0)
            .inner_margin(egui::Margin::symmetric(10, 6))
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                let icon = format!("{}  ", area_icon(area));
                node_ui(
                    ui,
                    &a.root,
                    col,
                    0,
                    !needle.is_empty() || view.filter.is_some(),
                    &icon,
                );
            });
        ui.add_space(4.0);
    }
}

/// "Map" and "List" views of the same changes. The list can remove entries
/// when `editable` (used to drop noise before saving).
pub fn map_or_list(
    ui: &mut egui::Ui,
    view: &mut TreeView,
    show_list: &mut bool,
    cs: &mut ChangeSet,
    editable: bool,
) {
    ui.horizontal(|ui| {
        if super::a11y::explain(
            ui.selectable_label(!*show_list, format!("🗺 {}", t("Map"))),
            &t("Show the changes as a colored map, grouped by area of the PC."),
        )
        .clicked()
        {
            *show_list = false;
        }
        if super::a11y::explain(
            ui.selectable_label(*show_list, format!("☰ {}", t("List"))),
            &t("Show the changes as a plain list, grouped by the kind of change."),
        )
        .clicked()
        {
            *show_list = true;
        }
        if editable && !*show_list {
            small_muted(ui, t("Switch to List to remove entries."));
        }
    });
    ui.add_space(4.0);
    if *show_list {
        super::logs::changes_ui(ui, cs, editable);
    } else {
        diff_tree_ui(ui, view, cs);
    }
}
