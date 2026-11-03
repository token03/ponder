//! Colours, type and spacing, for the dark and light themes.

use egui::{Color32, CornerRadius, FontId, Stroke, TextStyle, Theme, Visuals};

/// The accent: selections, the primary button and the active tab.
pub fn accent(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(222, 178, 92) } else { Color32::from_rgb(160, 110, 20) }
}

pub fn good(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(120, 210, 140) } else { Color32::from_rgb(20, 130, 50) }
}

pub fn bad(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(240, 120, 110) } else { Color32::from_rgb(185, 35, 25) }
}

pub fn info(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(120, 180, 250) } else { Color32::from_rgb(25, 95, 180) }
}

/// A mod line's colour by role (`gear::Row.role`).
pub fn role(role: &str, dark: bool) -> Color32 {
    let (d, l) = match role {
        "target" => ((136, 136, 255), (50, 50, 190)),
        "fractured" => ((166, 141, 92), (120, 90, 30)),
        "bench" => ((150, 220, 170), (20, 120, 50)),
        _ => ((165, 165, 165), (95, 95, 95)),
    };
    let (r, g, b) = if dark { d } else { l };
    Color32::from_rgb(r, g, b)
}

/// The name colour of a magic item.
pub fn magic(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(136, 136, 255) } else { Color32::from_rgb(50, 50, 190) }
}

/// The name colour of a rare item.
pub fn rare(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(255, 231, 119) } else { Color32::from_rgb(140, 105, 0) }
}

pub fn apply(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Dark, dark());
    ctx.set_visuals_of(Theme::Light, light());
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 4.0);
        style.spacing.interact_size.y = 22.0;
        style.spacing.scroll.floating = true;
        style.interaction.selectable_labels = false;
        style.interaction.tooltip_delay = 0.05;
        style.interaction.show_tooltips_only_when_still = false;
        for (text, size) in [
            (TextStyle::Heading, 19.0),
            (TextStyle::Body, 13.5),
            (TextStyle::Button, 13.5),
            (TextStyle::Small, 11.0),
            (TextStyle::Monospace, 12.5),
        ] {
            let family = if text == TextStyle::Monospace { FontId::monospace(size) } else { FontId::proportional(size) };
            style.text_styles.insert(text, family);
        }
    });
}

fn rounded(visuals: &mut Visuals) {
    let radius = CornerRadius::same(6);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = radius;
    }
    visuals.window_corner_radius = CornerRadius::same(10);
    visuals.menu_corner_radius = CornerRadius::same(8);
}

fn dark() -> Visuals {
    let mut visuals = Visuals::dark();
    let gold = accent(true);
    visuals.panel_fill = Color32::from_rgb(23, 25, 30);
    visuals.window_fill = Color32::from_rgb(29, 32, 38);
    visuals.extreme_bg_color = Color32::from_rgb(15, 16, 20);
    visuals.faint_bg_color = Color32::from_rgb(29, 32, 38);
    visuals.code_bg_color = Color32::from_rgb(15, 16, 20);
    visuals.window_stroke = Stroke::new(1.0, Color32::from_rgb(48, 52, 60));
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(44, 48, 56));
    visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(38, 42, 50);
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(38, 42, 50);
    visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(50, 55, 65);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, gold.gamma_multiply(0.6));
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, gold);
    visuals.selection.bg_fill = Color32::from_rgb(84, 66, 32);
    visuals.selection.stroke = Stroke::new(1.0, gold);
    visuals.hyperlink_color = info(true);
    rounded(&mut visuals);
    visuals
}

fn light() -> Visuals {
    let mut visuals = Visuals::light();
    let gold = accent(false);
    visuals.panel_fill = Color32::from_rgb(246, 245, 242);
    visuals.window_fill = Color32::from_rgb(252, 251, 249);
    visuals.faint_bg_color = Color32::from_rgb(238, 236, 231);
    visuals.extreme_bg_color = Color32::from_rgb(255, 255, 255);
    visuals.selection.bg_fill = Color32::from_rgb(242, 222, 176);
    visuals.selection.stroke = Stroke::new(1.0, gold);
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, gold.gamma_multiply(0.6));
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, gold);
    visuals.hyperlink_color = info(false);
    rounded(&mut visuals);
    visuals
}
