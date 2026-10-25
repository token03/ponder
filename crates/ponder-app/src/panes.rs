//! The tiles: each pane draws one view of the run or of the selected estimate.

use egui::{Align, Color32, Layout, RichText, ScrollArea, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use egui_plot::{Legend, Plot, PlotPoints, Points};
use ponder::pipeline::{CraftRoute, Estimate, Finish};
use ponder_core::{MAGIC, NORMAL};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::state::State;
use crate::view::{self, Col};
use crate::widgets::{self, chip, empty, link_row, money, signed};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pane {
    Crafts,
    Overview,
    Item,
    Recipe,
    Fractures,
    Products,
    Recomb,
    Finish,
    Log,
}

impl Pane {
    pub const ALL: [Pane; 9] =
        [Pane::Crafts, Pane::Overview, Pane::Item, Pane::Recipe, Pane::Fractures, Pane::Products, Pane::Recomb, Pane::Finish, Pane::Log];

    pub fn title(self) -> &'static str {
        match self {
            Pane::Crafts => "Crafts",
            Pane::Overview => "Overview",
            Pane::Item => "Item",
            Pane::Recipe => "Recipe",
            Pane::Fractures => "Fractures",
            Pane::Products => "Pivots",
            Pane::Recomb => "Donors",
            Pane::Finish => "Finish",
            Pane::Log => "Log",
        }
    }
}

/// The default arrangement: the table and overview on the left, the selected item on the right.
pub fn default_tree() -> egui_tiles::Tree<Pane> {
    let mut tiles = egui_tiles::Tiles::default();
    let tabs = |tiles: &mut egui_tiles::Tiles<Pane>, panes: &[Pane]| {
        let ids = panes.iter().map(|p| tiles.insert_pane(*p)).collect();
        tiles.insert_tab_tile(ids)
    };
    let left = tabs(&mut tiles, &[Pane::Crafts, Pane::Overview, Pane::Finish, Pane::Log]);
    let top = tabs(&mut tiles, &[Pane::Item, Pane::Recipe]);
    let bottom = tabs(&mut tiles, &[Pane::Fractures, Pane::Products, Pane::Recomb]);
    let right = tiles.insert_vertical_tile(vec![top, bottom]);
    let root = tiles.insert_horizontal_tile(vec![left, right]);
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(l))) = tiles.get_mut(root) {
        l.shares.set_share(left, 1.4);
    }
    if let Some(egui_tiles::Tile::Container(egui_tiles::Container::Linear(l))) = tiles.get_mut(right) {
        l.shares.set_share(top, 1.6);
    }
    egui_tiles::Tree::new(egui::Id::unique("ponder_tiles"), root, tiles)
}

pub struct Behavior<'a> {
    pub state: &'a mut State,
}

impl egui_tiles::Behavior<Pane> for Behavior<'_> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        match pane {
            Pane::Crafts if !self.state.estimates().is_empty() => format!("Crafts ({})", self.state.rows.len()).into(),
            Pane::Fractures => match self.state.selected_estimate().map(|e| e.fractures.len()) {
                Some(n) if n > 0 => format!("Fractures ({n})").into(),
                _ => pane.title().into(),
            },
            Pane::Products => match self.state.selected_estimate().map(|e| e.pivots.len()) {
                Some(n) if n > 0 => format!("Pivots ({n})").into(),
                _ => pane.title().into(),
            },
            _ => pane.title().into(),
        }
    }

    fn pane_ui(&mut self, ui: &mut Ui, _tile: egui_tiles::TileId, pane: &mut Pane) -> egui_tiles::UiResponse {
        egui::Frame::new().inner_margin(6).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            match pane {
                Pane::Crafts => crafts(ui, self.state),
                Pane::Overview => overview(ui, self.state),
                Pane::Item => item(ui, self.state),
                Pane::Recipe => recipe(ui, self.state),
                Pane::Fractures => fractures(ui, self.state),
                Pane::Products => products(ui, self.state),
                Pane::Recomb => recomb(ui, self.state),
                Pane::Finish => finish(ui, self.state),
                Pane::Log => log(ui, self.state),
            }
        });
        egui_tiles::UiResponse::None
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() }
    }

    fn is_tab_closable(&self, _tiles: &egui_tiles::Tiles<Pane>, _tile: egui_tiles::TileId) -> bool {
        true
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 {
        26.0
    }

    fn gap_width(&self, _style: &egui::Style) -> f32 {
        4.0
    }
}

fn no_run(ui: &mut Ui, s: &State) -> bool {
    if s.data.is_some() {
        return false;
    }
    let msg = if s.db.is_none() {
        "loading game data…"
    } else if s.jobs.is_running("ladder") {
        "running the ladder pipeline…"
    } else if !s.has_ladder {
        "no ladder data yet: Run ➡ Fetch ladder characters, then Run ladder"
    } else {
        "press Run ladder (left) to cost what the ladder wears"
    };
    empty(ui, msg);
    true
}

fn no_selection(ui: &mut Ui, s: &State) -> Option<usize> {
    if no_run(ui, s) {
        return None;
    }
    if s.rows.is_empty() {
        empty(ui, "no crafts match; widen the selection or reset filters");
    } else if s.selected.is_none() {
        empty(ui, "select a craft in the table");
    }
    s.selected
}

// -- crafts table ----------------------------------------------------------------------------

