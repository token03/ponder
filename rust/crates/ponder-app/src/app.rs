//! The window: menu bar, the run and filter panel, a status bar, and the tiles in the middle.

use egui::{Align, Key, Layout, RichText, Ui};

use crate::panes::{self, Behavior, Pane};
use crate::state::{Saved, State};
use crate::view::{Filters, Kind};

const SAVED: &str = "ponder";
const TILES: &str = "ponder_tiles";

pub struct App {
    state: State,
    tree: egui_tiles::Tree<Pane>,
    show_panel: bool,
    about: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> App {
        let saved: Saved = cc.storage.and_then(|s| eframe::get_value(s, SAVED)).unwrap_or_default();
        let tree = cc.storage.and_then(|s| eframe::get_value(s, TILES)).unwrap_or_else(panes::default_tree);
        style(&cc.egui_ctx);
        App { state: State::new(&cc.egui_ctx, saved), tree, show_panel: true, about: false }
    }

    /// Show a pane: make its tab active, or add it back if it was closed.
    fn reveal(&mut self, pane: Pane) {
        let found = self.tree.tiles.iter().find_map(|(id, t)| matches!(t, egui_tiles::Tile::Pane(p) if *p == pane).then_some(*id));
        let id = match found {
            Some(id) => id,
            None => {
                let id = self.tree.tiles.insert_pane(pane);
                match self.tree.root {
                    Some(root) => {
                        let tabs = self.tree.tiles.insert_tab_tile(vec![id]);
                        if let Some(egui_tiles::Tile::Container(c)) = self.tree.tiles.get_mut(root) {
                            c.add_child(tabs);
                        }
                    }
                    None => self.tree.root = Some(id),
                }
                id
            }
        };
        self.tree.make_active(|tid, _| tid == id);
    }

    fn menu(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.add_enabled(self.state.data.is_some(), egui::Button::new("Export report JSON")).clicked() {
                    self.state.export();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Run", |ui| {
                let ready = self.state.db.is_some() && self.state.has_ladder && !self.state.jobs.is_running("ladder");
                if ui.add_enabled(ready, egui::Button::new("Run ladder").shortcut_text("Ctrl+R")).clicked() {
                    self.state.run_ladder();
                }
                ui.separator();
                if ui.button("Fetch ladder characters (poe.ninja)").clicked() {
                    self.state.fetch_ladder();
                }
                if ui.button("Fetch prices (poe.ninja)").clicked() {
                    self.state.fetch_prices();
                }
            });
            ui.menu_button("View", |ui| {
                ui.checkbox(&mut self.show_panel, "Run & filter panel");
                ui.separator();
                for pane in Pane::ALL {
                    if ui.button(pane.title()).clicked() {
                        self.reveal(pane);
                    }
                }
                ui.separator();
                if ui.button("Reset layout").clicked() {
                    self.tree = panes::default_tree();
                }
                ui.separator();
                egui::widgets::global_theme_preference_buttons(ui);
            });
            ui.menu_button("Help", |ui| {
                if ui.button("About ponder").clicked() {
                    self.about = true;
                }
            });
        });
    }

    fn side_panel(&mut self, ui: &mut Ui) {
        let s = &mut self.state;
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.heading("Run");
            let ready = s.db.is_some() && s.has_ladder;
            ui.horizontal(|ui| {
                let running = s.jobs.is_running("ladder");
                if ui.add_enabled(ready && !running, egui::Button::new(RichText::new("▶ Run ladder").strong())).clicked() {
                    s.run_ladder();
                }
                if running {
                    ui.spinner();
                }
            });
            ui.checkbox(&mut s.saved.auto_run, "run on start");
            let r = &mut s.saved.run;
            egui::CollapsingHeader::new("What to cost").default_open(true).show(ui, |ui| {
                egui::Grid::new("run").num_columns(2).show(ui, |ui| {
                    let num = |ui: &mut Ui, label: &str, hint: &str, v: &mut usize, max: usize| {
                        ui.label(label).on_hover_text(hint);
                        ui.add(egui::DragValue::new(v).range(0..=max));
                        ui.end_row();
                    };
                    num(ui, "gear archetypes", "most worn mod combinations to cost", &mut r.gear_top, 2000);
                    num(ui, "per base group", "also the top few of every base group", &mut r.per_group, 50);
                    num(ui, "niche combos", "3+ mod combos worn together far above chance", &mut r.niche, 500);
                    num(ui, "cluster combos", "cluster jewel notable combos", &mut r.cluster_top, 2000);
                    num(ui, "min characters", "rarer combinations are skipped", &mut r.min_characters, 200);
                    ui.label("only group").on_hover_text("only base groups / cluster enchants containing this text");
                    ui.add(egui::TextEdit::singleline(&mut r.group).desired_width(110.0).hint_text("any"));
                    ui.end_row();
                    ui.label("chaos per hour").on_hover_text("what an hour of clicking is worth; added to every plan's cost");
                    ui.add(egui::DragValue::new(&mut r.chaos_per_hour).range(0.0..=100_000.0).speed(10.0).suffix("c"));
                    ui.end_row();
                });
                ui.checkbox(&mut r.recombs, "recombinator trees").on_hover_text("slow the first time; plans are cached");
                ui.add_enabled_ui(s.has_cookie, |ui| {
                    ui.checkbox(&mut r.trade, "price on trade").on_disabled_hover_text("needs POESESSID in .env");
                    if r.trade {
                        ui.horizontal(|ui| {
                            ui.label("searches");
                            ui.add(egui::DragValue::new(&mut r.trade_budget).range(0..=500));
                        });
                    }
                });
            });
            ui.separator();
            filters(ui, s);
        });
    }

    fn status_bar(&self, ui: &mut Ui) {
        let s = &self.state;
        ui.horizontal(|ui| {
            if s.jobs.busy() {
                ui.spinner();
            }
            ui.label(&s.status);
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(d) = &s.data {
                    ui.label(RichText::new(format!("{} · divine {:.0}c · prices {}", d.report.league, d.report.divine, d.report.prices_source)).weak());
                }
                if !s.has_cookie {
                    ui.label(RichText::new("no POESESSID: trade off").weak()).on_hover_text("put POESESSID=... in a git-ignored .env to price on trade");
                }
            });
        });
    }

    fn keys(&mut self, ctx: &egui::Context) {
        if ctx.memory(|m| m.focused().is_some()) {
            return; // typing in a field
        }
        let (down, up, page_down, page_up, run) = ctx.input(|i| {
            (
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::PageDown),
                i.key_pressed(Key::PageUp),
                i.modifiers.command && i.key_pressed(Key::R),
            )
        });
        let s = &mut self.state;
        for (pressed, delta) in [(down, 1), (up, -1), (page_down, 20), (page_up, -20)] {
            if pressed {
                s.step_selection(delta);
            }
        }
        if run && s.db.is_some() && s.has_ladder {
            s.run_ladder();
        }
    }
}

