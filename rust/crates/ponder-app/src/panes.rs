//! The tiles: each pane draws one view of the run or of the selected estimate.

use egui::{Align, Color32, Layout, RichText, ScrollArea, Sense, Ui};
use egui_extras::{Column, TableBuilder};
use egui_plot::{Legend, Plot, PlotPoints, Points};
use ponder::pipeline::{Estimate, Finish};
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
            Pane::Fractures => "Fractured bases",
            Pane::Products => "Products & starts",
            Pane::Recomb => "Recombinator",
            Pane::Finish => "Finish an item",
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
                Some(n) if n > 0 => format!("Fractured bases ({n})").into(),
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
    if s.selected.is_none() {
        empty(ui, "select a craft in the table");
    }
    s.selected
}

// -- crafts table ----------------------------------------------------------------------------

fn crafts(ui: &mut Ui, s: &mut State) {
    if no_run(ui, s) {
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
                            if e.error.is_some() {
                                r.on_hover_text(e.error.as_deref().unwrap_or_default());
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
    ui.label(RichText::new("Cheapest way to make each craft against how many ladder characters wear it. Click a point to select it.").weak());
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
                plot.points(Points::new(*name, PlotPoints::from(pts.iter().map(|(_, p)| *p).collect::<Vec<_>>())).color(*color).radius(3.0));
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
    let e = s.estimates()[i].clone();
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(&e.title);
            ui.label(RichText::new(if e.kind == "cluster" { "cluster jewel" } else { "gear" }).weak());
        });
        ui.label(format!(
            "{} · {} base{} share this mod pool · item level {}{}",
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
        if let Some(err) = &e.error {
            ui.colored_label(widgets::bad(dark), err);
        }
        ui.add_space(4.0);
        actions(ui, s, &e);
        ui.separator();

        // mod lines are long: one section above the other
        mods(ui, &e, dark);
        ui.add_space(6.0);
        costs(ui, &e, divine, dark);
        ui.separator();
        currency(ui, &e, divine);
    });
}

fn actions(ui: &mut Ui, s: &mut State, e: &Estimate) {
    let i = s.selected.unwrap_or(0);
    ui.horizontal_wrapped(|ui| {
        let busy = |s: &State, name: &str| s.jobs.is_running(&format!("{name} #{i}"));
        let trade_hint = if s.has_cookie { "" } else { " (needs POESESSID in .env)" };
        if ui
            .add_enabled(s.has_cookie && !busy(s, "trade prices"), egui::Button::new("💰 Price on trade"))
            .on_hover_text(format!("market price of the finished item, its base and the best fractured bases{trade_hint}"))
            .on_disabled_hover_text(format!("price on trade{trade_hint}"))
            .clicked()
        {
            s.price_selected();
        }
        if ui
            .add_enabled(s.has_cookie && e.kind == "gear" && !busy(s, "half-done items"), egui::Button::new("🔎 Find half-done items"))
            .on_hover_text(format!("listed items that already have some targets, and what finishing them costs{trade_hint}"))
            .clicked()
        {
            s.starts_selected();
        }
        if ui
            .add_enabled(e.kind == "gear" && !busy(s, "recombinator"), egui::Button::new("♻ Cost recombinator"))
            .on_hover_text("cheapest recombinator tree of clean donors (NNN padding included)")
            .clicked()
        {
            s.recomb_selected();
        }
        if ui.button("📋 JSON").on_hover_text("copy this estimate as JSON").clicked() {
            ui.ctx().copy_text(serde_json::to_string_pretty(&e.to_dict()).unwrap_or_default());
        }
    });
    link_row(ui, "trade search for the finished item", e.trade_url.as_deref());
    link_row(ui, "trade search for the base", e.base_url.as_deref());
}

fn costs(ui: &mut Ui, e: &Estimate, divine: f64, dark: bool) {
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
        line(ui, "craft", money(e.craft_cost(), divine), "expected".into());
        if !e.bench.is_empty() {
            line(ui, "bench", money(Some(e.bench_cost()), divine), e.bench.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(", "));
        }
        if e.base_price.is_some() || !e.fixed.is_empty() {
            let note = if e.fixed.is_empty() { e.base_source.clone() } else { format!("fractured: {}", e.fixed.join(" + ")) };
            line(ui, "base", money(e.base_price.map(|b| b.0), divine), note);
        }
        line(ui, "total", money(e.total(), divine).strong(), String::new());
        if let Some(f) = e.best_fracture() {
            line(ui, "fractured", money(f.total(), divine), format!("from a fractured {}", f.target));
        }
        if e.recomb.is_some() {
            line(ui, "recomb", money(e.recomb_total(), divine), "recombinator tree".into());
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
    let Some(plan) = &e.plan else { return };
    let usage = e.usage();
    let currency = plan.currency(&e.settings);
    ui.label(RichText::new("Expected currency per finished item").strong());
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
            "{} in currency · ~{} clicking (charged {} at {:.0}c/h) · {} solver states",
            view::money(Some(currency), divine),
            view::hours(plan.hours),
            view::money(Some(plan.cost - currency), divine),
            e.settings.chaos_per_hour,
            plan.states
        ))
        .weak(),
    );
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
    let steps = e.steps();
    if steps.is_empty() {
        empty(ui, e.error.as_deref().unwrap_or("no plan"));
        return;
    }
    let jump = s.jump;
    let mut go = None;
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        for st in &steps {
            let frame = egui::Frame::group(ui.style()).fill(if jump == Some(st.number) { ui.visuals().selection.bg_fill.gamma_multiply(0.3) } else { Color32::TRANSPARENT });
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
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(format!("reached ×{:.2} · {:.1} actions each time", st.reached, st.per_reach())).weak().small());
                    });
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
    if go.is_some() {
        s.jump = go;
    }
}

