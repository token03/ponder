//! Small drawing helpers shared by the panes.

use egui::{Color32, RichText, Ui};

use crate::view;

/// A mod line's colour by role (`gear::Row.role`), readable on dark and light backgrounds.
pub fn role_color(role: &str, dark: bool) -> Color32 {
    let (d, l) = match role {
        "target" => ((120, 200, 255), (20, 90, 170)),
        "fractured" => ((230, 180, 90), (150, 95, 0)),
        "essence" => ((200, 140, 255), (110, 50, 170)),
        "bench" => ((140, 220, 160), (20, 120, 50)),
        "skipped" => ((150, 150, 150), (120, 120, 120)),
        _ => ((185, 185, 185), (90, 90, 90)), // filler
    };
    let (r, g, b) = if dark { d } else { l };
    Color32::from_rgb(r, g, b)
}

pub fn good(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(120, 210, 130) } else { Color32::from_rgb(20, 130, 40) }
}

pub fn bad(dark: bool) -> Color32 {
    if dark { Color32::from_rgb(240, 120, 110) } else { Color32::from_rgb(180, 30, 20) }
}

/// Chaos or divines; divine amounts in bold so the expensive end of a column stands out.
pub fn money(x: Option<f64>, divine: f64) -> RichText {
    let t = RichText::new(view::money(x, divine)).monospace();
    match x {
        None => t.weak(),
        Some(v) if divine > 0.0 && v.abs() >= divine => t.strong(),
        _ => t,
    }
}

/// A margin or saving: green when positive, red when negative.
pub fn signed(x: Option<f64>, divine: f64, dark: bool) -> RichText {
    let t = money(x, divine);
    match x {
        Some(v) if v > 0.0 => t.color(good(dark)),
        Some(v) if v < 0.0 => t.color(bad(dark)),
        _ => t,
    }
}

/// A small rounded label, e.g. a skill name.
pub fn chip(ui: &mut Ui, text: &str) {
    egui::Frame::new()
        .fill(ui.visuals().faint_bg_color)
        .stroke(ui.visuals().widgets.noninteractive.bg_stroke)
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).small());
        });
}

/// A link that opens in the browser, with a copy button next to it.
pub fn link_row(ui: &mut Ui, label: &str, url: Option<&str>) {
    let Some(url) = url else { return };
    ui.horizontal(|ui| {
        ui.hyperlink_to(label, url);
        if ui.small_button("copy").on_hover_text(url).clicked() {
            ui.ctx().copy_text(url.to_string());
        }
    });
}

/// A grey hint when there is nothing to show.
pub fn empty(ui: &mut Ui, text: &str) {
    ui.add_space(12.0);
    ui.vertical_centered(|ui| ui.label(RichText::new(text).weak()));
}
