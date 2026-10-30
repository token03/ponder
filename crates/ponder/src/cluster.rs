//! Cluster jewel mod pools.
//!
//! A cluster jewel's enchant ("Added Small Passive Skills grant: 12% increased
//! Fire Damage") adds a tag such as `affliction_fire_damage`, and that tag sets
//! which notables and other mods can roll, and how often. The number of added
//! passives is fixed on the base, so crafting only changes the mods.
//!
//! A craft wants notables (by name) and/or tiered small-passive mods, written
//! "Group@tier": "HaveIncreasedEffect@1" is 35% increased Effect, and
//! "GrantAttributes@2" is +3 to All Attributes or better. "Passive point" jewels
//! (a 12-passive large with Effect and three "also grant" mods, no notables) are
//! crafts of those.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Mutex, OnceLock};

use crate::gear::PoolMod;
use anyhow::{Result, anyhow};
use indexmap::IndexMap;
use ponder_game::data;

use crate::gear::loaded;

pub const SIZES: [(&str, &str); 3] = [
    ("small", "Metadata/Items/Jewels/JewelPassiveTreeExpansionSmall"),
    ("medium", "Metadata/Items/Jewels/JewelPassiveTreeExpansionMedium"),
    ("large", "Metadata/Items/Jewels/JewelPassiveTreeExpansionLarge"),
];
/// Most notables a cluster of each size can hold (community knowledge, not in the data).
pub const NOTABLE_CAP: [(&str, usize); 3] = [("small", 1), ("medium", 2), ("large", 3)];
/// Passive counts with the fewest small passives, which buyers want.
pub const DEFAULT_PASSIVES: [(&str, i64); 3] = [("small", 2), ("medium", 4), ("large", 8)];
const ENCHANT_PREFIX: &str = "Added Small Passive Skills grant: ";
const NOTABLE_PREFIX: &str = "1 Added Passive Skill is ";
pub const GROUP_PREFIX: &str = "AfflictionJewelSmallPassives";
pub const DEFAULT_ITEM_LEVEL: i64 = 84;

fn size_id(size: &str) -> &'static str {
    SIZES.iter().find(|(s, _)| *s == size).unwrap_or_else(|| panic!("no cluster jewel size {size:?}")).1
}

pub fn notable_cap(size: &str) -> usize {
    NOTABLE_CAP.iter().find(|(s, _)| *s == size).unwrap_or_else(|| panic!("no cluster jewel size {size:?}")).1
}

pub fn default_passives(size: &str) -> i64 {
    DEFAULT_PASSIVES.iter().find(|(s, _)| *s == size).unwrap_or_else(|| panic!("no cluster jewel size {size:?}")).1
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Enchant {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub text: Vec<String>,
    /// The enchant as poe.ninja names it, e.g. "12% increased Cold Damage".
    pub short: String,
}

impl Enchant {
    pub fn new(id: &str, name: &str, tag: &str, text: Vec<String>) -> Enchant {
        let short = text.iter().map(|t| t.strip_prefix(ENCHANT_PREFIX).unwrap_or(t).to_string()).collect::<Vec<_>>().join(", ");
        Enchant { id: id.into(), name: name.into(), tag: tag.into(), text, short }
    }
}

pub fn enchants(size: &str) -> Vec<Enchant> {
    let jewels = loaded(data::cluster_jewels());
    jewels[size_id(size)].passive_skills.iter().map(|p| Enchant::new(&p.id, &p.name, &p.tag, p.stat_text.clone())).collect()
}

pub fn find_enchant(size: &str, name: &str) -> Result<Enchant> {
    let options = enchants(size);
    let wanted = name.to_lowercase();
    let mut hits: Vec<&Enchant> = options.iter().filter(|e| e.name.to_lowercase() == wanted || e.short.to_lowercase() == wanted).collect();
    if hits.is_empty() {
        hits = options.iter().filter(|e| e.tag == wanted).collect();
    }
    if hits.is_empty() {
        hits = options
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&wanted) || e.text.iter().any(|t| t.to_lowercase().contains(&wanted)))
            .collect();
    }
    if hits.len() != 1 {
        let shown = if hits.is_empty() { options.iter().collect() } else { hits.clone() };
        let names = shown.iter().map(|e| e.name.as_str()).collect::<Vec<_>>().join(", ");
        return Err(anyhow!("enchant {name:?} matches {} of: {names}", hits.len()));
    }
    Ok(hits[0].clone())
}