// -- fractured bases -------------------------------------------------------------------------

fn fractures(ui: &mut Ui, s: &mut State) {
    let Some(i) = no_selection(ui, s) else { return };
    let dark = ui.visuals().dark_mode;
    let divine = s.divine();
    let e = &s.estimates()[i];
    if e.fractures.is_empty() {
        empty(ui, "no fractured-base options (they need 2+ wanted mods)");
        return;
    }
    let scratch = e.total();
    ui.label(RichText::new("Start from a base with one target already fractured, then craft the rest.").weak());
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
                        ui.label(money(f.craft_cost(), divine));
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
    let dark = ui.visuals().dark_mode;
    let e = s.estimates()[i].clone();
    let mut go = None;
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        let mut list = |ui: &mut Ui, title: &str, hint: &str, refs: &[ponder::pipeline::ProductRef], idx: &[usize]| {
            ui.label(RichText::new(title).strong()).on_hover_text(hint);
            if refs.is_empty() {
                ui.label(RichText::new("none").weak());
            }
            for (k, p) in refs.iter().enumerate() {
                ui.horizontal(|ui| {
                    let text = format!("{} · {}", p.title, p.targets.join(" + "));
                    match idx.get(k) {
                        Some(&j) => {
                            if ui.link(text).clicked() {
                                go = Some(j);
                            }
                        }
                        None => {
                            ui.label(text);
                        }
                    }
                    ui.label(money(p.cheapest, divine));
                });
            }
            ui.add_space(6.0);
        };
        list(ui, "Smaller products", "fewer targets: what a failed or half-way craft can be sold as", &e.smaller_products, &e.smaller);
        list(ui, "Bigger products", "more targets: what this can be crafted on into", &e.bigger_products, &e.bigger);
        ui.separator();
        ui.label(RichText::new("Half-done items on trade").strong());
        if e.starts.is_empty() {
            ui.label(RichText::new("none fetched yet: Item ➡ Find half-done items").weak());
        } else {
            link_row(ui, "the search", e.starts_url.as_deref());
            finishes_table(ui, "starts", &e.starts, divine, dark);
        }
    });
    if let Some(j) = go {
        s.select(j);
        s.reveal = true;
    }
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
    let e = &s.estimates()[i];
    let Some(r) = &e.recomb else {
        empty(ui, "no recombinator tree yet: Item ➡ Cost recombinator, or tick recombinators in the run settings");
        return;
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("cheapest tree").strong());
        ui.label(money(r.cost, divine).strong());
        ui.label(RichText::new(format!("+ bench = {}", view::money(e.recomb_total(), divine))).weak());
    });
    ScrollArea::vertical().auto_shrink(false).show(ui, |ui| json_tree(ui, "recomb", &r.json, divine));
}

/// A JSON value as collapsible sections; numbers under cost-like keys as money.
fn json_tree(ui: &mut Ui, id: &str, v: &Value, divine: f64) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                match x {
                    Value::Object(_) | Value::Array(_) if !is_flat(x) => {
                        egui::CollapsingHeader::new(k).id_salt((id, k)).default_open(true).show(ui, |ui| json_tree(ui, &format!("{id}.{k}"), x, divine));
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
                egui::CollapsingHeader::new(format!("{}", n + 1)).id_salt((id, n)).default_open(true).show(ui, |ui| json_tree(ui, &format!("{id}.{n}"), x, divine));
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
        Value::Array(a) => RichText::new(a.iter().map(|x| x.as_str().map_or_else(|| x.to_string(), str::to_string)).collect::<Vec<_>>().join(", ")),
        Value::String(s) => RichText::new(s),
        Value::Null => RichText::new("–").weak(),
        x => RichText::new(x.to_string()).monospace(),
    }
}

// -- finishing a real item -------------------------------------------------------------------

fn finish(ui: &mut Ui, s: &mut State) {
    let divine = s.divine();
    let dark = ui.visuals().dark_mode;
    ui.label(RichText::new("Paste an item (Ctrl+C or Ctrl+Alt+C in game, or PoB text): what can it be finished into, and for how much?").weak());
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
            ui.label(RichText::new(format!("{} ({}) · {}", if f.item.item.name.is_empty() { &f.item.item.base } else { &f.item.item.name }, f.item.item.base, f.group)).strong());
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