fn filters(ui: &mut Ui, s: &mut State) {
    let before = s.saved.filters.clone();
    let groups = s.groups.clone();
    let f = &mut s.saved.filters;
    ui.horizontal(|ui| {
        ui.heading("Filter");
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled(!f.is_default(), egui::Button::new("reset")).clicked() {
                *f = Filters::default();
            }
        });
    });
    ui.add(egui::TextEdit::singleline(&mut f.text).hint_text("🔍 item, base, mod or skill").desired_width(f32::INFINITY));
    ui.horizontal(|ui| {
        for k in Kind::ALL {
            ui.selectable_value(&mut f.kind, k, k.label());
        }
    });
    egui::ComboBox::from_id_salt("group")
        .width(ui.available_width() - 8.0)
        .selected_text(if f.group.is_empty() { "every base group" } else { &f.group })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut f.group, String::new(), "every base group");
            for g in &groups {
                ui.selectable_value(&mut f.group, g.clone(), g);
            }
        });
    egui::Grid::new("filters").num_columns(2).show(ui, |ui| {
        ui.label("min demand");
        ui.add(egui::DragValue::new(&mut f.min_demand).range(0..=10_000).suffix(" chars"));
        ui.end_row();
        ui.label("max cost");
        ui.add(egui::DragValue::new(&mut f.max_cost).range(0.0..=10_000_000.0).speed(25.0).suffix("c"))
            .on_hover_text("cheapest way to make it; 0 for no limit");
        ui.end_row();
    });
    ui.checkbox(&mut f.only_market, "priced on trade");
    ui.checkbox(&mut f.only_fractures, "has a fractured-base option");
    ui.checkbox(&mut f.only_recomb, "has a recombinator tree");
    ui.checkbox(&mut f.only_sales, "sells as smaller products");
    ui.checkbox(&mut f.hide_errors, "hide failed estimates");
    if *f != before {
        s.refilter();
    }
    ui.label(RichText::new(format!("{} of {} shown", s.rows.len(), s.estimates().len())).weak());
}

fn style(ctx: &egui::Context) {
    ctx.all_styles_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 5.0);
        style.spacing.button_padding = egui::vec2(8.0, 3.0);
        style.interaction.selectable_labels = false; // rows are clicked, not selected as text
    });
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for msg in self.state.jobs.drain() {
            self.state.handle(msg);
        }
        // a pasted item lands in the Finish box even when it isn't focused
        let pasted = ctx.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Paste(t) if t.contains("Rarity:") => Some(t.clone()),
                _ => None,
            })
        });
        if let Some(t) = pasted
            && ctx.memory(|m| m.focused().is_none())
        {
            self.state.finish_text = t;
            self.reveal(Pane::Finish);
        }
        self.keys(ctx);
        if self.state.jobs.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status_bar(ui));
        if self.show_panel {
            egui::Panel::left("side").resizable(true).default_size(250.0).show(ui, |ui| self.side_panel(ui));
        }
        egui::CentralPanel::default().frame(egui::Frame::new().inner_margin(2)).show(ui, |ui| {
            self.tree.ui(&mut Behavior { state: &mut self.state }, ui);
        });
        if self.about {
            egui::Window::new("About ponder").open(&mut self.about).collapsible(false).resizable(false).show(ui.ctx(), |ui| {
                ui.label("ponder finds Path of Exile 3.30 market holes: what ladder characters wear, the cheapest way to craft it, and what trade has.");
                ui.label("Costs are expected chaos per finished item from the solver (currency, bench crafts and clicking time).");
                ui.label("Click a column header to sort; ↑/↓ and PgUp/PgDn move through the table; drag tabs to rearrange the panes.");
            });
        }
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, SAVED, &self.state.saved);
        eframe::set_value(storage, TILES, &self.tree);
    }
}