fn crafts(ui: &mut Ui, s: &mut State) {
    if no_run(ui, s) {
        return;
    }
    if s.rows.is_empty() {
        empty(ui, "no crafts match; widen the selection or reset filters");
        return;
    }
    let dark = ui.visuals().dark_mode;
    let divine = s.divine();
    let mut clicked_col = None;
    let mut clicked_row = None;
    let rows = s.rows.clone();
    let reveal = s.reveal.then(|| s.selected.and_then(|sel| rows.iter().position(|&r| r == sel))).flatten();
    s.reveal = false;
    let text_h = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
    ScrollArea::horizontal().auto_shrink(false).show(ui, |ui| {
        let mut table = TableBuilder::new(ui)
            .striped(true)
            .resizable(true)
            .sense(Sense::click())
            .cell_layout(Layout::left_to_right(Align::Center))
            .column(Column::exact(34.0))
            .column(Column::initial(190.0).at_least(80.0).clip(true))
            .columns(Column::initial(66.0).at_least(40.0), 8)
            .column(Column::remainder().at_least(80.0).clip(true))
            .min_scrolled_height(0.0);
        if let Some(r) = reveal {
            table = table.scroll_to_row(r, Some(Align::Center));
        }
        let sort = s.saved.sort;
        let est = s.estimates();
        table
            .header(22.0, |mut header| {
                for col in Col::ALL {
                    header.col(|ui| {
                        let arrow = if sort.col == col { if sort.descending { " ⏷" } else { " ⏶" } } else { "" };
                        let r = ui.add(egui::Label::new(RichText::new(format!("{}{arrow}", col.label())).strong()).sense(Sense::click()));
                        if r.on_hover_text(col.tooltip()).clicked() {
                            clicked_col = Some(col);
                        }
                    });
                }
            })
            .body(|body| {
                body.rows(text_h, rows.len(), |mut row| {
                    let i = rows[row.index()];
                    let e = &est[i];
                    row.set_selected(s.selected == Some(i));
                    for col in Col::ALL {
                        row.col(|ui| match col {
                            Col::Rank => {
                                ui.label(RichText::new(s.ranks.get(i).copied().unwrap_or(0).to_string()).weak().monospace());
                            }
                            Col::Item => {
                                let t = if e.kind == "cluster" { RichText::new(&e.title).italics() } else { RichText::new(&e.title) };
                                let r = ui.label(t);
                                if let Some(error) = e.failure() {
                                    r.on_hover_text(error);
                                } else {
                                    r.on_hover_text(format!("{} · {} bases", e.base, e.bases));
                                }
                            }
                            Col::Margin => {
                                ui.label(signed(e.margin(), divine, dark));
                            }
                            Col::Hole => {
                                ui.label(RichText::new(e.hole().map_or("–".into(), |h| format!("{h:.1}"))).monospace());
                            }
                            Col::Demand => {
                                ui.label(RichText::new(e.demand.to_string()).monospace());
                            }
                            Col::Mods => {
                                ui.label(e.targets.join(" · "));
                            }
                            Col::Craft if e.total().is_none() && e.direct_lower_bound().is_some() => {
                                ui.label(RichText::new(format!("≥{}", view::money(e.direct_lower_bound(), divine))).monospace())
                                    .on_hover_text("Certified lower bound. A complete recipe is cheaper, so this search stopped.");
                            }
                            Col::Craft | Col::Fractured | Col::Recomb | Col::Cheapest | Col::Market => {
                                ui.label(money(col.value(e), divine));
                            }
                        });
                    }
                    if row.response().clicked() {
                        clicked_row = Some(i);
                    }
                });
            });
    });
    if let Some(col) = clicked_col {
        s.saved.sort.click(col);
        s.refilter();
    }
    if let Some(i) = clicked_row {
        s.select(i);
    }
}

/// A plot series: name, colour, and (estimate index, point) pairs.
type Series<'a> = (&'a str, Color32, Vec<(usize, [f64; 2])>);

// -- overview --------------------------------------------------------------------------------

fn overview(ui: &mut Ui, s: &mut State) {
    if no_run(ui, s) {
        return;
    }
    let d = s.data.as_ref().expect("checked");
    let r = &d.report;
    let divine = r.divine;
    ui.horizontal_wrapped(|ui| {
        for (k, v) in [
            ("league", r.league.clone()),
            ("characters", r.characters.to_string()),
            ("rare items", r.items.to_string()),
            ("clusters", r.clusters.to_string()),
            ("archetypes", r.archetypes.to_string()),
            ("cluster combos", r.cluster_archetypes.to_string()),
            ("estimates", r.estimates.len().to_string()),
            ("divine", format!("{divine:.0}c")),
            ("prices", r.prices_source.clone()),
            ("run took", format!("{:.1}s", d.took.as_secs_f64())),
        ] {
            ui.group(|ui| {
                ui.vertical(|ui| {
                    // never wrapped: a card that doesn't fit moves to the next line
                    ui.add(egui::Label::new(RichText::new(k).small().weak()).extend());
                    ui.add(egui::Label::new(RichText::new(v).strong()).extend());
                });
            });
        }
    });
    ui.add_space(6.0);
    ui.label(
        RichText::new("Cheapest way to make each craft against how many ladder characters wear it. Click a point to select it.").weak(),
    );
    let shown: std::collections::HashSet<usize> = s.rows.iter().copied().collect();
    let point = |e: &Estimate| Some([e.demand as f64, e.cheapest()?.max(1.0).log10()]);
    let mut series: Vec<Series> =
        vec![("gear", Color32::from_rgb(90, 160, 240), vec![]), ("clusters", Color32::from_rgb(230, 160, 60), vec![])];
    for (i, e) in r.estimates.iter().enumerate() {
        if let (true, Some(p)) = (shown.contains(&i), point(e)) {
            series[usize::from(e.kind == "cluster")].2.push((i, p));
        }
    }
    let selected = s.selected.and_then(|i| point(&r.estimates[i]));
    let labels: std::collections::HashMap<String, Vec<String>> = series
        .iter()
        .map(|(name, _, pts)| {
            let l = pts
                .iter()
                .map(|(i, _)| {
                    let e = &r.estimates[*i];
                    format!("{}\n{}\n{} · {} characters", e.title, e.targets.join(" · "), view::money(e.cheapest(), divine), e.demand)
                })
                .collect();
            (name.to_string(), l)
        })
        .collect();
    let fmt_cost = move |y: f64| view::money(Some(10f64.powf(y)), divine);
    let resp = Plot::new("cost_vs_demand")
        .legend(Legend::default())
        .x_axis_label("characters wearing it")
        .y_axis_label("cheapest (log)")
        .y_axis_formatter(move |mark, _| fmt_cost(mark.value))
        .label_formatter(move |hover| match hover {
            egui_plot::HoverPosition::NearDataPoint { plot_name, index, .. } => labels.get(*plot_name).and_then(|l| l.get(*index)).cloned(),
            egui_plot::HoverPosition::Elsewhere { .. } => None,
        })
        .show(ui, |plot| {
            for (name, color, pts) in &series {
                plot.points(
                    Points::new(*name, PlotPoints::from(pts.iter().map(|(_, p)| *p).collect::<Vec<_>>())).color(*color).radius(3.0),
                );
            }
            if let Some(p) = selected {
                plot.points(Points::new("selected", PlotPoints::from(vec![p])).color(Color32::WHITE).radius(6.0).filled(false));
            }
            plot.pointer_coordinate()
        });
    if resp.response.clicked()
        && let Some(at) = resp.inner
    {
        let bounds = resp.transform.bounds();
        let (sx, sy) = (bounds.width().max(1e-9), bounds.height().max(1e-9));
        let nearest = series
            .iter()
            .flat_map(|(_, _, pts)| pts.iter())
            .min_by(|a, b| {
                let d = |p: &[f64; 2]| ((p[0] - at.x) / sx).powi(2) + ((p[1] - at.y) / sy).powi(2);
                d(&a.1).total_cmp(&d(&b.1))
            })
            .map(|(i, _)| *i);
        if let Some(i) = nearest {
            s.select(i);
            s.reveal = true;
        }
    }
}

