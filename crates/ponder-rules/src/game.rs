//! Game data interned for the rules: mods, bases and which mods can roll where.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use crate::data::{self, Repoe};
use crate::goal::{Influence, Source};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BaseId(pub u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GroupId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TagId(pub u16);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    Prefix,
    Suffix,
}

pub struct ModDef {
    pub key: String,
    pub side: Side,
    pub source: Source,
    pub groups: Vec<GroupId>,
    /// Tags fossils, Harvest and metamods read (`implicit_tags`).
    pub tags: Vec<TagId>,
    pub level: u32,
    pub template: String,
    /// The influence an influenced mod needs on the item.
    pub influence: Option<Influence>,
    /// The first matching tag decides the weight; zero means it cannot roll there.
    spawn: Vec<(TagId, u32)>,
    /// Percent multipliers by tag.
    generation: Vec<(TagId, u32)>,
}

pub struct BaseDef {
    pub name: String,
    pub class: String,
    tags: Vec<TagId>,
}

/// Everything that decides which mods can roll: base, item level and influences.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Kind {
    pub base: BaseId,
    pub ilvl: u8,
    pub influences: Influences,
}

/// A set of influences as bits in `Influence` order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Influences(pub u8);

impl Influences {
    pub fn of(list: &[Influence]) -> Self {
        Influences(list.iter().fold(0, |b, i| b | 1 << *i as u8))
    }

    pub fn iter(self) -> impl Iterator<Item = Influence> {
        Influence::ALL.into_iter().filter(move |i| self.0 & 1 << *i as u8 != 0)
    }
}

/// The mods that can roll on one item kind (natural, influenced, fossil-only, unveiled) with their
/// spawn weights, and the natural tiers per template.
pub struct Pool {
    pub mods: Vec<(ModId, f64)>,
    /// Template -> natural mods of that template on this kind, highest level first.
    pub tiers: HashMap<String, Vec<ModId>>,
}

pub struct Game {
    pub mods: Vec<ModDef>,
    pub bases: Vec<BaseDef>,
    groups: Vec<String>,
    tags: Vec<String>,
    tag_ids: HashMap<String, TagId>,
    mod_ids: HashMap<String, ModId>,
    base_ids: HashMap<String, BaseId>,
    /// Lowercased template -> every mod with it.
    templates: HashMap<String, Vec<ModId>>,
    /// Bench metamods: [prefixes locked, suffixes locked, no attack, no caster, multimod].
    pub metamods: Vec<ModId>,
    pub essences: Vec<data::Essence>,
    pub fossils: Vec<data::Fossil>,
    pub bench: Vec<data::BenchOption>,
    /// The veiled placeholder mod of each side, until unveiled.
    pub veils: [ModId; 2],
    pools: Mutex<HashMap<Kind, Arc<Pool>>>,
}

impl Game {
    pub fn new(repoe: Repoe) -> Game {
        let mut game = Game {
            mods: Vec::new(),
            bases: Vec::new(),
            groups: Vec::new(),
            tags: Vec::new(),
            tag_ids: HashMap::new(),
            mod_ids: HashMap::new(),
            base_ids: HashMap::new(),
            templates: HashMap::new(),
            metamods: Vec::new(),
            essences: repoe.essences.into_values().collect(),
            fossils: repoe.fossils.into_values().collect(),
            bench: repoe.bench,
            veils: [ModId(0); 2],
            pools: Mutex::default(),
        };
        let mut group_ids: BTreeMap<String, GroupId> = BTreeMap::new();
        for (key, m) in &repoe.mods {
            let side = match m.generation_type.as_str() {
                "prefix" => Side::Prefix,
                "suffix" => Side::Suffix,
                _ => continue,
            };
            let veil = m.domain == "veiled" && VEILS.contains(&key.as_str());
            let Some(source) = (if veil { Some(Source::Veiled) } else { source_of(key, m) }) else { continue };
            let text = match &m.text {
                Some(t) => t.clone(),
                None if veil => format!("<{key}>"),
                None => continue,
            };
            let groups = m
                .groups
                .iter()
                .map(|g| {
                    let next = GroupId(group_ids.len() as u16);
                    *group_ids.entry(g.clone()).or_insert(next)
                })
                .collect();
            let def = ModDef {
                key: key.clone(),
                side,
                source,
                groups,
                tags: m.implicit_tags.iter().map(|t| game.tag(t)).collect(),
                level: m.required_level,
                template: data::template(&text),
                influence: Influence::ALL
                    .into_iter()
                    .find(|i| m.spawn_weights.iter().any(|w| w.weight > 0 && w.tag.ends_with(&format!("_{}", i.tag())))),
                spawn: m.spawn_weights.iter().map(|w| (game.tag(&w.tag), w.weight.max(0) as u32)).collect(),
                generation: m.generation_weights.iter().map(|w| (game.tag(&w.tag), w.weight.max(0) as u32)).collect(),
            };
            let id = ModId(game.mods.len() as u32);
            game.mod_ids.insert(key.clone(), id);
            game.templates.entry(def.template.to_lowercase()).or_default().push(id);
            game.mods.push(def);
        }
        game.groups = group_ids.into_keys().collect();
        game.veils = VEILS.map(|k| game.mod_id(k).expect("veiled placeholder mods"));
        game.metamods = METAMODS.iter().filter_map(|k| game.mod_id(k)).collect();
        for b in repoe.bases.values() {
            if b.release_state != "released" || b.name.is_empty() || game.base_ids.contains_key(&b.name) {
                continue;
            }
            let tags = b.tags.iter().map(|t| game.tag(t)).collect();
            game.base_ids.insert(b.name.clone(), BaseId(game.bases.len() as u32));
            game.bases.push(BaseDef { name: b.name.clone(), class: b.item_class.clone(), tags });
        }
        game
    }

