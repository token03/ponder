use std::collections::{BTreeMap, BTreeSet, HashMap};

use egui::{RichText, Ui};
use ponder::gear;
use ponder::pipeline::Estimate;
use ponder_game::data::{BaseItem, BaseItems};
use ponder_game::mods::ModDB;
use serde::{Deserialize, Serialize};

type ItemPath = Vec<String>;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ItemSelection {
    pub paths: BTreeSet<ItemPath>,
}

impl ItemSelection {
    fn toggle(&mut self, path: &[String]) {
        if !self.paths.remove(path) {
            self.paths.retain(|p| !p.starts_with(path) && !path.starts_with(p));
            self.paths.insert(path.to_vec());
        }
    }

    fn admits(&self, path: &[String]) -> bool {
        self.paths.is_empty() || self.paths.iter().any(|p| path.starts_with(p))
    }

    fn has_descendants(&self, path: &[String]) -> bool {
        self.paths.iter().any(|p| p.len() > path.len() && p.starts_with(path))
    }
}

#[derive(Default)]
struct Node {
    path: ItemPath,
    children: BTreeMap<String, Node>,
    count: usize,
}

impl Node {
    fn insert(&mut self, path: &[String]) {
        if let Some(label) = path.get(self.path.len()) {
            let mut child_path = self.path.clone();
            child_path.push(label.clone());
            self.children.entry(label.clone()).or_insert_with(|| Node { path: child_path, ..Default::default() }).insert(path);
        }
    }

    fn matches(&self, words: &[String]) -> bool {
        self.matches_path(words) || self.children.values().any(|child| child.matches(words))
    }

    fn matches_path(&self, words: &[String]) -> bool {
        let label = self.path.join(" ").to_lowercase();
        words.iter().all(|word| label.split(|c: char| !c.is_alphanumeric()).any(|token| token.starts_with(word)))
    }

    fn set_counts(&mut self, counts: &HashMap<ItemPath, usize>) {
        self.count = counts.get(&self.path).copied().unwrap_or_default();
        for child in self.children.values_mut() {
            child.set_counts(counts);
        }
    }

    fn checkbox(&self, ui: &mut Ui, selected: &mut ItemSelection) {
        let mut checked = selected.paths.contains(&self.path);
        let partial = selected.has_descendants(&self.path);
        let label = self.path.last().map(String::as_str).unwrap_or_default();
        ui.horizontal(|ui| {
            if ui.add(egui::Checkbox::new(&mut checked, label).indeterminate(partial)).on_hover_text(self.path.join(" → ")).changed() {
                selected.toggle(&self.path);
            }
            ui.label(RichText::new(self.count.to_string()).weak().small()).on_hover_text("compatible crafts in this run");
        });
    }

    fn ui(&self, ui: &mut Ui, selected: &mut ItemSelection, words: &[String]) {
        if !self.matches(words) {
            return;
        }
        if self.children.is_empty() {
            ui.indent((&self.path, "leaf"), |ui| self.checkbox(ui, selected));
            return;
        }
        let id = ui.make_persistent_id((&self.path, words));
        let open = if words.is_empty() { self.path.len() == 1 || selected.has_descendants(&self.path) } else { !self.matches_path(words) };
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, open);
        state.show_header(ui, |ui| self.checkbox(ui, selected)).body(|ui| {
            for child in self.children.values() {
                child.ui(ui, selected, words);
            }
        });
    }
}

#[derive(Default)]
pub struct ItemCatalog {
    root: Node,
    bases: HashMap<String, usize>,
    groups: Vec<Vec<ItemPath>>,
    search: String,
}

impl ItemCatalog {
    pub fn new(db: &ModDB) -> Self {
        Self::from_bases(&db.bases, gear::spawn_tags())
    }

    fn from_bases(bases: &BaseItems, spawn_tags: &BTreeSet<String>) -> Self {
        let mut catalog = Self::default();
        let mut groups = BTreeMap::new();
        for base in bases.values().filter(|base| base.release_state == "released") {
            let Some(path) = base_path(base) else {
                continue;
            };
            catalog.root.insert(&path);
            let signature: BTreeSet<&str> = base.tags.iter().map(String::as_str).filter(|tag| spawn_tags.contains(*tag)).collect();
            let cluster = if base.domain == "affliction_jewel" { base.name.as_str() } else { "" };
            groups
                .entry((base.domain.as_str(), base.item_class.as_str(), signature, cluster))
                .or_insert_with(BTreeMap::new)
                .insert(base.name.clone(), path);
        }
        for paths in groups.into_values() {
            let index = catalog.groups.len();
            for name in paths.keys() {
                catalog.bases.insert(name.clone(), index);
            }
            catalog.groups.push(paths.into_values().collect());
        }
        catalog
    }

    fn paths(&self, estimate: &Estimate) -> Vec<&[String]> {
        let index =
            self.bases.get(&estimate.base).or_else(|| self.bases.get(&estimate.used_base)).or_else(|| self.bases.get(&estimate.title));
        match index {
            Some(&index) => self.groups[index].iter().map(Vec::as_slice).collect(),
            None => Vec::new(),
        }
    }