// -- the selected item -----------------------------------------------------------------------

fn item(ui: &mut Ui, s: &mut State) {
    let Some(i) = no_selection(ui, s) else { return };
    let dark = ui.visuals().dark_mode;
    let divine = s.divine();
    let e = &s.estimates()[i];
    let mut action = None;
    let mut resale = s.resale_draft;
    let mut quote_action = None;
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(&e.title);
            ui.label(RichText::new(if e.kind == "cluster" { "cluster jewel" } else { "gear" }).weak());
        });
        ui.label(format!(
            "{} · {} base{} · ilvl {}{}",
            e.base,
            e.bases,
            if e.bases == 1 { "" } else { "s" },
            e.item_level,
            if e.used_base.is_empty() || e.used_base == e.base { String::new() } else { format!(" · worn on {}", e.used_base) }
        ));
        if !e.skills.is_empty() {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(format!("{} wearers:", e.demand)).weak());
                for k in e.skills.iter().take(12) {
                    chip(ui, k);
                }
            });
        }
        if let Some(err) = e.failure() {
            ui.colored_label(widgets::bad(dark), err);
        }
        ui.add_space(4.0);
        action = actions(ui, s, e);
        ui.separator();

        // mod lines are long: one section above the other
        mods(ui, e, dark);
        ui.add_space(6.0);
        costs(ui, e, divine, dark);
        ui.separator();
        currency(ui, e, divine);
        if e.protected.is_some() || e.awakener.is_some() {
            ui.separator();
            quote_action = resale_controls(ui, e, &mut resale, s.jobs.is_running(&format!("recovery quote #{i}")), divine);
        }
    });
    s.resale_draft = resale;
    match action {
        Some(ItemAction::Price) => s.price_selected(),
        Some(ItemAction::Partials) => s.starts_selected(),
        None => {}
    }
    if let Some(enabled) = quote_action {
        s.quote_resale(enabled);
    }
}

fn resale_controls(ui: &mut Ui, e: &Estimate, quote: &mut ponder::pipeline::ResaleQuote, busy: bool, divine: f64) -> Option<bool> {
    let mut apply = None;
    egui::CollapsingHeader::new("Sell blocked rolls").show(ui, |ui| {
        ui.set_max_width((ui.clip_rect().right() - ui.cursor().left()).max(0.0));
        ui.add(
            egui::Label::new(
                "Quote full items that retain the valuable crafting core. Annul risk and replacement costs are compared automatically.",
            )
            .wrap(),
        );
        for (label, value, suffix) in [
            ("Sale proceeds", &mut quote.proceeds, " c"),
            ("Replacement bases", &mut quote.replacement_base, " c"),
            ("Trade time", &mut quote.trade_seconds, " s"),
        ] {
            ui.horizontal(|ui| {
                ui.label(label);
                ui.add(egui::DragValue::new(value).range(0.0..=f64::MAX).suffix(suffix));
            });
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!busy, egui::Button::new("Apply quote")).clicked() {
                apply = Some(true);
            }
            if ui.add_enabled(!busy, egui::Button::new("Clear quote")).clicked() {
                apply = Some(false);
            }
        });
        let (thresholds, error) = if let Some(r) = e.protected.as_ref().filter(|_| e.cheapest_route() != Some(CraftRoute::Awakener)) {
            (&r.sale_thresholds, &r.sale_error)
        } else {
            let r = e.awakener.as_ref().expect("recovery route");
            (&r.sale_thresholds, &r.sale_error)
        };
        if let Some(core) = e.recovery_core()
            && let Some(p) = &e.problem
        {
            let names = p
                .targets
                .iter()
                .enumerate()
                .filter(|(i, _)| core.targets & (1 << i) != 0)
                .map(|(_, t)| t.name.as_str())
                .collect::<Vec<_>>();
            ui.add(egui::Label::new(format!("Quote applies while retaining: {}", names.join(" + "))).wrap());
        }
        if let Some(error) = error {
            ui.colored_label(widgets::bad(ui.visuals().dark_mode), error);
        }
        ui.add(
            egui::Label::new("Break-even sale prices include replacement bases and trade time, compared with continuing without sales.")
                .wrap(),
        );
        let replacement = quote.replacement_base + quote.trade_seconds * e.settings.chaos_per_hour / 3600.0;
        for t in thresholds {
            ui.add(
                egui::Label::new(format!("Break-even {} · {}", view::money(Some(t.premium + replacement), divine), t.description)).wrap(),
            );
        }
    });
    apply
}

enum ItemAction {
    Price,
    Partials,
}