pub fn is_notable(m: &data::Mod) -> bool {
    let text = m.text_or_empty();
    text.starts_with(NOTABLE_PREFIX) && !text.contains("Jewel Socket")
}

pub fn notable_name(m: &data::Mod) -> String {
    m.text_or_empty().strip_prefix(NOTABLE_PREFIX).unwrap_or("").to_string()
}

/// Every mod that can roll on this cluster, with its weight.
pub fn pool(size: &str, enchant: &Enchant, item_level: i64) -> Vec<PoolMod> {
    let bases = loaded(data::base_items());
    let tags: BTreeSet<String> = bases[size_id(size)].tags.iter().cloned().chain([enchant.tag.clone()]).collect();
    let mut out = Vec::new();
    for (mod_id, m) in loaded(data::mods()).iter() {
        if m.domain != "affliction_jewel"
            || (m.generation_type != "prefix" && m.generation_type != "suffix")
            || m.required_level > item_level
        {
            continue;
        }
        let weight = ponder_game::mods::ModDB::generation_weight(m, &tags);
        if weight <= 0.0 {
            continue;
        }
        let notable = is_notable(m);
        let label = if notable { notable_name(m) } else { m.text_or_empty().replace('\n', " / ") };
        out.push(PoolMod { id: mod_id.to_string(), weight, groups: m.groups.clone(), notable, label });
    }
    out
}

/// Tier of each non-notable mod in a pool: 1 is its group's highest-level roll. Mods are
/// listed by group (in order of first appearance), each group's in pool order.
pub fn tiers(mods: &[PoolMod]) -> IndexMap<String, i64> {
    let levels = loaded(data::mods());
    let mut by_group: IndexMap<&str, Vec<&PoolMod>> = IndexMap::new();
    for m in mods {
        if !m.notable && !m.groups.is_empty() {
            by_group.entry(&m.groups[0]).or_default().push(m);
        }
    }
    let mut out = IndexMap::new();
    for group in by_group.values() {
        let mut ranks: Vec<i64> = group.iter().map(|m| levels[m.id.as_str()].required_level).collect();
        ranks.sort_unstable_by(|a, b| b.cmp(a));
        ranks.dedup();
        for m in group {
            let level = levels[m.id.as_str()].required_level;
            out.insert(m.id.clone(), ranks.iter().position(|&r| r == level).unwrap() as i64 + 1);
        }
    }
    out
}

/// [`tiers`] of the whole pool of a size and enchant tag (the pool depends only on the tag).
pub fn mod_tiers(size: &str, tag: &str) -> IndexMap<String, i64> {
    type Cache = Mutex<HashMap<(String, String), IndexMap<String, i64>>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    let key = (size.to_string(), tag.to_string());
    if let Some(t) = cache.lock().unwrap().get(&key) {
        return t.clone();
    }
    let made = tiers(&pool(size, &Enchant::new(tag, tag, tag, Vec::new()), DEFAULT_ITEM_LEVEL));
    cache.lock().unwrap().entry(key).or_insert(made).clone()
}

/// A tiered small-passive mod as a craft names it, e.g. "HaveIncreasedEffect@1".
pub fn feature(mod_id: &str, tier: i64) -> String {
    let group = &loaded(data::mods())[mod_id].groups[0];
    format!("{}@{tier}", group.strip_prefix(GROUP_PREFIX).unwrap_or(group))
}

pub fn is_feature(name: &str) -> bool {
    name.contains('@')
}

fn parse(name: &str) -> Result<(String, i64)> {
    let (group, tier) = name.rsplit_once('@').ok_or_else(|| anyhow!("{name:?} is not a Group@tier"))?;
    Ok((format!("{GROUP_PREFIX}{group}"), tier.trim().parse().map_err(|_| anyhow!("invalid literal for int() with base 10: {tier:?}"))?))
}

