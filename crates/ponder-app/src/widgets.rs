//! Small drawing helpers shared by the panes.

use egui::{Color32, RichText, Ui};

use crate::{theme, view};

/// Chaos or divines; divine amounts in bold so the expensive end of a column stands out.
pub fn money(x: Option<f64>, divine: f64) -> RichText {
    let text = RichText::new(view::money(x, divine)).monospace();
    match x {
        None => text.weak(),
        Some(v) if divine > 0.0 && v.abs() >= divine => text.strong(),
        _ => text,
    }
}

/// A margin or saving: green when positive, red when negative.
pub fn signed(x: Option<f64>, divine: f64, dark: bool) -> RichText {
    let text = money(x, divine);
    match x {
        Some(v) if v > 0.0 => text.color(theme::good(dark)),
        Some(v) if v < 0.0 => text.color(theme::bad(dark)),
        _ => text,
    }
}

/// A small rounded label, tinted when given a colour.
pub fn chip(ui: &mut Ui, text: &str, tint: Option<Color32>) -> egui::Response {
    let visuals = ui.visuals();
    let (fill, stroke, color) = match tint {
        Some(tint) => (tint.gamma_multiply(0.14), tint.gamma_multiply(0.5), tint),
        None => (visuals.faint_bg_color, visuals.widgets.noninteractive.bg_stroke.color, visuals.text_color()),
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke))
        .corner_radius(9.0)
        .inner_margin(egui::Margin::symmetric(7, 1))
        .show(ui, |ui| ui.label(RichText::new(text).small().color(color)))
        .response
}

/// A headline number with its label underneath, in a tile of fixed size so a row of them wraps.
pub fn stat(ui: &mut Ui, label: &str, value: RichText, hint: &str) {
    let size = egui::vec2(104.0, 50.0);
    ui.allocate_ui(size, |ui| {
        egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(8.0).inner_margin(egui::Margin::symmetric(12, 7)).show(
            ui,
            |ui| {
                ui.set_min_size(size - egui::vec2(24.0, 14.0));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(value.size(17.0));
                    ui.label(RichText::new(label).small().weak());
                });
            },
        )
    })
    .response
    .on_hover_text(hint);
}

/// A section title inside a pane.
pub fn section(ui: &mut Ui, title: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(title.to_uppercase()).small().strong().weak());
}

/// A button that opens a trade search, with the url copied on right click.
pub fn trade_link(ui: &mut Ui, label: &str, url: Option<&str>) {
    let Some(url) = url else { return };
    let response = ui.button(format!("↗ {label}")).on_hover_text(format!("{url}\n\nright click to copy"));
    if response.clicked() {
        ui.ctx().open_url(egui::OpenUrl::new_tab(url));
    }
    if response.secondary_clicked() {
        ui.ctx().copy_text(url.to_string());
    }
}

/// A grey hint centred in the space, when there is nothing to show.
pub fn empty(ui: &mut Ui, icon: &str, text: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.3).min(120.0));
        ui.label(RichText::new(icon).size(30.0).weak());
        ui.label(RichText::new(text).weak());
    });
}

/// An operation's name for people: `essence:screaming-anger` reads "Essence · screaming anger".
pub fn operation(name: &str) -> String {
    let (kind, rest) = name.split_once(':').unwrap_or((name, ""));
    let mut kind: Vec<char> = kind.replace(['-', '_'], " ").chars().collect();
    if let Some(first) = kind.first_mut() {
        *first = first.to_ascii_uppercase();
    }
    let kind: String = kind.into_iter().collect();
    if rest.is_empty() { kind } else { format!("{kind} · {}", rest.replace(['-', '_'], " ")) }
}

/// Counts as bars behind their labels, the largest filling the width; hover for the share.
/// Returns the row clicked, if `clickable`.
pub fn bars(ui: &mut Ui, rows: &[(String, usize)], of: usize, color: Color32, clickable: bool) -> Option<usize> {
    let most = rows.iter().map(|(_, n)| *n).max().unwrap_or(1).max(1) as f32;
    let text = ui.visuals().text_color();
    let weak = ui.visuals().weak_text_color();
    let sense = if clickable { egui::Sense::click() } else { egui::Sense::hover() };
    let mut clicked = None;
    for (i, (label, n)) in rows.iter().enumerate() {
        let (rect, response) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 17.0), sense);
        let painter = ui.painter_at(rect);
        let mut bar = rect;
        bar.set_width(rect.width() * *n as f32 / most);
        painter.rect_filled(bar, 3.0, color.gamma_multiply(if response.hovered() && clickable { 0.4 } else { 0.22 }));
        let font = egui::FontId::proportional(11.5);
        painter.text(rect.left_center() + egui::vec2(5.0, 0.0), egui::Align2::LEFT_CENTER, label, font.clone(), text);
        painter.text(rect.right_center() - egui::vec2(5.0, 0.0), egui::Align2::RIGHT_CENTER, n.to_string(), font, weak);
        let hint = format!("{label}: {n} of {of} ({:.0}%)", 100.0 * *n as f64 / of.max(1) as f64);
        let response = response.on_hover_text(if clickable { format!("{hint}\nclick to filter by it") } else { hint });
        if response.clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// A need without its numbers: "+(100-114) to maximum Life" reads "maximum Life". A number
/// the sentence can't read without ("Skills have -(7-6) to Total Mana Cost") stays.
pub fn need(text: &str) -> String {
    let all: Vec<&str> = text.split_whitespace().collect();
    let number = |word: &str| word.starts_with(|c: char| c.is_ascii_digit() || matches!(c, '+' | '-' | '('));
    let needed = |i: usize| i > 0 && matches!(all[i - 1], "have" | "has") && all.get(i + 1) == Some(&"to");
    let words: Vec<&str> = all.iter().enumerate().filter(|&(i, word)| !number(word) || needed(i)).map(|(_, word)| *word).collect();
    let words = words.strip_prefix(&["to"]).unwrap_or(&words);
    let short = words.join(" ").replace("elemental resistances", "ele res").replace("increased ", "");
    if text.starts_with("any of: ") {
        return "a filler mod".into();
    }
    let short = short.replace("Added Small Passive Skills also grant: to ", "").replace("Added Small Passive Skills also grant: ", "");
    let short = short.replace("Added Small Passive Skills have ", "small passive ");
    let short = match short.strip_prefix("Adds to ") {
        Some(added) => format!("added {}", added.replace(" Damage to Attacks", " to attacks").replace(" Damage to Spells", " to spells")),
        None => short,
    };
    if short.is_empty() { text.to_string() } else { short }
}