fn actions(ui: &mut Ui, s: &State, e: &Estimate) -> Option<ItemAction> {
    let i = s.selected.unwrap_or(0);
    let mut action = None;
    ui.horizontal_wrapped(|ui| {
        let busy = |s: &State, name: &str| s.jobs.is_running(&format!("{name} #{i}"));
        let trade_hint = if s.has_cookie { "" } else { " (needs POESESSID in .env)" };
        if ui
            .add_enabled(s.has_cookie && !busy(s, "trade prices"), egui::Button::new("Price"))
            .on_hover_text(format!("Price the item and bases{trade_hint}"))
            .on_disabled_hover_text(format!("Price the item and bases{trade_hint}"))
            .clicked()
        {
            action = Some(ItemAction::Price);
        }
        if ui
            .add_enabled(s.has_cookie && e.kind == "gear" && !busy(s, "half-done items"), egui::Button::new("Find partials"))
            .on_hover_text(format!("Find listed items with useful mods and price the finish{trade_hint}"))
            .clicked()
        {
            action = Some(ItemAction::Partials);
        }
        if ui.button("Copy JSON").clicked() {
            ui.ctx().copy_text(serde_json::to_string_pretty(&e.to_dict()).unwrap_or_default());
        }
    });
    link_row(ui, "Trade", e.trade_url.as_deref());
    link_row(ui, "Base", e.base_url.as_deref());
    action
}

fn costs(ui: &mut Ui, e: &Estimate, divine: f64, dark: bool) {
    let route = e.cheapest_route();
    let fracture = match route {
        Some(CraftRoute::Fracture(i)) => Some(&e.fractures[i]),
        _ => None,
    };
    ui.label(RichText::new("Cost").strong());
    egui::Grid::new("costs").num_columns(3).spacing([10.0, 3.0]).show(ui, |ui| {
        let line = |ui: &mut Ui, k: &str, v: RichText, note: String| {
            ui.label(RichText::new(k).weak());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| ui.label(v));
            if !note.is_empty() {
                ui.add(egui::Label::new(RichText::new(&note).weak().small()).truncate()).on_hover_text(note);
            }
            ui.end_row();
        };
        let craft_cost = match route {
            Some(CraftRoute::Fracture(i)) => e.fractures[i].plan.as_ref().map(|p| p.cost),
            Some(CraftRoute::Recomb) => e.recomb.as_ref().and_then(|r| r.cost),
            Some(CraftRoute::Awakener) => e.awakener.as_ref().map(|r| r.cost),
            Some(CraftRoute::Protected) => e.protected.as_ref().map(|r| r.cost),
            _ => e.craft_cost(),
        };
        line(
            ui,
            match route {
                Some(CraftRoute::Recomb) => "recomb",
                Some(CraftRoute::Awakener) => "Awakener",
                Some(CraftRoute::Protected) => "protected craft",
                _ => "craft",
            },
            money(craft_cost, divine),
            String::new(),
        );
        if !e.bench.is_empty() {
            line(ui, "bench", money(Some(e.bench_cost()), divine), e.bench.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(", "));
        }
        if let Some(f) = fracture {
            let source = if f.self_made() { "make" } else { "buy" };
            line(ui, "base", money(f.base_cost(), divine), format!("{source}: {}", f.target));
        } else if route != Some(CraftRoute::Recomb) && (e.base_price.is_some() || !e.fixed.is_empty()) {
            let note = if e.fixed.is_empty() { e.base_source.clone() } else { format!("fractured: {}", e.fixed.join(" + ")) };
            line(ui, "base", money(e.base_price.map(|b| b.0), divine), note);
        }
        line(ui, "total", money(e.cheapest(), divine).strong(), String::new());
        if route != Some(CraftRoute::Craft) && e.total().is_some() {
            line(ui, "scratch", money(e.total(), divine), String::new());
        } else if let Some(lower) = e.direct_lower_bound() {
            line(
                ui,
                "scratch",
                RichText::new(format!("≥{}", view::money(Some(lower), divine))).monospace(),
                "A complete recipe is cheaper".into(),
            );
        }
        if fracture.is_none()
            && let Some(f) = e.best_fracture()
        {
            line(ui, "fractured", money(f.total(), divine), f.target.clone());
        }
        if e.recomb.is_some() && route != Some(CraftRoute::Recomb) {
            line(ui, "recomb", money(e.recomb_total(), divine), String::new());
        }
        if e.awakener.is_some() && route != Some(CraftRoute::Awakener) {
            line(ui, "Awakener", money(e.awakener_total(), divine), String::new());
        }
        let listed = e.market.map(|m| format!("{} listed", m.1)).or_else(|| e.market_error.clone()).unwrap_or_default();
        line(ui, "market", money(e.market.map(|m| m.0), divine), listed);
        line(ui, "margin", signed(e.margin(), divine, dark), String::new());
        if let Some(h) = e.hole() {
            line(ui, "hole", RichText::new(format!("{h:.1}")).monospace(), "demand per listing, by margin".into());
        }
    });
}

fn mods(ui: &mut Ui, e: &Estimate, dark: bool) {
    ui.label(RichText::new("Mods").strong());
    egui::Grid::new("mods").num_columns(3).spacing([8.0, 3.0]).show(ui, |ui| {
        for r in &e.rows {
            let c = widgets::role_color(r.role, dark);
            ui.label(RichText::new(r.role).color(c).small());
            ui.label(RichText::new(format!("{:?}", r.side).to_lowercase()).weak().small());
            ui.label(RichText::new(format!("{}  {}", r.text, r.tier)).color(c));
            ui.end_row();
        }
    });
    if !e.skipped.is_empty() {
        ui.label(RichText::new(format!("not crafted: {}", e.skipped.join(", "))).weak());
    }
}

