//! One craft: the item and its numbers side by side, then who wears it, how to make it and
//! what it was read from.

use egui::{RichText, Ui};
use ponder::gear::{Row, Side};
use ponder::pipeline::{Depth, Estimate};

use super::recipe;
use crate::state::State;
use crate::tags::{Kind, Tag};
use crate::widgets::{bars, chip, money, section, signed, trade_link};
use crate::{theme, view};

enum Action {
    Select,
    Fetch,
    Deep,
    Copy(String),
    Filter(Tag),
}

pub fn ui(ui: &mut Ui, state: &mut State, index: usize) {
    let estimate = &state.estimates()[index];
    let busy = Busy {
        prices: !state.has_cookie || state.jobs.is_running(&format!("trade prices #{index}")),
        deep: state.jobs.is_running("deep solve"),
        provisional: state.quick.contains(&estimate.key),
    };
    let most_demand = state.estimates().iter().map(|e| e.demand).max().unwrap_or(1);
    let bench = recipe::bench_text(state.db.clone());
    let cx = recipe::Context { base: &estimate.base, item_level: estimate.item_level, bench: &bench };
    let action = egui::ScrollArea::vertical()
        .id_salt(("craft", index))
        .auto_shrink(false)
        .show(ui, |ui| page(ui, estimate, &cx, state.divine(), most_demand, &busy))
        .inner;
    match action {
        Some(Action::Select) => state.select(index),
        Some(Action::Fetch) => {
            state.select(index);
            state.price_selected();
        }
        Some(Action::Deep) => {
            state.select(index);
            state.deep_solve();
        }
        Some(Action::Copy(text)) => {
            ui.ctx().copy_text(text);
            state.announce("recipe copied", false);
        }
        Some(Action::Filter(tag)) => {
            state.saved.filters.add(tag);
            state.refilter();
        }
        None => {}
    }
}

struct Busy {
    prices: bool,
    deep: bool,
    provisional: bool,
}

fn page(ui: &mut Ui, estimate: &Estimate, cx: &recipe::Context, divine: f64, most_demand: usize, busy: &Busy) -> Option<Action> {
    let mut action = None;
    let dark = ui.visuals().dark_mode;
    ui.horizontal_wrapped(|ui| {
        if ui.add(egui::Label::new(RichText::new(&estimate.title).size(17.0).strong()).sense(egui::Sense::click())).clicked() {
            action = Some(Action::Select);
        }
        ui.label(RichText::new(format!("{} · ilvl {}", estimate.base, estimate.item_level)).weak());
        if busy.provisional {
            chip(ui, "provisional", Some(theme::info(dark)));
        }
        if estimate.summary.as_ref().is_some_and(|summary| summary.depth == Depth::Deep) {
            chip(ui, "★ every mechanic", Some(theme::accent(dark)));
        }
    });
    if let Some(error) = estimate.failure() {
        ui.label(RichText::new(format!("⚠ {error}")).color(ui.visuals().error_fg_color));
    }
    let wide = ui.available_width() > 620.0;
    let card_width = if wide { (ui.available_width() * 0.5).min(420.0) } else { ui.available_width() };
    let mut numbers_and_actions = |ui: &mut Ui| {
        numbers(ui, estimate, divine);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            trade_link(ui, "item", estimate.trade_url.as_deref());
            trade_link(ui, "base", estimate.base_url.as_deref());
            if ui.add_enabled(!busy.prices, egui::Button::new("⟳ prices")).on_hover_text("look the item and base up on trade").clicked() {
                action = Some(Action::Fetch);
            }
            let can_deep = estimate.summary.as_ref().is_some_and(|summary| summary.depth == Depth::Ladder) && !busy.provisional;
            let label = if busy.deep { "★ …" } else { "★ every mechanic" };
            if ui
                .add_enabled(can_deep && !busy.deep, egui::Button::new(label))
                .on_hover_text("One-time check with memory strands, Dominance, Hinekora's Lock and the Recombinator: up to a couple of minutes and a few GB, saved for later scans")
                .clicked()
            {
                action = Some(Action::Deep);
            }
            if let Some(summary) = &estimate.summary {
                if ui.button("📋 recipe").on_hover_text("copy the recipe as text").clicked() {
                    action = Some(Action::Copy(recipe::text(summary, cx)));
                }
                if ui.button("📋 Kalandralang").on_hover_text("copy the recipe as a Kalandralang-style script (may need edits to run)").clicked() {
                    action = Some(Action::Copy(recipe::script(summary, cx)));
                }
            }
        });
    };
    if wide {
        ui.horizontal_top(|ui| {
            ui.allocate_ui_with_layout(egui::vec2(card_width, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                item_card(ui, &estimate.base, &estimate.rows)
            });
            ui.vertical(&mut numbers_and_actions);
        });
    } else {
        item_card(ui, &estimate.base, &estimate.rows);
        numbers_and_actions(ui);
    }
    if let Some(tag) = wearers(ui, estimate, most_demand) {
        action = Some(Action::Filter(tag));
    }
    if let Some(summary) = &estimate.summary {
        section(ui, "Recipe");
        if let Some(name) = recipe::ui(ui, summary, cx, divine) {
            action = Some(Action::Filter(Tag::new(Kind::Uses, name)));
        }
    }
    source(ui, estimate);
    action
}