    fn tag(&mut self, name: &str) -> TagId {
        if let Some(&t) = self.tag_ids.get(name) {
            return t;
        }
        let t = TagId(self.tags.len() as u16);
        self.tags.push(name.to_owned());
        self.tag_ids.insert(name.to_owned(), t);
        t
    }

    pub fn tag_id(&self, name: &str) -> Option<TagId> {
        self.tag_ids.get(name).copied()
    }

    pub fn tag_name(&self, t: TagId) -> &str {
        &self.tags[t.0 as usize]
    }

    pub fn group_name(&self, g: GroupId) -> &str {
        &self.groups[g.0 as usize]
    }

    pub fn base(&self, name: &str) -> Option<BaseId> {
        self.base_ids.get(name).copied()
    }

    pub fn mod_id(&self, key: &str) -> Option<ModId> {
        self.mod_ids.get(key).copied()
    }

    /// Every mod whose template matches, ignoring case.
    pub fn with_template(&self, template: &str) -> &[ModId] {
        self.templates.get(&template.to_lowercase()).map_or(&[], Vec::as_slice)
    }

    pub fn def(&self, m: ModId) -> &ModDef {
        &self.mods[m.0 as usize]
    }

    /// The spawn tags of an item kind: the base's, plus `<tag>_<influence>` for each influence.
    pub fn kind_tags(&self, kind: Kind) -> Vec<TagId> {
        let base = &self.bases[kind.base.0 as usize].tags;
        let mut tags = base.clone();
        for inf in kind.influences.iter() {
            tags.extend(base.iter().filter_map(|t| self.tag_id(&format!("{}_{}", self.tag_name(*t), inf.tag()))));
        }
        tags
    }

    /// Mods that roll naturally on this kind (any item level), with spawn weights.
    pub fn pool(&self, kind: Kind) -> Arc<Pool> {
        let kind = Kind { ilvl: 100, ..kind };
        if let Some(p) = self.pools.lock().unwrap().get(&kind) {
            return p.clone();
        }
        let tags = self.kind_tags(kind);
        let weight = |rules: &[(TagId, u32)], default: u32| rules.iter().find(|(t, _)| tags.contains(t)).map_or(default, |r| r.1);
        let mut mods = Vec::new();
        let mut tiers: HashMap<String, Vec<ModId>> = HashMap::new();
        for (i, m) in self.mods.iter().enumerate() {
            if !matches!(m.source, Source::Natural | Source::Influence | Source::Fossil | Source::Veiled) {
                continue;
            }
            let w = weight(&m.spawn, 0) as f64 * weight(&m.generation, 100) as f64 / 100.0;
            if w > 0.0 {
                mods.push((ModId(i as u32), w));
                if matches!(m.source, Source::Natural | Source::Influence) {
                    tiers.entry(m.template.clone()).or_default().push(ModId(i as u32));
                }
            }
        }
        for list in tiers.values_mut() {
            list.sort_by_key(|id| std::cmp::Reverse(self.def(*id).level));
        }
        let pool = Arc::new(Pool { mods, tiers });
        self.pools.lock().unwrap().insert(kind, pool.clone());
        pool
    }

    /// Tier of a mod on this kind: 1 for the highest required level of its template.
    /// Tier 1 is the highest natural level of the mod's template on this base. Other mods
    /// (essences) rank where their level falls among the natural ones.
    pub fn tier(&self, kind: Kind, m: ModId) -> Option<u8> {
        let pool = self.pool(kind);
        let list = pool.tiers.get(&self.def(m).template)?;
        let levels: std::collections::BTreeSet<u32> = list.iter().map(|id| self.def(*id).level).collect();
        Some(levels.iter().filter(|&&l| l > self.def(m).level).count() as u8 + 1)
    }
}

/// The veiled placeholder mods, prefix then suffix.
const VEILS: [&str; 2] = ["VeiledPrefix", "VeiledSuffix"];

/// Bench metamod keys, in `Game::metamods` order.
pub const METAMODS: [&str; 5] = [
    "StrMasterItemGenerationCannotChangePrefixes",
    "DexMasterItemGenerationCannotChangeSuffixes",
    "IntMasterItemGenerationCannotRollAttackAffixes",
    "StrDexMasterItemGenerationCannotRollCasterAffixes",
    "StrIntMasterItemGenerationCanHaveMultipleCraftedMods",
];

/// How a RePoE mod enters items, or `None` for mods the rules never place.
fn source_of(key: &str, m: &data::Mod) -> Option<Source> {
    Some(match m.domain.as_str() {
        "crafted" => Source::Crafted,
        "unveiled" => Source::Veiled,
        "delve" => Source::Fossil,
        "item" if m.is_essence_only => Source::Essence,
        "item" if m.spawn_weights.iter().all(|w| w.weight <= 0) => {
            if key.contains("Maven") {
                Source::Elevated
            } else {
                Source::Legacy
            }
        }
        "item" if influenced(&m.spawn_weights) => Source::Influence,
        "item" => Source::Natural,
        _ => return None,
    })
}

fn influenced(weights: &[data::Weight]) -> bool {
    weights.iter().any(|w| w.weight > 0 && Influence::ALL.iter().any(|i| w.tag.ends_with(&format!("_{}", i.tag()))))
}