fn currency(ui: &mut Ui, e: &Estimate, divine: f64) {
    let Some(currency) = e.currency_cost() else {
        return;
    };
    let hours = e.clicking_hours().unwrap_or(0.0);
    let usage = e.usage();
    ui.label(RichText::new("Currency").strong());
    let most = usage.iter().map(|u| u.1).fold(0.0, f64::max).max(1e-9);
    egui::Grid::new("usage").num_columns(3).spacing([10.0, 2.0]).show(ui, |ui| {
        for (action, n) in usage.iter().filter(|u| u.1 >= 0.05) {
            ui.label(action);
            ui.label(RichText::new(if *n >= 10.0 { format!("{n:.0}") } else { format!("{n:.1}") }).monospace());
            ui.add(egui::ProgressBar::new((*n / most) as f32).desired_width(160.0).desired_height(8.0));
            ui.end_row();
        }
    });
    ui.label(
        RichText::new(format!(
            "{} net spend · {} · {} time",
            view::money(Some(currency), divine),
            view::hours(hours),
            view::money(Some(hours * e.settings.chaos_per_hour), divine),
        ))
        .weak(),
    )
    .on_hover_text(format!("Time charged at {:.0}c/h", e.settings.chaos_per_hour));
    if let Some((_, plan)) = e.cheapest_plan()
        && plan.bounds.spread(plan.cost) >= 0.005
    {
        ui.label(
            RichText::new(format!(
                "certified {} – {}",
                view::money(Some(plan.bounds.lower), divine),
                view::money(Some(plan.bounds.upper), divine)
            ))
            .weak(),
        )
        .on_hover_text(
            "The solver counts other mods without knowing which they are, so its chances are estimates.\n\
             No way of crafting, even reading every mod, expects to spend less than the first figure;\n\
             this plan expects to spend at most the second. Both hold for the solver's game model.",
        );
    }
}

// -- recipe ----------------------------------------------------------------------------------

fn rarity(r: u8) -> &'static str {
    match r {
        NORMAL => "normal",
        MAGIC => "magic",
        _ => "rare",
    }
}

fn recipe(ui: &mut Ui, s: &mut State) {
    let Some(i) = no_selection(ui, s) else { return };
    let dark = ui.visuals().dark_mode;
    let e = &s.estimates()[i];
    if e.cheapest_route() == Some(CraftRoute::Protected)
        && let Some(r) = &e.protected
    {
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.strong(format!("Prepare {}es · {:.2} expected runs", r.side.name(), 1.0 + r.finishing.restarts));
            compact_recipe(ui, &ponder_core::steps(&r.preparation, &r.plan, 0.02));
            ui.strong(&r.cleanup.name);
            ui.strong("Finish the protected item");
            compact_recipe(ui, &e.steps());
            recovery_summary(ui, &r.finishing, s.divine());
        });
        return;
    }
    if e.cheapest_route() == Some(CraftRoute::Recomb) {
        recomb(ui, s);
        return;
    }
    if let Some(r) = &e.awakener
        && e.cheapest_route() == Some(CraftRoute::Awakener)
    {
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| awakener_recipe(ui, r, s.divine()));
        return;
    }
    let Some((problem, plan)) = e.cheapest_plan() else {
        empty(ui, e.error.as_deref().unwrap_or("No priced plan"));
        return;
    };
    let steps = ponder_core::steps(problem, plan, 0.02);
    if let Some(CraftRoute::Fracture(i)) = e.cheapest_route() {
        let f = &e.fractures[i];
        let source = if f.self_made() { "Make" } else { "Buy" };
        ui.label(format!("{source} fractured {} · {}", f.target, view::money(f.base_cost(), s.divine())));
    }
    if steps.is_empty() {
        empty(ui, e.error.as_deref().unwrap_or("no plan"));
        return;
    }
    let jump = s.jump;
    let (pivots, divine) = (e.pivots.clone(), s.divine());
    let (mut go, mut pick) = (None, None);
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for st in &steps {
            let frame = egui::Frame::group(ui.style()).fill(if s.highlighted_step == Some(st.number) {
                ui.visuals().selection.bg_fill.gamma_multiply(0.3)
            } else {
                Color32::TRANSPARENT
            });
            let r = frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(format!("{}.", st.number)).strong().monospace());
                    ui.label(RichText::new(rarity(st.rarity)).italics());
                    if st.have.is_empty() {
                        ui.label(RichText::new("nothing yet").weak());
                    }
                    for h in &st.have {
                        ui.label(RichText::new(h).color(widgets::role_color("target", dark)));
                    }
                    ui.label(RichText::new(format!("×{:.2} visits · {:.1} actions/visit", st.reached, st.per_reach())).weak().small());
                });
                for (k, (action, when, per)) in st.actions.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(if k == 0 { "➡" } else { "  or" });
                        ui.label(RichText::new(action).strong());
                        if !when.is_empty() {
                            ui.label(RichText::new(format!("when {when}")).weak());
                        }
                        ui.label(RichText::new(format!("×{per:.1}")).weak().monospace());
                    });
                }
                for p in pivots.iter().filter(|p| p.step == st.number) {
                    if pivot_row(ui, p, divine) {
                        pick = Some(p.to);
                    }
                }
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("then").weak());
                    for x in &st.exits {
                        let color = match x.kind.as_str() {
                            "done" | "win" => widgets::good(dark),
                            "back" => widgets::bad(dark),
                            _ => ui.visuals().text_color(),
                        };
                        let text = RichText::new(format!("{} {}", view::percent(x.chance), x.what)).color(color);
                        match x.to {
                            Some(to) => {
                                if ui.link(text.clone()).on_hover_text(format!("go to step {to}")).clicked() {
                                    go = Some(to);
                                }
                                ui.label(RichText::new(format!("➡{to}")).weak().small());
                            }
                            None => {
                                ui.label(text);
                            }
                        }
                    }
                });
            });
            if jump == Some(st.number) {
                r.response.scroll_to_me(Some(Align::TOP));
            }
        }
    });
    s.jump = go;
    if go.is_some() {
        s.highlighted_step = go;
    }
    if let Some(j) = pick {
        s.select(j);
        s.reveal = true;
    }
}

// -- fractured bases -------------------------------------------------------------------------