/// "HaveIncreasedEffect@1" -> "35% increased Effect"; tiers past 1 read "(T2+)".
pub fn feature_label(name: &str, mods: &[PoolMod]) -> String {
    if !is_feature(name) {
        return name.to_string();
    }
    let Ok((group, tier)) = parse(name) else {
        return name.to_string();
    };
    let texts = loaded(data::mods());
    let hits: Vec<PoolMod> = mods.iter().filter(|m| m.groups.first() == Some(&group)).cloned().collect();
    let ranked = if hits.is_empty() { IndexMap::new() } else { tiers(&hits) };
    let mod_id = ranked
        .iter()
        .find(|(_, t)| **t == tier)
        .map(|(m, _)| m.as_str())
        .or_else(|| texts.iter().find(|(_, m)| m.groups.contains(&group)).map(|(id, _)| id.as_str()));
    let text = match mod_id {
        Some(id) if !texts[id].text_or_empty().is_empty() => texts[id].text_or_empty(),
        _ => &group,
    };
    let text =
        text.replace("Added Small Passive Skills also grant: ", "").replace("Added Small Passive Skills have ", "").replace('\n', " / ");
    if tier > 1 { format!("{text} (T{tier}+)") } else { text }
}

/// Targets for notable names and "Group@tier" small-passive mods.
pub fn problem(
    size: &str,
    enchant: &str,
    names: &[String],
    item_level: i64,
    fractured: &[String],
    filler: &[String],
) -> Result<crate::candidate::Candidate> {
    use crate::candidate::Need;
    let enchant = find_enchant(size, enchant)?;
    let mods = pool(size, &enchant, item_level);
    let ranks = tiers(&mods);
    let resolve = |name: &str| -> Result<BTreeSet<String>> {
        let matches: BTreeSet<_> = if is_feature(name) {
            let (group, tier) = parse(name)?;
            mods.iter()
                .filter(|m| m.groups.contains(&group) && ranks.get(&m.id).is_some_and(|rank| *rank <= tier))
                .map(|m| m.id.clone())
                .collect()
        } else {
            mods.iter().filter(|m| m.notable && m.label.eq_ignore_ascii_case(name)).map(|m| m.id.clone()).collect()
        };
        if matches.is_empty() {
            return Err(anyhow!("{name:?} cannot roll on this cluster"));
        }
        Ok(matches)
    };
    let mut needs = names
        .iter()
        .map(|name| Ok(Need { name: name.clone(), mods: resolve(name)?, count: 1, fractured: false }))
        .collect::<Result<Vec<_>>>()?;
    let fixed = fractured
        .iter()
        .map(|name| {
            let mut acceptable = resolve(name)?;
            if is_feature(name) {
                let (_, tier) = parse(name)?;
                acceptable.retain(|id| ranks.get(id) == Some(&tier));
            }
            acceptable.into_iter().next().ok_or_else(|| anyhow!("missing fractured mod"))
        })
        .collect::<Result<Vec<_>>>()?;
    for id in &fixed {
        if let Some(need) = needs.iter_mut().find(|need| need.mods.contains(id)) {
            need.fractured = true;
        } else {
            needs.push(Need { name: id.clone(), mods: BTreeSet::from([id.clone()]), count: 1, fractured: true });
        }
    }
    if !filler.is_empty() {
        let mut acceptable = BTreeSet::new();
        for name in filler {
            acceptable.extend(resolve(name)?);
        }
        needs.push(Need { name: format!("any of: {}", filler.join(", ")), mods: acceptable, count: 1, fractured: false });
    }
    Ok(crate::candidate::Candidate {
        base: format!("{} Cluster Jewel [{}]", size[..1].to_uppercase() + &size[1..], enchant.tag),
        item_level: u8::try_from(item_level)?,
        influences: BTreeSet::new(),
        needs,
        fixed,
        min_mods: if filler.is_empty() { 0 } else { 4 },
    })
}
