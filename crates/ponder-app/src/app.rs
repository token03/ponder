//! The window: a top bar with the scan, the search and the layouts, filters and scan settings on
//! the left, a status bar, and the tiles in the middle.

use egui::{Align, Key, KeyboardShortcut, Layout, Modifiers, RichText, Ui};
use egui_tiles::{Tile, TileId};

use crate::panes::{self, Behavior, Pane};
use crate::state::{Saved, State};
use crate::tags;
use crate::theme;
use crate::view::Filters;

const SAVED: &str = "ponder";
const TILES: &str = "ponder_tiles";
const SCAN: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::R);
const SEARCH: KeyboardShortcut = KeyboardShortcut::new(Modifiers::COMMAND, Key::F);

pub struct App {
    state: State,
    tree: egui_tiles::Tree<Pane>,
    show_panel: bool,
    about: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let saved: Saved = cc.storage.and_then(|s| eframe::get_value(s, SAVED)).unwrap_or_default();
        let tree = cc.storage.and_then(|s| eframe::get_value(s, TILES)).unwrap_or_else(|| panes::Layout::Browse.tree());
        theme::apply(&cc.egui_ctx);
        App { state: State::new(&cc.egui_ctx, saved), tree, show_panel: true, about: false }
    }

    fn find(&self, pane: &Pane) -> Option<TileId> {
        self.tree.tiles.iter().find_map(|(id, tile)| matches!(tile, Tile::Pane(p) if p == pane).then_some(*id))
    }

    /// Show a pane: make its tab active, or add it as a tab of `tabs` (by default beside the
    /// selected craft, else at the root).
    fn reveal(&mut self, pane: Pane, tabs: Option<TileId>) {
        let id = match self.find(&pane) {
            Some(id) => id,
            None => {
                let near = tabs.or_else(|| self.find(&Pane::Craft).and_then(|craft| self.tree.tiles.parent_of(craft))).or(self.tree.root);
                let id = self.tree.tiles.insert_pane(pane);
                match near {
                    Some(parent) => {
                        if let Some(Tile::Container(container)) = self.tree.tiles.get_mut(parent) {
                            container.add_child(id);
                        }
                    }
                    None => self.tree.root = Some(id),
                }
                id
            }
        };
        self.tree.make_active(|tile, _| tile == id);
    }

    fn ready(&self) -> bool {
        self.state.db.is_some() && self.state.has_ladder && !self.state.jobs.is_running("ladder")
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let dark = ui.visuals().dark_mode;
        ui.horizontal(|ui| {
            ui.label(RichText::new("ponder").size(18.0).strong().color(theme::accent(dark)));
            ui.add_space(6.0);
            let scanning = self.state.jobs.is_running("ladder");
            let label = RichText::new(if scanning { "⟳ Scanning…" } else { "⟳ Scan ladder" }).strong().color(egui::Color32::BLACK);
            let button = egui::Button::new(label).fill(theme::accent(dark)).shortcut_text(ui.ctx().format_shortcut(&SCAN));
            if ui
                .add_enabled(self.ready(), button)
                .on_hover_text("Read the saved ladder characters, find the mod combinations they wear and cost each one")
                .on_disabled_hover_text(self.why_not_ready())
                .clicked()
            {
                self.state.scan();
            }
            let (done, total) = self.state.solved;
            if self.state.solving() {
                let progress = egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                    .desired_width(160.0)
                    .text(format!("solving {done}/{total}"))
                    .animate(true);
                ui.add(progress).on_hover_text("rows with a ring show a provisional plan until solved");
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button("☰", |ui| self.menu(ui));
                ui.menu_button("Layout", |ui| self.layouts(ui));
                ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                    ui.add_space(12.0);
                    let focus = ui.input_mut(|i| i.consume_shortcut(&SEARCH));
                    let state = &mut self.state;
                    let estimates = state.data.as_ref().map_or(&[][..], |data| data.report.estimates.as_slice());
                    let filters = &mut state.saved.filters;
                    if tags::search(ui, estimates, &mut filters.tags, &mut filters.text, focus) {
                        state.refilter();
                    }
                });
            });
        });
    }

    fn why_not_ready(&self) -> &'static str {
        if self.state.db.is_none() {
            "loading game data"
        } else if !self.state.has_ladder {
            "fetch ladder characters first (☰)"
        } else {
            "scanning"
        }
    }

    fn layouts(&mut self, ui: &mut Ui) {
        for layout in panes::Layout::ALL {
            if ui.button(layout.name()).clicked() {
                self.tree = layout.tree();
            }
        }
        ui.separator();
        ui.label(RichText::new("open").small().weak());
        for pane in Pane::ADDABLE {
            if ui.button(pane.title()).clicked() {
                self.reveal(pane, None);
            }
        }
        ui.separator();
        ui.checkbox(&mut self.show_panel, "filters & settings");
        egui::widgets::global_theme_preference_buttons(ui);
    }

    fn menu(&mut self, ui: &mut Ui) {
        if ui.button("Fetch ladder characters").on_hover_text("from poe.ninja").clicked() {
            self.state.fetch_ladder();
        }
        if ui.button("Fetch prices").on_hover_text("from poe.ninja").clicked() {
            self.state.fetch_prices();
        }
        if ui.add_enabled(self.state.data.is_some(), egui::Button::new("Export JSON")).clicked() {
            self.state.export();
        }
        ui.separator();
        if ui.button("About").clicked() {
            self.about = true;
        }
        if ui.button("Quit").clicked() {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn side_panel(&mut self, ui: &mut Ui) {
        let s = &mut self.state;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            filters(ui, s);
            ui.add_space(8.0);
            ui.separator();
            egui::CollapsingHeader::new(RichText::new("Scan").strong()).default_open(false).show(ui, |ui| {
                let r = &mut s.saved.scan;
                egui::Grid::new("scan").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                    let num = |ui: &mut Ui, label: &str, hint: &str, v: &mut usize, max: usize| {
                        ui.label(label).on_hover_text(hint);
                        ui.add(egui::DragValue::new(v).range(0..=max));
                        ui.end_row();
                    };
                    num(ui, "gear crafts", "the most worn mod combinations to cost", &mut r.gear_top, 2000);
                    num(ui, "cluster jewels", "the most worn cluster jewel notable combinations to cost", &mut r.cluster_top, 2000);
                    num(ui, "min wearers", "skip combinations fewer characters wear", &mut r.min_characters, 200);
                    ui.label("time").on_hover_text("what an hour at the bench is worth; added to every cost");
                    ui.add(egui::DragValue::new(&mut r.chaos_per_hour).range(0.0..=100_000.0).speed(10.0).suffix(" c/h"));
                    ui.end_row();
                });
                egui::CollapsingHeader::new(RichText::new("more").small()).show(ui, |ui| {
                    egui::Grid::new("scan more").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
                        ui.label("per base group").on_hover_text("also cost the top few of every base group");
                        ui.add(egui::DragValue::new(&mut r.per_group).range(0..=50));
                        ui.end_row();
                        ui.label("niche combos").on_hover_text("also cost 3+ mod combinations worn together far above chance");
                        ui.add(egui::DragValue::new(&mut r.niche).range(0..=500));
                        ui.end_row();
                    });
                });
                ui.add_enabled_ui(s.has_cookie, |ui| {
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut r.trade, "price on trade").on_disabled_hover_text("needs POESESSID in .env");
                        if r.trade {
                            ui.add(egui::DragValue::new(&mut r.trade_budget).range(0..=500).suffix(" searches"));
                        }
                    });
                });
                ui.label(RichText::new("applies on the next scan").small().weak());
            });
        });
    }

    fn status_bar(&self, ui: &mut Ui) {
        let s = &self.state;
        ui.horizontal(|ui| {
            if s.jobs.busy() {
                ui.spinner();
            }
            ui.add(egui::Label::new(RichText::new(&s.status).small()).truncate());
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if !s.has_cookie {
                    ui.label(RichText::new("trade off").small().weak())
                        .on_hover_text("put POESESSID=... in a git-ignored .env to price on trade");
                }
                if let Some(d) = &s.data {
                    ui.label(
                        RichText::new(format!("{} · 1 div = {:.0}c · {}", d.report.league, d.report.divine, d.report.prices_source))
                            .small()
                            .weak(),
                    );
                }
            });
        });
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_shortcut(&SCAN)) && self.ready() {
            self.state.scan();
        }
        if ctx.memory(|m| m.focused().is_some()) {
            return;
        }
        let steps = ctx.input(|i| {
            [(Key::ArrowDown, 1), (Key::ArrowUp, -1), (Key::PageDown, 20), (Key::PageUp, -20)]
                .into_iter()
                .filter(|(key, _)| i.key_pressed(*key))
                .map(|(_, delta)| delta)
                .sum::<isize>()
        });
        if steps != 0 {
            self.state.step_selection(steps);
        }
    }

    fn draw(&mut self, ui: &mut Ui) {
        egui::Panel::top("top")
            .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(10, 6)))
            .show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.show_panel {
            egui::Panel::left("side")
                .resizable(true)
                .default_size(240.0)
                .frame(egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(10, 10)))
                .show(ui, |ui| self.side_panel(ui));
        }
        let mut behavior = Behavior { state: &mut self.state, add: None };
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(3)).show(ui, |ui| self.tree.ui(&mut behavior, ui));
        if let Some((tabs, pane)) = behavior.add {
            self.reveal(pane, Some(tabs));
        }
        if let Some(index) = self.state.pin.take() {
            self.reveal(Pane::Pinned(State::id(&self.state.estimates()[index])), None);
        }
        egui::Window::new("About ponder").open(&mut self.about).collapsible(false).resizable(false).show(ui.ctx(), |ui| {
            ui.label("What ladder characters wear, the cheapest way to craft it, and what trade has (Path of Exile 3.30).");
            ui.label("Costs are expected chaos per finished item: currency, bench crafts and time.");
            ui.label("Scans try imprints only on Awakener's Orb crafts; ★ every mechanic checks one craft with memory strands, Dominance, Hinekora's Lock and the Recombinator too.");
            ui.label("⬆⬇ PgUp PgDn move through the crafts · double-click one to pin it · drag tabs to rearrange · ➕ adds a pane.");
        });
        self.state.toasts.show(ui.ctx());
    }
}