fn fractures(ui: &mut Ui, s: &mut State) {
    let Some(i) = no_selection(ui, s) else { return };
    let dark = ui.visuals().dark_mode;
    let divine = s.divine();
    donor_range(ui, &mut s.saved.donors.fracture);
    let range = s.saved.donors.fracture;
    ui.label(
        RichText::new(format!(
            "Regal magic donors, then fill to 4 modifiers before fracturing; up to {} added modifiers.",
            4usize.saturating_sub(range[0])
        ))
        .weak()
        .small(),
    );
    let league = s.data.as_ref().map_or("Standard", |d| d.report.league.as_str());
    let e = &s.estimates()[i];
    if e.fractures.is_empty() {
        empty(ui, "No fracture alternatives");
        return;
    }
    for f in &e.fractures {
        if let Some(query) = &f.donor_query {
            let query = ponder::pipeline::donor_mod_range(query, range[0], range[1]);
            ui.hyperlink_to(
                format!("Buy donor: {}", ponder::pipeline::short_name(&f.target, 42)),
                ponder_net::trade::search_url(league, &query),
            );
        }
    }
    let scratch = e.total();
    TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(220.0).clip(true))
        .columns(Column::initial(76.0), 5)
        .column(Column::remainder())
        .header(20.0, |mut h| {
            for t in ["fractured mod", "craft rest", "buy base", "make base", "total", "vs scratch", ""] {
                h.col(|ui| {
                    ui.strong(t);
                });
            }
        })
        .body(|mut body| {
            for f in &e.fractures {
                body.row(20.0, |mut row| {
                    row.col(|ui| {
                        let r = ui.label(&f.target);
                        if let Some(err) = f.error.as_deref().or(f.base_error.as_deref()) {
                            r.on_hover_text(err);
                        }
                    });
                    row.col(|ui| {
                        if let Some(lower) = f.craft_lower_bound.filter(|_| f.plan.is_none()) {
                            ui.label(RichText::new(format!("≥{}", view::money(Some(lower + f.bench_cost), divine))).monospace())
                                .on_hover_text("Certified lower bound. A complete recipe is cheaper, so this search stopped.");
                        } else {
                            ui.label(money(f.craft_cost(), divine));
                        }
                    });
                    row.col(|ui| {
                        let r = ui.label(money(f.base_price.map(|b| b.0), divine));
                        if let Some((_, n)) = f.base_price {
                            r.on_hover_text(format!("{n} listed"));
                        }
                    });
                    row.col(|ui| {
                        let r = ui.label(money(f.made.as_ref().map(|m| m.cost), divine));
                        if let Some(m) = &f.made {
                            r.on_hover_text(format!(
                                "spam {} until the mod lands with ≤{} mods ({} a try), then fracture ({} it picks it); ~{}",
                                m.action,
                                m.max_mods,
                                view::percent(m.hit),
                                view::percent(m.fracture_chance),
                                view::hours(m.hours)
                            ));
                        }
                    });
                    row.col(|ui| {
                        ui.label(money(f.total(), divine).strong());
                    });
                    row.col(|ui| {
                        let saving = match (scratch, f.total()) {
                            (Some(a), Some(b)) => Some(a - b),
                            _ => None,
                        };
                        ui.label(signed(saving, divine, dark));
                    });
                    row.col(|ui| {
                        if let Some(url) = &f.trade_url {
                            ui.hyperlink_to("trade", url);
                        }
                    });
                });
            }
        });
}

// -- product tree and half-done items --------------------------------------------------------

fn products(ui: &mut Ui, s: &mut State) {
    let Some(i) = no_selection(ui, s) else { return };
    let divine = s.divine();
    let e = s.estimates()[i].clone();
    let mut go = None;
    let mut step = None;
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.label(RichText::new("Pivots").strong())
            .on_hover_text("clicks along this recipe that land a good start on another product of the same base: finish that one instead");
        if e.pivots.is_empty() {
            ui.label(RichText::new("none: no click along this recipe gives another product a head start").weak());
        }
        let mut last = None;
        for p in &e.pivots {
            if last != Some(p.step) {
                ui.add_space(4.0);
                if ui.link(RichText::new(format!("at step {}", p.step)).strong()).on_hover_text("show the step in the recipe").clicked() {
                    step = Some(p.step);
                }
                last = Some(p.step);
            }
            if pivot_row(ui, p, divine) {
                go = Some(p.to);
            }
        }
        ui.separator();
        ui.label(RichText::new("Partials").strong());
        if e.starts.is_empty() {
            ui.label(RichText::new("Use Find partials to search trade").weak());
        } else {
            link_row(ui, "the search", e.starts_url.as_deref());
            finishes_table(ui, "starts", &e.starts, divine, ui.visuals().dark_mode);
        }
    });
    if let Some(n) = step {
        s.jump = Some(n);
        s.highlighted_step = Some(n);
    }
    if let Some(j) = go {
        s.select(j);
        s.reveal = true;
    }
}

/// One pivot: how often, to what, and what finishing it costs from there. True when its product was clicked.
fn pivot_row(ui: &mut Ui, p: &ponder::pipeline::Pivot, divine: f64) -> bool {
    let mut clicked = false;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("↪").weak());
        let name = p.product.targets.iter().map(|t| ponder::pipeline::short_name(t, 22)).collect::<Vec<_>>().join(" + ");
        clicked = ui.link(RichText::new(name).strong()).on_hover_text(format!("{} · go to this product", p.product.title)).clicked();
        let finish = format!("finish {}", view::money(Some(p.to_finish), divine));
        ui.label(format!("{finish} (fresh {})", view::money(Some(p.fresh), divine)));
        ui.label(RichText::new(format!("{} of clicks · ~{:.2} per item", view::percent(p.per_click), p.per_craft)).weak());
    })
    .response
    .on_hover_text(format!("e.g. {}", p.have));
    clicked
}

