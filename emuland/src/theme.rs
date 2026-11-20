//! Le style : noir, blanc et gris. Rien d'autre.

use eframe::egui::{self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, Stroke};
use std::sync::Arc;

pub const BG: Color32 = Color32::from_rgb(0, 0, 0);
pub const SURFACE: Color32 = Color32::from_rgb(12, 12, 12);
pub const HOVER: Color32 = Color32::from_rgb(28, 28, 28);
pub const LINE: Color32 = Color32::from_rgb(38, 38, 38);
pub const INK: Color32 = Color32::from_rgb(240, 240, 240);
pub const MUTED: Color32 = Color32::from_rgb(140, 140, 140);
pub const DIM: Color32 = Color32::from_rgb(85, 85, 85);
// seules couleurs « d'état », discrètes
pub const OK: Color32 = Color32::from_rgb(120, 200, 140);
pub const WARN: Color32 = Color32::from_rgb(210, 170, 80);
pub const BAD: Color32 = Color32::from_rgb(220, 110, 110);

/// Famille de police en gras (Segoe UI Bold si elle existe).
pub fn bold() -> FontFamily {
    FontFamily::Name("bold".into())
}

pub fn apply(ctx: &egui::Context) {
    // Segoe UI (présente sur tous les Windows) devant la police intégrée.
    let mut fonts = FontDefinitions::default();
    let default_prop = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let mut regular = Vec::new();
    if let Ok(data) = std::fs::read("C:/Windows/Fonts/segoeui.ttf") {
        fonts.font_data.insert("segoe".into(), Arc::new(FontData::from_owned(data)));
        regular.push("segoe".to_string());
    }
    let mut bold_first = Vec::new();
    if let Ok(data) = std::fs::read("C:/Windows/Fonts/segoeuib.ttf") {
        fonts.font_data.insert("segoe-bold".into(), Arc::new(FontData::from_owned(data)));
        bold_first.push("segoe-bold".to_string());
    }
    let mut prop = regular.clone();
    prop.extend(default_prop.clone());
    fonts.families.insert(FontFamily::Proportional, prop);
    let mut bold_family = if bold_first.is_empty() { regular } else { bold_first };
    bold_family.extend(default_prop);
    fonts.families.insert(bold(), bold_family);
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = BG;
    v.faint_bg_color = SURFACE;
    v.override_text_color = Some(INK);
    v.selection.bg_fill = Color32::from_rgb(60, 60, 60);
    v.selection.stroke = Stroke::new(1.0, INK);
    v.hyperlink_color = INK;
    let radius = CornerRadius::same(4);
    for w in [&mut v.widgets.noninteractive, &mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active] {
        w.corner_radius = radius;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.inactive.bg_fill = BG;
    v.widgets.inactive.weak_bg_fill = BG;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, LINE);
    v.widgets.hovered.bg_fill = HOVER;
    v.widgets.hovered.weak_bg_fill = HOVER;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, DIM);
    v.widgets.active.bg_fill = HOVER;
    v.widgets.active.weak_bg_fill = HOVER;
    v.widgets.active.bg_stroke = Stroke::new(1.0, MUTED);
    ctx.set_visuals(v);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(10.0, 8.0);
        s.spacing.button_padding = egui::vec2(14.0, 6.0);
    });
}
