//! The tiles in the middle of the window, which can be dragged into any arrangement; each tab
//! bar's ➕ adds a pane next to it.

mod craft;
mod crafts;
mod log;
mod map;
mod pipeline;
mod recipe;

use egui::Ui;
use egui_tiles::{Container, Tile, TileId, Tiles, Tree};
use serde::{Deserialize, Serialize};

use crate::state::State;
use crate::theme;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Pane {
    Crafts,
    /// the selected craft
    Craft,
    Recipe,
    Map,
    Pipeline,
    Log,
    /// one craft kept open, by its id (see `State::id`)
    Pinned(String),
}

impl Pane {
    /// The panes the ➕ menus offer.
    pub const ADDABLE: [Pane; 6] = [Pane::Crafts, Pane::Craft, Pane::Recipe, Pane::Map, Pane::Pipeline, Pane::Log];

    pub fn title(&self) -> String {
        match self {
            Pane::Crafts => "☰ Crafts".into(),
            Pane::Craft => "💎 Craft".into(),
            Pane::Recipe => "⚒ Recipe".into(),
            Pane::Map => "📈 Market".into(),
            Pane::Pipeline => "⛃ Pipeline".into(),
            Pane::Log => "📜 Log".into(),
            Pane::Pinned(id) => format!("📌 {}", id.split_once('\u{1f}').map_or(id.as_str(), |(title, _)| title)),
        }
    }
}

/// Arrangements to start from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    /// the crafts beside the selected one, its recipe below
    Browse,
    /// the recipe large, the craft and the list beside it
    Recipe,
    /// the market map beside the crafts
    Market,
}

impl Layout {
    pub const ALL: [Layout; 3] = [Layout::Browse, Layout::Recipe, Layout::Market];

    pub fn name(self) -> &'static str {
        match self {
            Layout::Browse => "Browse",
            Layout::Recipe => "Recipe",
            Layout::Market => "Market",
        }
    }

    pub fn tree(self) -> Tree<Pane> {
        let mut tiles = Tiles::default();
        let tabs = |tiles: &mut Tiles<Pane>, panes: Vec<Pane>| {
            let ids = panes.into_iter().map(|pane| tiles.insert_pane(pane)).collect();
            tiles.insert_tab_tile(ids)
        };
        let root = match self {
            Layout::Browse => {
                let crafts = tabs(&mut tiles, vec![Pane::Crafts, Pane::Pipeline]);
                let craft = tabs(&mut tiles, vec![Pane::Craft]);
                let below = tabs(&mut tiles, vec![Pane::Recipe, Pane::Map, Pane::Log]);
                let detail = tiles.insert_vertical_tile(vec![craft, below]);
                share(&mut tiles, detail, craft, 1.5);
                let root = tiles.insert_horizontal_tile(vec![crafts, detail]);
                share(&mut tiles, root, detail, 1.2);
                root
            }
            Layout::Recipe => {
                let crafts = tabs(&mut tiles, vec![Pane::Crafts]);
                let recipe = tabs(&mut tiles, vec![Pane::Recipe]);
                let craft = tabs(&mut tiles, vec![Pane::Craft, Pane::Log]);
                let root = tiles.insert_horizontal_tile(vec![crafts, recipe, craft]);
                share(&mut tiles, root, recipe, 2.2);
                share(&mut tiles, root, craft, 1.2);
                root
            }
            Layout::Market => {
                let map = tabs(&mut tiles, vec![Pane::Map]);
                let crafts = tabs(&mut tiles, vec![Pane::Crafts]);
                let left = tiles.insert_vertical_tile(vec![map, crafts]);
                let craft = tabs(&mut tiles, vec![Pane::Craft, Pane::Recipe, Pane::Pipeline]);
                tiles.insert_horizontal_tile(vec![left, craft])
            }
        };
        Tree::new(egui::Id::unique("ponder_tiles"), root, tiles)
    }
}

fn share(tiles: &mut Tiles<Pane>, parent: TileId, child: TileId, share: f32) {
    if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(parent) {
        linear.shares.set_share(child, share);
    }
}

pub struct Behavior<'a> {
    pub state: &'a mut State,
    /// a pane asked for from a tab bar's ➕, and the tabs to put it in
    pub add: Option<(TileId, Pane)>,
}

impl egui_tiles::Behavior<Pane> for Behavior<'_> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        pane.title().into()
    }

    fn pane_ui(&mut self, ui: &mut Ui, _: TileId, pane: &mut Pane) -> egui_tiles::UiResponse {
        egui::Frame::new().inner_margin(egui::Margin::symmetric(8, 6)).show(ui, |ui| {
            ui.set_min_size(ui.available_size());
            match pane {
                Pane::Crafts => crafts::ui(ui, self.state),
                Pane::Craft => match self.state.selected {
                    Some(index) => craft::ui(ui, self.state, index),
                    None => crate::widgets::empty(ui, "💎", "Select a craft"),
                },
                Pane::Pinned(id) => match self.state.find(id) {
                    Some(index) => craft::ui(ui, self.state, index),
                    None => crate::widgets::empty(ui, "📌", "Not in this scan"),
                },
                Pane::Recipe => recipe::pane(ui, self.state),
                Pane::Map => map::ui(ui, self.state),
                Pane::Pipeline => pipeline::ui(ui, self.state),
                Pane::Log => log::ui(ui, self.state),
            }
        });
        egui_tiles::UiResponse::None
    }

    fn top_bar_right_ui(&mut self, _: &Tiles<Pane>, ui: &mut Ui, tile_id: TileId, _: &egui_tiles::Tabs, _: &mut f32) {
        ui.menu_button("➕", |ui| {
            for pane in Pane::ADDABLE {
                if ui.button(pane.title()).clicked() {
                    self.add = Some((tile_id, pane));
                }
            }
        });
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions { all_panes_must_have_tabs: true, ..Default::default() }
    }

    fn is_tab_closable(&self, _: &Tiles<Pane>, _: TileId) -> bool {
        true
    }

    fn tab_bar_height(&self, _: &egui::Style) -> f32 {
        28.0
    }

    fn gap_width(&self, _: &egui::Style) -> f32 {
        3.0
    }

    fn tab_outline_stroke(&self, visuals: &egui::Visuals, _: &Tiles<Pane>, _: TileId, state: &egui_tiles::TabState) -> egui::Stroke {
        if state.active { egui::Stroke::new(1.0, theme::accent(visuals.dark_mode).gamma_multiply(0.5)) } else { egui::Stroke::NONE }
    }
}