fn finishes_table(ui: &mut Ui, id: &str, found: &[Finish], divine: f64, dark: bool) {
    for (k, f) in found.iter().enumerate() {
        let d = f.to_dict();
        let have = d["have"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" + ")).unwrap_or_default();
        let head = format!("{} ➡ {}", if have.is_empty() { "nothing useful" } else { &have }, f.product);
        egui::CollapsingHeader::new(RichText::new(head).strong()).id_salt((id, k)).show(ui, |ui| {
            egui::Grid::new((id, k, "grid")).num_columns(2).show(ui, |ui| {
                ui.label(RichText::new("targets").weak());
                ui.label(f.targets.join(" · "));
                ui.end_row();
                if let Some(state) = d["state"].as_str() {
                    ui.label(RichText::new("now").weak());
                    ui.label(state);
                    ui.end_row();
                }
                ui.label(RichText::new("price").weak());
                ui.label(money(f.price, divine));
                ui.end_row();
                ui.label(RichText::new("to finish").weak());
                ui.label(money(f.craft_cost, divine));
                ui.end_row();
                ui.label(RichText::new("total").weak());
                ui.label(money(f.total(), divine).strong());
                ui.end_row();
                ui.label(RichText::new("saves").weak());
                ui.label(signed(f.saves(), divine, dark));
                ui.end_row();
                if let Some(w) = d["whisper"].as_str() {
                    ui.label(RichText::new("seller").weak());
                    ui.horizontal(|ui| {
                        ui.label(d["seller"].as_str().unwrap_or("?"));
                        if ui.small_button("copy whisper").clicked() {
                            ui.ctx().copy_text(w.to_string());
                        }
                    });
                    ui.end_row();
                }
            });
            if let Some(err) = &f.error {
                ui.colored_label(widgets::bad(dark), err);
            }
            for st in f.steps() {
                ui.label(format!(
                    "{}. {} {} ➡ {}",
                    st.number,
                    rarity(st.rarity),
                    if st.have.is_empty() { "nothing yet".into() } else { st.have.join(" + ") },
                    st.actions.first().map_or("", |a| a.0.as_str())
                ));
            }
        });
    }
}

// -- recombinator ----------------------------------------------------------------------------

fn recomb(ui: &mut Ui, s: &mut State) {
    let Some(i) = no_selection(ui, s) else { return };
    let divine = s.divine();
    donor_range(ui, &mut s.saved.donors.recomb);
    let range = s.saved.donors.recomb;
    let league = s.data.as_ref().map_or("Standard", |d| d.report.league.as_str());
    let e = &s.estimates()[i];
    if let Some(recipe) = &e.awakener {
        ScrollArea::vertical().auto_shrink(false).show(ui, |ui| awakener_recipe(ui, recipe, divine));
        return;
    }
    let Some(r) = &e.recomb else {
        empty(ui, "No donor route applies to this craft");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("cheapest tree").strong());
        ui.label(money(r.cost, divine).strong());
        ui.label(RichText::new(format!("+ bench = {}", view::money(e.recomb_total(), divine))).weak());
    });
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        if let Some(donors) = r.json["donors"].as_array().filter(|d| !d.is_empty()) {
            ui.strong("Prepare donors");
            for (i, donor) in donors.iter().enumerate() {
                let targets = donor["targets"].as_array().into_iter().flatten().filter_map(Value::as_str).collect::<Vec<_>>().join(" + ");
                ui.label(format!(
                    "{}: {targets} · {:.2} expected copies",
                    donor["base"].as_str().unwrap_or("donor"),
                    donor["copies"].as_f64().unwrap_or(1.0)
                ));
                if donor["query"].is_object() {
                    let required = donor["required_mods"].as_u64().unwrap_or(1) as usize;
                    let query = ponder::pipeline::donor_mod_range(&donor["query"], range[0].max(required), range[1].max(required));
                    ui.hyperlink_to("Find donor start", ponder_net::trade::search_url(league, &query));
                }
                if let Some((donor, _)) = r.donors.get(i)
                    && let Some((p, plan)) = donor.problem.as_ref().zip(donor.plan.as_ref())
                {
                    compact_recipe(ui, &ponder_core::steps(p, plan, 0.02));
                }
            }
            ui.separator();
        }
        let mut tree = r.json.clone();
        if let Some(object) = tree.as_object_mut() {
            object.remove("donors");
        }
        json_tree(ui, "recomb", &tree, divine);
        if let Some((p, plan)) = e.problem.as_ref().zip(r.finishing.as_ref()) {
            ui.strong("Finish recombined item");
            compact_recipe(ui, &ponder_core::steps(p, plan, 0.02));
            if let Some(recovery) = &r.recovery
                && plan.restarts > 0.0
            {
                ui.strong("Rebuild the crafting core after failed finishing");
                compact_recipe(ui, &ponder_core::steps(&recovery.preparation, &recovery.plan, 0.02));
                ui.label(&recovery.cleanup.name);
                recovery_summary(ui, plan, divine);
            }
        }
    });
}

fn awakener_recipe(ui: &mut Ui, r: &ponder::awakener::Recipe, divine: f64) {
    ui.label(RichText::new("Awakener’s Orb").strong());
    ui.label(format!("{:.2} expected donor pairs and transfers, including retries", 1.0 + r.restarts));
    for donor in &r.donors {
        ui.label(format!("Prepare {} {} with {}", donor.influence, donor.base, donor.target));
        ui.label(view::money(Some(donor.cost), divine));
        if let Some(url) = &donor.trade_url {
            ui.hyperlink_to("Find donor", url);
        }
        compact_recipe(ui, &donor.steps);
    }
    ui.label(format!("Transfer both modifiers, then reforge the remaining affixes · {}", view::money(Some(r.orb_cost), divine)));
    ui.label(format!("Expected cost to finish after reforging: {}", view::money(Some(r.finish), divine)));
    compact_recipe(ui, &r.finish_steps);
    if let Some(plan) = &r.finishing {
        recovery_summary(ui, plan, divine);
    }
    if !r.restart_conditions.is_empty() {
        egui::CollapsingHeader::new("Repeat donor preparation and transfer for these outcomes").show(ui, |ui| {
            for condition in &r.restart_conditions {
                ui.label(condition);
            }
        });
    }
}