/// The costs as a two-column table: what each number is, and the number.
fn numbers(ui: &mut Ui, estimate: &Estimate, divine: f64) {
    let dark = ui.visuals().dark_mode;
    let time = estimate.clicking_hours().map(view::hours).unwrap_or_else(|| "–".into());
    let base = estimate.base_price.map(|price| price.0);
    egui::Grid::new(("numbers", &estimate.title, estimate.demand)).num_columns(2).spacing([14.0, 3.0]).show(ui, |ui| {
        let rows: [(&str, RichText, &str); 6] = [
            (
                "⚒ craft",
                money(estimate.craft_cost(), divine).size(16.0),
                "expected chaos per finished item: currency, bench crafts and clicking time",
            ),
            ("⌛ time", RichText::new(time).monospace(), "expected time at the bench"),
            ("◻ base", money(base, divine), "the starting base on trade"),
            ("Σ total", money(estimate.total(), divine), "crafting and the bases it uses"),
            ("🏷 market", money(estimate.market.map(|price| price.0), divine), "what the finished item sells for"),
            ("± margin", signed(estimate.margin(), divine, dark), "market minus total"),
        ];
        for (label, value, hint) in rows {
            ui.label(RichText::new(label).weak()).on_hover_text(hint);
            ui.label(value);
            ui.end_row();
        }
    });
    for error in [&estimate.market_error, &estimate.base_error].into_iter().flatten() {
        ui.label(RichText::new(error).small().color(ui.visuals().warn_fg_color));
    }
}

/// The item as the game shows it: prefixes above suffixes, each mod with the tiers it accepts.
pub fn item_card(ui: &mut Ui, base: &str, rows: &[Row]) {
    let dark = ui.visuals().dark_mode;
    egui::Frame::new()
        .fill(if dark { egui::Color32::from_rgb(12, 11, 10) } else { ui.visuals().extreme_bg_color })
        .stroke(egui::Stroke::new(1.0, theme::rare(dark).gamma_multiply(0.35)))
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| ui.label(RichText::new(base).size(14.5).color(theme::rare(dark))));
            for (side, name) in [(Side::Prefix, "prefixes"), (Side::Suffix, "suffixes")] {
                let mine: Vec<&Row> = rows.iter().filter(|row| row.side == side).collect();
                ui.add(egui::Separator::default().spacing(4.0));
                ui.label(RichText::new(name).small().weak());
                if mine.is_empty() {
                    ui.label(RichText::new("any").small().weak());
                }
                for row in mine {
                    mod_row(ui, row);
                }
            }
        });
}

fn mod_row(ui: &mut Ui, row: &Row) {
    let dark = ui.visuals().dark_mode;
    ui.horizontal(|ui| {
        let tag = match row.role {
            "fractured" => "frac",
            "bench" => "bench",
            "filler" => "any",
            _ => "",
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(tiers) = tiers(&row.tier) {
                ui.label(RichText::new(tiers).small().monospace().weak()).on_hover_text("tiers accepted; T1 is the best");
            }
            if !tag.is_empty() {
                ui.label(RichText::new(tag).small().color(theme::role(row.role, dark)));
            }
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(&row.text).color(theme::role(row.role, dark))).truncate());
            });
        });
    });
}

