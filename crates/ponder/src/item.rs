use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use ponder_game::{
    data,
    item::{Item as Text, ModLine, split_lines},
    mods::{IdMods, ItemMatch, ModDB},
};
use ponder_net::ladder::Lite;
pub use ponder_net::ladder::RareItem as Raw;
use ponder_net::trade::ListingItem;
use rayon::prelude::*;

pub fn listing_to_item(l: &ListingItem) -> Text {
    let mut item = Text::new(l.rarity.as_str(), l.name.as_str(), l.base.as_str());
    item.item_level = l.ilvl;
    item.influences = l.influences.clone();
    item.flags = l.flags.iter().cloned().collect();
    item.implicits = l.implicits.iter().map(ModLine::new).collect();
    item.explicits = l
        .explicits
        .iter()
        .map(|e| {
            if e.fractured {
                ModLine::with_marker(&e.text, "fractured")
            } else if e.crafted {
                ModLine::with_marker(&e.text, "crafted")
            } else {
                ModLine::new(&e.text)
            }
        })
        .collect();
    item
}

#[derive(Debug, Clone)]
pub struct Item {
    /// account/name
    pub character: String,
    pub skill: String,
    pub match_: Arc<ItemMatch>,
    pub group: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
    pub character: String,
    pub skill: String,
    /// small / medium / large
    pub size: String,
    /// e.g. "12% increased Fire Damage"
    pub enchant: String,
    /// the enchant's spawn tag, e.g. affliction_fire_damage
    pub tag: String,
    pub passives: i64,
    pub item_level: i64,
    /// notable names, sorted
    pub notables: Vec<String>,
    /// fractured notables and small-passive mods ("Group@tier") among them
    pub fractured: Vec<String>,
    /// small-passive mods as "Group@tier", sorted
    pub mods: Vec<String>,
}

#[derive(Default)]
pub struct Items {
    pub gear: Vec<Item>,
    pub clusters: Vec<Cluster>,
}

pub fn normalize(db: &ModDB, characters: &[Lite]) -> Items {
    let context = ClusterCtx::new();
    let batches: Vec<Items> = characters
        .par_iter()
        .map(|character| {
            let who = format!("{}/{}", character.account.as_deref().unwrap_or("None"), character.name.as_deref().unwrap_or("None"));
            let skill = if character.skill.is_empty() { "?" } else { &character.skill };
            let mut items = Items::default();
            for raw in &character.rares {
                if raw.base_type().is_some_and(|base| base.ends_with("Cluster Jewel")) {
                    if let Some(cluster) = context.worn(&who, skill, raw) {
                        items.clusters.push(cluster);
                    }
                    continue;
                }
                let matched = match_rare(db, raw);
                if matched.base_id.is_some() && !matched.mods.is_empty() {
                    items.gear.push(Item {
                        character: who.clone(),
                        skill: skill.to_owned(),
                        group: crate::gear::group(db, &matched).label.clone(),
                        match_: Arc::new(matched),
                    });
                }
            }
            items
        })
        .collect();
    let mut items = Items::default();
    for batch in batches {
        items.gear.extend(batch.gear);
        items.clusters.extend(batch.clusters);
    }
    items
}

fn match_rare(db: &ModDB, it: &Raw) -> ItemMatch {
    let influences = ponder_net::trade::influences(it.influences.as_ref().and_then(|v| v.as_ref()));
    let ids = |kind: &str| -> Vec<String> { it.mods_of(kind).iter().map(|e| e.id().unwrap_or("").to_string()).collect() };
    let mods = IdMods { explicit: ids("explicit"), fractured: ids("fractured"), crafted: ids("crafted") };
    let ilvl = it.ilvl.flatten();
    db.match_ids(it.base_type().unwrap_or(""), &mods, ilvl, &influences, "RARE", "", &BTreeMap::new())
}

const ENCHANT_PREFIX: &str = "Added Small Passive Skills grant: ";

type TierTables = HashMap<(String, String), Arc<HashMap<String, i64>>>;

struct ClusterCtx {
    mods: Arc<data::Mods>,
    by_text: HashMap<&'static str, HashMap<BTreeSet<String>, (String, String)>>,
    tiers: Mutex<TierTables>,
}

impl ClusterCtx {
    fn new() -> ClusterCtx {
        ClusterCtx {
            mods: data::mods().unwrap_or_else(|e| panic!("{e}")),
            by_text: ["small", "medium", "large"]
                .into_iter()
                .map(|size| (size, crate::cluster::enchants(size).into_iter().map(|e| (e.short, e.tag)).collect::<Vec<_>>()))
                .map(|(size, list)| {
                    (size, list.into_iter().map(|(short, tag)| (short.split(", ").map(str::to_string).collect(), (short, tag))).collect())
                })
                .collect(),
            tiers: Mutex::new(HashMap::new()),
        }
    }

    fn tiers(&self, size: &str, tag: &str) -> Arc<HashMap<String, i64>> {
        let key = (size.to_string(), tag.to_string());
        if let Some(t) = self.tiers.lock().unwrap().get(&key) {
            return t.clone();
        }
        let t = Arc::new(crate::cluster::mod_tiers(size, tag).into_iter().collect());
        self.tiers.lock().unwrap().entry(key).or_insert(t).clone()
    }

    fn worn(&self, who: &str, skill: &str, it: &Raw) -> Option<Cluster> {
        let base = it.base_type().unwrap_or("");
        if !base.ends_with("Cluster Jewel") {
            return None;
        }
        let size = base.split_whitespace().next().unwrap_or("").to_lowercase();
        let lines: BTreeSet<String> = it
            .enchant_mods
            .iter()
            .flatten()
            .flatten()
            .flat_map(|text| split_lines(text))
            .filter_map(|ln| ln.strip_prefix(ENCHANT_PREFIX)) // line order varies
            .map(str::to_string)
            .collect();
        let (short, tag) = self.by_text.get(size.as_str())?.get(&lines)?;
        let passives = it
            .mods_of("enchant")
            .iter()
            .find_map(|m| m.0.get("stats").map(|s| s.get("local_jewel_expansion_passive_node_count").cloned()))
            .flatten()
            .filter(|v| v.as_f64().is_some_and(|n| n != 0.0))?;
        let passives = passives.as_i64().or_else(|| passives.as_f64().map(|f| f as i64))?;
        let (mut notables, mut fractured, mut small) = (Vec::new(), Vec::new(), Vec::new());
        let tiers = self.tiers(&size, tag);
        for source in ["explicit", "fractured"] {
            for m in it.mods_of(source) {
                let mod_ = m.id().and_then(|id| self.mods.get(id));
                let is_notable = mod_.is_some_and(crate::cluster::is_notable);
                let added = match (mod_, m.id()) {
                    (Some(md), _) if is_notable => {
                        notables.push(crate::cluster::notable_name(md));
                        notables.last().cloned()
                    }
                    (Some(_), Some(id)) if tiers.contains_key(id) => {
                        small.push(crate::cluster::feature(id, tiers[id]));
                        small.last().cloned()
                    }
                    _ => None,
                };
                if source == "fractured" {
                    fractured.extend(added);
                }
            }
        }
        if (notables.is_empty() && small.is_empty()) || passives == 0 {
            return None;
        }
        notables.sort();
        fractured.sort();
        small.sort();
        Some(Cluster {
            character: who.to_string(),
            skill: skill.to_string(),
            size,
            enchant: short.clone(),
            tag: tag.clone(),
            passives,
            item_level: it.ilvl.flatten().unwrap_or(0),
            notables,
            fractured,
            mods: small,
        })
    }
}