fn recovery_summary(ui: &mut Ui, plan: &ponder_core::Plan, divine: f64) {
    if plan.restarts > 0.0 {
        ui.label(format!(
            "Expected recovery: {:.2} rebuilds, {:.2} sales · proceeds {}",
            plan.restarts - plan.sales,
            plan.sales,
            view::money(Some(plan.sale_proceeds), divine)
        ));
        ui.label("If an annul loses the crafting core, repeat preparation. Recovery costs are included.");
    }
}

fn compact_recipe(ui: &mut Ui, steps: &[ponder_core::Step]) {
    for step in steps {
        for (action, when, per_visit) in &step.actions {
            ui.label(format!(
                "{action} ×{:.2}{}",
                per_visit * step.reached,
                if when.is_empty() { String::new() } else { format!(" · {when}") }
            ));
        }
        for exit in step.exits.iter().filter(|exit| exit.to.is_none() && exit.kind == "back") {
            ui.label(RichText::new(format!("{}: {}", view::percent(exit.chance), exit.what)).weak());
        }
    }
}

fn donor_range(ui: &mut Ui, range: &mut [usize; 2]) {
    ui.horizontal(|ui| {
        ui.label("Donor modifiers").on_hover_text("A donor carrying multiple required modifiers needs at least that many slots.");
        ui.add(egui::DragValue::new(&mut range[0]).range(1..=6).prefix("min "));
        range[1] = range[1].max(range[0]);
        let min = range[0];
        ui.add(egui::DragValue::new(&mut range[1]).range(min..=6).prefix("max "));
    });
}

/// A JSON value as collapsible sections; numbers under cost-like keys as money.
fn json_tree(ui: &mut Ui, id: &str, v: &Value, divine: f64) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match x {
                    Value::Object(_) | Value::Array(_) if !is_flat(x) => {
                        egui::CollapsingHeader::new(k)
                            .id_salt((id, k))
                            .default_open(true)
                            .show(ui, |ui| json_tree(ui, &format!("{id}.{k}"), x, divine));
                    }
                    _ => {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(k).weak());
                            ui.label(scalar(k, x, divine));
                        });
                    }
                }
            }
        }
        Value::Array(a) => {
            for (n, x) in a.iter().enumerate() {
                egui::CollapsingHeader::new(format!("{}", n + 1))
                    .id_salt((id, n))
                    .default_open(true)
                    .show(ui, |ui| json_tree(ui, &format!("{id}.{n}"), x, divine));
            }
        }
        x => {
            ui.label(scalar("", x, divine));
        }
    }
}

fn is_flat(v: &Value) -> bool {
    match v {
        Value::Array(a) => a.iter().all(|x| !x.is_object() && !x.is_array()),
        Value::Object(_) => false,
        _ => true,
    }
}

fn scalar(key: &str, v: &Value, divine: f64) -> RichText {
    match v {
        Value::Number(n) if ["cost", "total", "back", "price"].iter().any(|k| key.contains(k)) => money(n.as_f64(), divine),
        Value::Number(n) if key.contains("chance") => RichText::new(view::percent(n.as_f64().unwrap_or(0.0))),
        Value::Array(a) => {
            RichText::new(a.iter().map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_string)).collect::<Vec<_>>().join(", "))
        }
        Value::String(s) => RichText::new(s),
        Value::Null => RichText::new("–").weak(),
        x => RichText::new(x.to_string()).monospace(),
    }
}

// -- finishing a real item -------------------------------------------------------------------

fn finish(ui: &mut Ui, s: &mut State) {
    let divine = s.divine();
    let dark = ui.visuals().dark_mode;
    ui.label(RichText::new("Paste an item copied in game or PoB.").weak());
    ui.add(
        egui::TextEdit::multiline(&mut s.finish_text)
            .font(egui::TextStyle::Monospace)
            .desired_rows(8)
            .desired_width(f32::INFINITY)
            .hint_text("Item Class: Rings\nRarity: Rare\n…"),
    );
    ui.horizontal(|ui| {
        let ready = s.db.is_some() && s.data.is_some() && !s.finish_text.trim().is_empty() && !s.jobs.is_running("finish item");
        if ui.add_enabled(ready, egui::Button::new("Find finishes")).clicked() {
            s.finish_item();
        }
        if ui.button("Paste").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::RequestPaste);
        }
        if ui.button("Clear").clicked() {
            s.finish_text.clear();
            s.finish = None;
        }
        if s.jobs.is_running("finish item") {
            ui.spinner();
        }
    });
    ui.separator();
    match &s.finish {
        None => {}
        Some(Err(e)) => {
            ui.colored_label(widgets::bad(dark), e);
        }
        Some(Ok(f)) => {
            ui.label(
                RichText::new(format!(
                    "{} ({}) · {}",
                    if f.item.item.name.is_empty() { &f.item.item.base } else { &f.item.item.name },
                    f.item.item.base,
                    f.group
                ))
                .strong(),
            );
            ui.horizontal_wrapped(|ui| {
                for m in &f.item.mods {
                    chip(ui, &format!("{}{}", m.family, m.tier.map(|t| format!(" T{t}")).unwrap_or_default()));
                }
            });
            if f.found.is_empty() {
                ui.label(RichText::new("no ladder product can be finished from this item").weak());
            }
            ScrollArea::vertical().auto_shrink(false).show(ui, |ui| finishes_table(ui, "finish", &f.found, divine, dark));
        }
    }
}

// -- log -------------------------------------------------------------------------------------

fn log(ui: &mut Ui, s: &mut State) {
    let running = s.jobs.running();
    if !running.is_empty() {
        for (name, t) in &running {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(format!("{name} · {:.0}s", t.as_secs_f64()));
            });
        }
        ui.separator();
    }
    ScrollArea::vertical().stick_to_bottom(true).auto_shrink(false).show(ui, |ui| {
        for (t, msg) in &s.log {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{:>6.1}s", (*t - s.started).as_secs_f64())).weak().monospace());
                ui.label(msg);
            });
        }
    });
}
