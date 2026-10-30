use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};
use serde::{Deserialize, Serialize};

use crate::{
    state::State,
    view::{self, Col},
    widgets::{chip, empty, link_row, money, role_color, signed},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pane {
    Crafts,
    Overview,
    Item,
    Recipe,
    Log,
}

impl Pane {
    pub const ALL: [Self; 5] = [Self::Crafts, Self::Overview, Self::Item, Self::Recipe, Self::Log];
    pub fn title(self) -> &'static str {
        match self {
            Self::Crafts => "Crafts",
            Self::Overview => "Overview",
            Self::Item => "Item",
            Self::Recipe => "Policy",
            Self::Log => "Log",
        }
    }
}

pub fn default_tree() -> egui_tiles::Tree<Pane> {
    let mut tiles = egui_tiles::Tiles::default();
    let crafts = tiles.insert_pane(Pane::Crafts);
    let detail: Vec<_> = [Pane::Overview, Pane::Item, Pane::Recipe, Pane::Log].into_iter().map(|pane| tiles.insert_pane(pane)).collect();
    let details = tiles.insert_tab_tile(detail);
    let root = tiles.insert_horizontal_tile(vec![crafts, details]);
    egui_tiles::Tree::new(egui::Id::unique("ponder_tiles"), root, tiles)
}

pub struct Behavior<'a> {
    pub state: &'a mut State,
}

impl egui_tiles::Behavior<Pane> for Behavior<'_> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        pane.title().into()
    }
    fn pane_ui(&mut self, ui: &mut Ui, _: egui_tiles::TileId, pane: &mut Pane) -> egui_tiles::UiResponse {
        egui::Frame::new().inner_margin(6).show(ui, |ui| match pane {
            Pane::Crafts => crafts(ui, self.state),
            Pane::Overview => overview(ui, self.state),
            Pane::Item => item(ui, self.state),
            Pane::Recipe => recipe(ui, self.state),
            Pane::Log => log(ui, self.state),
        });
        egui_tiles::UiResponse::None
    }
    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() }
    }
    fn is_tab_closable(&self, _: &egui_tiles::Tiles<Pane>, _: egui_tiles::TileId) -> bool {
        true
    }
}

fn crafts(ui: &mut Ui, state: &mut State) {
    if state.data.is_none() {
        empty(ui, "Run the ladder to find crafting candidates.");
        return;
    }
    if state.rows.is_empty() {
        empty(ui, "No crafts match these filters.");
        return;
    }
    let mut clicked = None;
    let mut sort = None;
    let rows = &state.rows;
    let estimates = state.estimates();
    let divine = state.divine();
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .columns(Column::auto().at_least(36.0), Col::ALL.len())
        .header(24.0, |mut header| {
            for col in Col::ALL {
                header.col(|ui| {
                    if ui.selectable_label(state.saved.sort.col == col, col.label()).on_hover_text(col.tooltip()).clicked() {
                        sort = Some(col);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(24.0, rows.len(), |mut row| {
                let index = rows[row.index()];
                let estimate = &estimates[index];
                row.set_selected(state.selected == Some(index));
                for col in Col::ALL {
                    row.col(|ui| match col {
                        Col::Rank => {
                            ui.label(state.ranks.get(index).copied().unwrap_or_default().to_string());
                        }
                        Col::Item => {
                            ui.label(&estimate.title).on_hover_text(estimate.failure().unwrap_or(&estimate.base));
                        }
                        Col::Mods => {
                            ui.label(estimate.targets.len().to_string()).on_hover_text(estimate.targets.join("\n"));
                        }
                        Col::Demand => {
                            ui.label(estimate.demand.to_string());
                        }
                        Col::Margin => {
                            ui.label(signed(estimate.margin(), divine, ui.visuals().dark_mode));
                        }
                        _ => {
                            ui.label(money(col.value(estimate), divine));
                        }
                    });
                }
                if row.response().clicked() {
                    clicked = Some(index);
                }
            })
        });
    if let Some(col) = sort {
        state.saved.sort.click(col);
        state.refilter();
    }
    if let Some(index) = clicked {
        state.select(index);
    }
}

fn overview(ui: &mut Ui, state: &mut State) {
    let Some(estimate) = state.selected_estimate() else {
        empty(ui, "Select a craft.");
        return;
    };
    ui.heading(&estimate.title);
    ui.label(&estimate.base);
    if let Some(error) = estimate.failure() {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    egui::Grid::new("estimate_totals").show(ui, |ui| {
        for (label, value) in [
            ("Crafting", estimate.craft_cost()),
            ("Base", estimate.base_price.map(|price| price.0)),
            ("Total", estimate.total()),
            ("Market", estimate.market.map(|price| price.0)),
            ("Margin", estimate.margin()),
        ] {
            ui.label(label);
            ui.label(money(value, state.divine()));
            ui.end_row();
        }
    });
    if estimate.base_price.is_none() {
        ui.label(RichText::new("Base acquisition is unquoted; the total and margin are unknown.").weak());
    }
    if let Some(hours) = estimate.clicking_hours() {
        ui.label(format!("{} expected crafting time", view::hours(hours)));
    }
    ui.horizontal_wrapped(|ui| {
        for skill in &estimate.skills {
            chip(ui, skill);
        }
    });
    link_row(ui, "Finished item on trade", estimate.trade_url.as_deref());
    link_row(ui, "Starting base on trade", estimate.base_url.as_deref());
    for error in [&estimate.market_error, &estimate.base_error].into_iter().flatten() {
        ui.colored_label(ui.visuals().error_fg_color, error);
    }
    if ui.add_enabled(state.has_cookie && !state.jobs.busy(), egui::Button::new("Fetch trade prices")).clicked() {
        state.price_selected();
    }
}

fn item(ui: &mut Ui, state: &State) {
    let Some(estimate) = state.selected_estimate() else {
        empty(ui, "Select a craft.");
        return;
    };
    ui.heading(&estimate.base);
    ui.label(format!("Item level {}", estimate.item_level));
    for row in &estimate.rows {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&row.text).color(role_color(row.role, ui.visuals().dark_mode)));
            ui.label(RichText::new(&row.tier).weak());
        });
    }
}

fn recipe(ui: &mut Ui, state: &State) {
    let Some(plan) = state.selected_estimate().and_then(|estimate| estimate.summary.as_ref()) else {
        empty(ui, "No solved policy for the selected craft.");
        return;
    };
    ui.heading("Policy operations");
    ui.label("Expected uses per finished item, including retries. Some outcome weights are estimated.");
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("policy_operations").striped(true).show(ui, |ui| {
            ui.strong("Operation");
            ui.strong("Expected uses");
            ui.end_row();
            for (operation, uses) in &plan.usage {
                ui.label(operation);
                ui.monospace(format!("{uses:.3}"));
                ui.end_row();
            }
        });
        if !plan.unpriced.is_empty() {
            ui.label(format!("Unavailable prices: {}", plan.unpriced.join(", ")));
        }
    });
}

fn log(ui: &mut Ui, state: &State) {
    for (name, duration) in state.jobs.running() {
        ui.label(format!("{name}: {:.1}s", duration.as_secs_f64()));
    }
    egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
        for (time, message) in &state.log {
            ui.label(format!("{:.1}s {message}", time.duration_since(state.started).as_secs_f64()));
        }
    });
}