    pub fn admits(&self, estimate: &Estimate, selected: &ItemSelection) -> bool {
        if selected.paths.is_empty() {
            return true;
        }
        let paths = self.paths(estimate);
        if paths.is_empty() {
            let fallback = if estimate.kind == "cluster" { vec!["Jewels".into(), "Clusters".into()] } else { vec!["Gear".into()] };
            return selected.admits(&fallback);
        }
        paths.iter().any(|path| selected.admits(path))
    }

    pub fn update_counts(&mut self, estimates: &[Estimate]) {
        let mut counts = HashMap::new();
        for estimate in estimates {
            let mut seen = BTreeSet::new();
            for path in self.paths(estimate) {
                for depth in 1..=path.len() {
                    seen.insert(path[..depth].to_vec());
                }
            }
            for path in seen {
                *counts.entry(path).or_default() += 1;
            }
        }
        self.root.set_counts(&counts);
    }

    pub fn ui(&mut self, ui: &mut Ui, selected: &mut ItemSelection) {
        let summary = match selected.paths.len() {
            0 => "Everything".to_string(),
            1 => selection_label(selected.paths.first().unwrap()),
            n => format!("{n} item selections"),
        };
        let arrow = egui::Atom::paint(egui::Vec2::splat(10.0), |ui, args| {
            ui.painter().add(egui::Shape::rotated_triangle(args.rect, 0.0, ui.visuals().text_color()));
        });
        let response = ui
            .add(egui::Button::new(summary).right_text(arrow).min_size(egui::vec2(ui.available_width(), 0.0)))
            .on_hover_text("Choose item types and bases. Crafts cover bases sharing a mod pool; costs use the displayed best base.");
        egui::Popup::menu(&response).width(354.0).close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside).show(|ui| {
            ui.set_width(340.0);
            ui.style_mut().animation_time = 0.0;
            ui.spacing_mut().indent = 12.0;
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.add(egui::TextEdit::singleline(&mut self.search).hint_text("Find a type or base…").desired_width(f32::INFINITY));
            if ui.selectable_label(selected.paths.is_empty(), "Everything").clicked() {
                selected.paths.clear();
            }
            let words: Vec<String> =
                self.search.split(|c: char| !c.is_alphanumeric()).filter(|word| !word.is_empty()).map(str::to_lowercase).collect();
            egui::ScrollArea::vertical().max_height(280.0).min_scrolled_height(280.0).auto_shrink([false, false]).show(ui, |ui| {
                let mut found = false;
                for node in self.root.children.values() {
                    if node.matches(&words) {
                        found = true;
                        node.ui(ui, selected, &words);
                    }
                }
                if !found {
                    ui.label(if self.root.children.is_empty() {
                        "Base types appear when game data loads."
                    } else {
                        "No matching bases. Try a slot, defence or name."
                    });
                }
            });
            ui.separator();
            ui.label(RichText::new("Select several branches; choose a child to narrow.").small().weak());
        });
        if selected.paths.is_empty() {
            return;
        }
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            for path in &selected.paths {
                if ui.small_button(format!("{} ×", selection_label(path))).on_hover_text(path.join(" → ")).clicked() {
                    remove = Some(path.clone());
                }
            }
        });
        if let Some(path) = remove {
            selected.paths.remove(&path);
        }
    }
}

fn selection_label(path: &[String]) -> String {
    if path.len() == 4 && path.get(1).is_some_and(|p| p == "Armour") {
        return format!("{} · {}", path[2], path[3]);
    }
    if path.len() == 3 && path.get(1).is_some_and(|p| p == "Clusters") {
        return format!("{} clusters", path[2]);
    }
    path.last().cloned().unwrap_or_default()
}

fn base_path(base: &BaseItem) -> Option<ItemPath> {
    let has = |tag: &str| base.tags.iter().any(|t| t == tag);
    let path: Vec<&str> = if base.domain == "affliction_jewel" {
        let size = if has("expansion_jewel_small") {
            "Small"
        } else if has("expansion_jewel_medium") {
            "Medium"
        } else if has("expansion_jewel_large") {
            "Large"
        } else {
            return None;
        };
        vec!["Jewels", "Clusters", size]
    } else if base.item_class == "Jewel" && base.domain == "misc" {
        vec!["Jewels", "Base jewels", &base.name]
    } else if base.item_class == "AbyssJewel" && base.domain == "abyss_jewel" {
        vec!["Jewels", "Abyss jewels", &base.name]
    } else if base.domain != "item" {
        return None;
    } else if has("weapon") {
        vec!["Gear", "Weapons", &base.item_class, &base.name]
    } else if has("armour") {
        let defence = base
            .tags
            .iter()
            .filter_map(|tag| tag.strip_suffix("_armour").or_else(|| tag.strip_suffix("_shield")))
            .find_map(|tag| {
                let names: Option<Vec<_>> = tag.split('_').map(gear::defence_name).collect();
                names.map(|names| names.join("/"))
            })
            .unwrap_or_else(|| if has("ward_armour") { "Ward".into() } else { "Other".into() });
        return Some(["Gear", "Armour", &base.item_class, &defence, &base.name].map(String::from).to_vec());
    } else if matches!(base.item_class.as_str(), "Ring" | "Amulet" | "Belt" | "Quiver") {
        vec!["Gear", "Accessories", &base.item_class, &base.name]
    } else {
        return None;
    };
    Some(path.into_iter().map(String::from).collect())
}
