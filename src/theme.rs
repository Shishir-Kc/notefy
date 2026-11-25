//! Look & feel: color palette, egui style, bundled fonts and Nerd Font icons.

use crate::live;
use eframe::egui::{self, Color32, CornerRadius, FontId, Stroke, TextStyle};

pub(crate) struct Palette {
    pub(crate) bg: Color32,
    pub(crate) panel: Color32,
    pub(crate) surface: Color32,
    pub(crate) text: Color32,
    pub(crate) muted: Color32,
    pub(crate) accent: Color32,
    pub(crate) border: Color32,
    pub(crate) link: Color32,
}

pub(crate) const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x1e, 0x1e, 0x1e),      // editor page
    panel: Color32::from_rgb(0x26, 0x26, 0x26),   // sidebar, tab strip, popups
    surface: Color32::from_rgb(0x36, 0x36, 0x36), // hover / selected rows
    text: Color32::from_rgb(0xf3, 0xef, 0xe6),    // milky white
    muted: Color32::from_rgb(0x9a, 0x96, 0x8e),
    accent: Color32::from_rgb(0xe8, 0xe2, 0xd4),
    border: Color32::from_rgb(0x33, 0x33, 0x33),
    link: Color32::from_rgb(0xd9, 0xc4, 0x9a), // warm latte
};

pub(crate) fn apply_theme(ctx: &egui::Context) {
    let p = &DARK;
    let mut v = egui::Visuals::dark();
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.panel;
    v.code_bg_color = p.surface;
    // No override_text_color: it would also repaint links. Text color comes from fg_stroke.
    v.override_text_color = None;
    v.hyperlink_color = p.link;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.selection.bg_fill = p.surface;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.text_cursor.stroke = Stroke::new(2.0, p.accent);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::same(4);
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    v.widgets.inactive.bg_fill = p.surface;
    v.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    v.widgets.inactive.bg_stroke = Stroke::NONE;
    v.widgets.hovered.bg_fill = p.surface;
    v.widgets.hovered.weak_bg_fill = p.surface;
    v.widgets.hovered.bg_stroke = Stroke::NONE;
    v.widgets.active.bg_fill = p.surface;
    v.widgets.active.weak_bg_fill = p.surface;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, p.accent);
    v.widgets.active.fg_stroke = Stroke::new(2.0, p.text);
    v.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    ctx.set_visuals(v);

    ctx.global_style_mut(|s| {
        s.text_styles
            .insert(TextStyle::Body, FontId::proportional(15.0));
        s.text_styles
            .insert(TextStyle::Heading, FontId::proportional(30.0));
        s.text_styles
            .insert(TextStyle::Monospace, FontId::monospace(14.5));
        s.text_styles
            .insert(TextStyle::Button, FontId::proportional(14.0));
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(8.0, 4.0);
    });
}

// JetBrainsMono Nerd Font (Omarchy's default), bundled so icons always render.
pub(crate) const NERD_FONT: &[u8] = include_bytes!("../assets/JetBrainsMonoNerdFont-Regular.ttf");
pub(crate) const NERD_FONT_BOLD: &[u8] = include_bytes!("../assets/JetBrainsMonoNerdFont-Bold.ttf");
pub(crate) const NERD_FONT_ITALIC: &[u8] =
    include_bytes!("../assets/JetBrainsMonoNerdFont-Italic.ttf");

/// Codicons (VS Code's line icon set, shipped in the bundled Nerd Font).
pub(crate) mod icon {
    pub const FILES: &str = "\u{eaf0}";
    pub const SEARCH: &str = "\u{ea6d}";
    pub const BOOKMARK: &str = "\u{eaa5}";
    pub const READ: &str = "\u{eaa4}";
    pub const SIDEBAR: &str = "\u{ebf3}";
    pub const NEW_NOTE: &str = "\u{ea7f}";
    pub const NEW_FOLDER: &str = "\u{ea80}";
    pub const SORT: &str = "\u{eb55}";
    pub const COLLAPSE_ALL: &str = "\u{eac5}";
    pub const EXPAND_ALL: &str = "\u{eb95}";
    pub const CHEVRON: &str = "\u{eab6}";
    pub const CHEVRON_DOWN: &str = "\u{eab4}";
    pub const CLOSE: &str = "\u{ea76}";
    pub const ADD: &str = "\u{ea60}";
    pub const BACK: &str = "\u{ea9b}";
    pub const FORWARD: &str = "\u{ea9c}";
    pub const MORE: &str = "\u{ea7c}";
    pub const FILE: &str = "\u{ea7b}";
    pub const FOLDER: &str = "\u{ea83}";
    pub const VAULT: &str = "\u{eb46}";
    pub const REFRESH: &str = "\u{eb37}";
    pub const TRASH: &str = "\u{ea81}";
    pub const CLOCK: &str = "\u{ea82}";
    pub const IMAGE: &str = "\u{eaea}";
    pub const TABLE: &str = "\u{ebb7}";
    pub const SHARE: &str = "\u{eb18}";
    pub const LINK: &str = "\u{eb15}";
    pub const COPY: &str = "\u{ebcc}";
    pub const LOCK: &str = "\u{ea75}";
    pub const GLOBE: &str = "\u{eb01}";
    /// GitHub Octicons "git-branch": the graph view.
    pub const GRAPH: &str = "\u{f418}";
}

pub(crate) fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        ("jbm-nerd", NERD_FONT),
        ("jbm-bold", NERD_FONT_BOLD),
        ("jbm-italic", NERD_FONT_ITALIC),
    ] {
        fonts.font_data.insert(
            name.into(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    let fallback = fonts.families[&egui::FontFamily::Proportional].clone();
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(fam)
            .or_default()
            .insert(0, "jbm-nerd".into());
    }
    for (fam, font) in [
        (live::bold_family(), "jbm-bold"),
        (live::italic_family(), "jbm-italic"),
    ] {
        let mut list = vec![font.to_string(), "jbm-nerd".to_string()];
        list.extend(fallback.iter().cloned());
        fonts.families.insert(fam, list);
    }
    ctx.set_fonts(fonts);
}