fn filters(ui: &mut Ui, s: &mut State) {
    let before = s.saved.filters.clone();
    let f = &mut s.saved.filters;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Filters").strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled(!f.is_default(), egui::Button::new("reset").small()).clicked() {
                *f = Filters::default();
            }
        });
    });
    s.catalog.ui(ui, &mut f.items);
    egui::Grid::new("filters").num_columns(2).spacing([10.0, 6.0]).show(ui, |ui| {
        ui.label("wearers ≥");
        ui.add(egui::DragValue::new(&mut f.min_demand).range(0..=10_000));
        ui.end_row();
        ui.label("total ≤").on_hover_text("crafting and bases; 0 for any");
        ui.add(egui::DragValue::new(&mut f.max_cost).range(0.0..=10_000_000.0).speed(25.0).suffix("c"));
        ui.end_row();
    });
    ui.checkbox(&mut f.only_market, "priced on trade");
    ui.checkbox(&mut f.hide_errors, "hide failed");
    if *f != before {
        s.refilter();
    }
    ui.label(RichText::new(format!("{} of {}", s.rows.len(), s.estimates().len())).small().weak());
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for msg in self.state.jobs.drain() {
            self.state.handle(msg);
        }
        self.keys(ctx);
        if self.state.jobs.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SAVED, &self.state.saved);
        eframe::set_value(storage, TILES, &self.tree);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_ui_renders_headlessly() {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        for layout in panes::Layout::ALL {
            let mut app = App { state: State::new(&ctx, Saved::default()), tree: layout.tree(), show_panel: true, about: false };
            for size in [[1500.0, 900.0], [900.0, 500.0]] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0], size[1]))),
                    ..Default::default()
                };
                let mut output = ctx.run_ui(input, |ui| app.draw(ui));
                let primitives = ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
                output.drop_without_applying_deltas();
                assert!(!primitives.is_empty());
            }
        }
    }
}
