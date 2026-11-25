//! Small shared widgets: Obsidian-style icon buttons and truncated labels.

use crate::theme::DARK;
use eframe::egui::{
    self, Color32, FontId, Galley,
    text::{LayoutJob, TextWrapping},
};
use std::sync::Arc;

/// Size of a toolbar icon button.
pub(crate) const ICON_BTN: f32 = 28.0;

/// Square icon button placed in the layout. `on` = highlighted (current view / toggled on).
pub(crate) fn icon_button(ui: &mut egui::Ui, icon: &str, tip: &str, on: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(ICON_BTN), egui::Sense::hover());
    icon_button_at(ui, rect, icon, tip, on)
}

/// Icon button at a fixed rect. Dimmed and inert when the `ui` is disabled.
pub(crate) fn icon_button_at(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: &str,
    tip: &str,
    on: bool,
) -> egui::Response {
    let p = &DARK;
    let resp = ui.allocate_rect(rect, egui::Sense::click());
    let enabled = ui.is_enabled();
    let hot = enabled && (on || resp.hovered());
    if hot {
        ui.painter().rect_filled(rect, 4.0, p.surface);
    }
    let color = if !enabled {
        p.muted.gamma_multiply(0.35)
    } else if hot {
        p.text
    } else {
        p.muted
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        FontId::proportional(16.0),
        color,
    );
    resp.on_hover_text(tip)
}

/// Single-line text cut off with "…" when wider than `max_w`.
pub(crate) fn truncated(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    color: Color32,
    max_w: f32,
) -> Arc<Galley> {
    let mut job = LayoutJob::simple_singleline(text.into(), FontId::proportional(size), color);
    job.wrap = TextWrapping::truncate_at_width(max_w.max(1.0));
    ui.fonts_mut(|f| f.layout_job(job))
}