/// "T3+" (tier 3 or better) as "T1–3".
fn tiers(tier: &str) -> Option<String> {
    let worst: u32 = tier.strip_prefix('T')?.strip_suffix('+')?.parse().ok()?;
    Some(if worst == 1 { "T1".into() } else { format!("T1–{worst}") })
}

/// Who wears it: how many against the most worn craft, and their ascendancies, skills and bases.
fn wearers(ui: &mut Ui, estimate: &Estimate, most_demand: usize) -> Option<Tag> {
    let dark = ui.visuals().dark_mode;
    section(ui, "Worn by");
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("👥 {}", estimate.demand)).strong()).on_hover_text("ladder characters wearing it");
        let share = estimate.demand as f32 / most_demand.max(1) as f32;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width().min(220.0), 8.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
        let mut filled = rect;
        filled.set_width(rect.width() * share);
        ui.painter().rect_filled(filled, 3.0, theme::info(dark));
        if estimate.items > 0 {
            ui.label(RichText::new(format!("{} items", estimate.items)).small().weak()).on_hover_text("items read off the ladder");
        }
    });
    let columns: Vec<(Kind, &[(String, usize)])> = [
        (Kind::Ascendancy, estimate.classes.as_slice()),
        (Kind::Skill, estimate.skills.as_slice()),
        (Kind::Base, estimate.worn_bases.as_slice()),
    ]
    .into_iter()
    .filter(|(_, rows)| !rows.is_empty())
    .collect();
    if columns.is_empty() {
        return None;
    }
    let mut picked = None;
    ui.columns(columns.len(), |uis| {
        for (ui, (kind, rows)) in uis.iter_mut().zip(&columns) {
            let title = if *kind == Kind::Base && estimate.bases > 1 {
                format!("base · {} share its mods", estimate.bases)
            } else {
                kind.name().to_string()
            };
            ui.label(RichText::new(title).small().weak());
            let rows = &rows[..rows.len().min(6)];
            if let Some(i) = bars(ui, rows, estimate.demand, kind.colour(dark), true) {
                picked = Some(Tag::new(*kind, rows[i].0.clone()));
            }
        }
    });
    picked
}

/// What the craft was read from: worn items, and the needs taken from them.
fn source(ui: &mut Ui, estimate: &Estimate) {
    if estimate.examples.is_empty() {
        return;
    }
    let dark = ui.visuals().dark_mode;
    egui::CollapsingHeader::new(RichText::new(format!("FROM {} WORN ITEMS", estimate.items)).small().strong().weak())
        .id_salt(("source", &estimate.title, estimate.demand))
        .show(ui, |ui| {
            ui.label(RichText::new("Items read off the ladder → the mods they share become the needs.").small().weak());
            ui.horizontal_wrapped(|ui| {
                for example in &estimate.examples {
                    egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(6.0).inner_margin(6).show(ui, |ui| {
                        ui.set_width(230.0);
                        ui.label(RichText::new(&example.base).small().color(theme::rare(dark)));
                        for prefix in [true, false] {
                            for worn in example.mods.iter().filter(|worn| worn.prefix == prefix) {
                                let tier = worn.tier.map_or(String::new(), |tier| format!("T{tier} "));
                                let text = format!("{}{tier}{}", if prefix { "P " } else { "S " }, worn.text);
                                ui.label(RichText::new(text).small()).on_hover_text(worn.source);
                            }
                        }
                    });
                }
                ui.label(RichText::new("⏵").size(20.0).weak());
                egui::Frame::new().fill(ui.visuals().faint_bg_color).corner_radius(6.0).inner_margin(6).show(ui, |ui| {
                    ui.set_width(200.0);
                    ui.label(RichText::new("needs").small().weak());
                    for target in &estimate.targets {
                        ui.label(RichText::new(target).small().color(theme::role("target", dark)));
                    }
                });
            });
        });
}
