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

use anyhow::{Result, anyhow};
use indexmap::IndexMap;
use ponder_core::{Kind, Mod, Plan, Problem, Side, Target, solve};
use ponder_game::data;

use crate::actions::{self, loaded};
use crate::gear::Pricing;

pub const SIZES: [(&str, &str); 3] = [
    (
        "small",
        "Metadata/Items/Jewels/JewelPassiveTreeExpansionSmall",
    ),
    (
        "medium",
        "Metadata/Items/Jewels/JewelPassiveTreeExpansionMedium",
    ),
    (
        "large",
        "Metadata/Items/Jewels/JewelPassiveTreeExpansionLarge",
    ),
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
    SIZES
        .iter()
        .find(|(s, _)| *s == size)
        .unwrap_or_else(|| panic!("no cluster jewel size {size:?}"))
        .1
}

pub fn notable_cap(size: &str) -> usize {
    NOTABLE_CAP
        .iter()
        .find(|(s, _)| *s == size)
        .unwrap_or_else(|| panic!("no cluster jewel size {size:?}"))
        .1
}

pub fn default_passives(size: &str) -> i64 {
    DEFAULT_PASSIVES
        .iter()
        .find(|(s, _)| *s == size)
        .unwrap_or_else(|| panic!("no cluster jewel size {size:?}"))
        .1
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
        let short = text
            .iter()
            .map(|t| t.strip_prefix(ENCHANT_PREFIX).unwrap_or(t).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        Enchant {
            id: id.into(),
            name: name.into(),
            tag: tag.into(),
            text,
            short,
        }
    }
}

pub fn enchants(size: &str) -> Vec<Enchant> {
    let jewels = loaded(data::cluster_jewels());
    jewels[size_id(size)]
        .passive_skills
        .iter()
        .map(|p| Enchant::new(&p.id, &p.name, &p.tag, p.stat_text.clone()))
        .collect()
}

pub fn find_enchant(size: &str, name: &str) -> Result<Enchant> {
    let options = enchants(size);
    let wanted = name.to_lowercase();
    let mut hits: Vec<&Enchant> = options
        .iter()
        .filter(|e| e.name.to_lowercase() == wanted || e.short.to_lowercase() == wanted)
        .collect();
    if hits.is_empty() {
        hits = options.iter().filter(|e| e.tag == wanted).collect();
    }
    if hits.is_empty() {
        hits = options
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&wanted)
                    || e.text.iter().any(|t| t.to_lowercase().contains(&wanted))
            })
            .collect();
    }
    if hits.len() != 1 {
        let shown = if hits.is_empty() {
            options.iter().collect()
        } else {
            hits.clone()
        };
        let names = shown
            .iter()
            .map(|e| e.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        return Err(anyhow!(
            "enchant {name:?} matches {} of: {names}",
            hits.len()
        ));
    }
    Ok(hits[0].clone())
}

pub fn is_notable(m: &data::Mod) -> bool {
    let text = m.text_or_empty();
    text.starts_with(NOTABLE_PREFIX) && !text.contains("Jewel Socket")
}

pub fn notable_name(m: &data::Mod) -> String {
    m.text_or_empty()
        .strip_prefix(NOTABLE_PREFIX)
        .unwrap_or("")
        .to_string()
}

/// Every mod that can roll on this cluster, with its weight.
pub fn pool(size: &str, enchant: &Enchant, item_level: i64) -> Vec<Mod> {
    let bases = loaded(data::base_items());
    let tags: BTreeSet<String> = bases[size_id(size)]
        .tags
        .iter()
        .cloned()
        .chain([enchant.tag.clone()])
        .collect();
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
        let label = if notable {
            notable_name(m)
        } else {
            m.text_or_empty().replace('\n', " / ")
        };
        out.push(Mod {
            id: mod_id.to_string(),
            side: if m.generation_type == "prefix" {
                Side::Prefix
            } else {
                Side::Suffix
            },
            weight,
            groups: m.groups.clone(),
            notable,
            label,
            tags: m.implicit_tags.clone(),
        });
    }
    out
}

/// Tier of each non-notable mod in a pool: 1 is its group's highest-level roll. Mods are
/// listed by group (in order of first appearance), each group's in pool order.
pub fn tiers(mods: &[Mod]) -> IndexMap<String, i64> {
    let levels = loaded(data::mods());
    let mut by_group: IndexMap<&str, Vec<&Mod>> = IndexMap::new();
    for m in mods {
        if !m.notable && !m.groups.is_empty() {
            by_group.entry(&m.groups[0]).or_default().push(m);
        }
    }
    let mut out = IndexMap::new();
    for group in by_group.values() {
        let mut ranks: Vec<i64> = group
            .iter()
            .map(|m| levels[m.id.as_str()].required_level)
            .collect();
        ranks.sort_unstable_by(|a, b| b.cmp(a));
        ranks.dedup();
        for m in group {
            let level = levels[m.id.as_str()].required_level;
            out.insert(
                m.id.clone(),
                ranks.iter().position(|&r| r == level).unwrap() as i64 + 1,
            );
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
    let made = tiers(&pool(
        size,
        &Enchant::new(tag, tag, tag, Vec::new()),
        DEFAULT_ITEM_LEVEL,
    ));
    cache.lock().unwrap().entry(key).or_insert(made).clone()
}

/// A tiered small-passive mod as a craft names it, e.g. "HaveIncreasedEffect@1".
pub fn feature(mod_id: &str, tier: i64) -> String {
    let group = &loaded(data::mods())[mod_id].groups[0];
    format!(
        "{}@{tier}",
        group.strip_prefix(GROUP_PREFIX).unwrap_or(group)
    )
}

pub fn is_feature(name: &str) -> bool {
    name.contains('@')
}

fn parse(name: &str) -> Result<(String, i64)> {
    let (group, tier) = name
        .rsplit_once('@')
        .ok_or_else(|| anyhow!("{name:?} is not a Group@tier"))?;
    Ok((
        format!("{GROUP_PREFIX}{group}"),
        tier.trim()
            .parse()
            .map_err(|_| anyhow!("invalid literal for int() with base 10: {tier:?}"))?,
    ))
}

/// "HaveIncreasedEffect@1" -> "35% increased Effect"; tiers past 1 read "(T2+)".
pub fn feature_label(name: &str, mods: &[Mod]) -> String {
    if !is_feature(name) {
        return name.to_string();
    }
    let Ok((group, tier)) = parse(name) else {
        return name.to_string();
    };
    let texts = loaded(data::mods());
    let hits: Vec<Mod> = mods
        .iter()
        .filter(|m| m.groups.first() == Some(&group))
        .cloned()
        .collect();
    let ranked = if hits.is_empty() {
        IndexMap::new()
    } else {
        tiers(&hits)
    };
    let mod_id = ranked
        .iter()
        .find(|(_, t)| **t == tier)
        .map(|(m, _)| m.as_str())
        .or_else(|| {
            texts
                .iter()
                .find(|(_, m)| m.groups.contains(&group))
                .map(|(id, _)| id.as_str())
        });
    let text = match mod_id {
        Some(id) if !texts[id].text_or_empty().is_empty() => texts[id].text_or_empty(),
        _ => &group,
    };
    let text = text
        .replace("Added Small Passive Skills also grant: ", "")
        .replace("Added Small Passive Skills have ", "")
        .replace('\n', " / ");
    if tier > 1 {
        format!("{text} (T{tier}+)")
    } else {
        text
    }
}

/// Targets for notable names and "Group@tier" small-passive mods.
pub fn resolve(mods: &[Mod], names: &[String]) -> Result<Vec<Target>> {
    let ranked = tiers(mods);
    let mut out = Vec::new();
    for name in names {
        if !is_feature(name) {
            let m = resolve_notables(mods, std::slice::from_ref(name))?.remove(0);
            out.push(Target {
                name: m.label.clone(),
                side: m.side,
                good: BTreeSet::from([m.id.clone()]),
                groups: m.groups.iter().cloned().collect(),
                notable: true,
            });
            continue;
        }
        let (group, tier) = parse(name)?;
        let good: Vec<&Mod> = mods
            .iter()
            .filter(|m| {
                m.groups.first() == Some(&group) && ranked.get(&m.id).copied().unwrap_or(99) <= tier
            })
            .collect();
        if good.is_empty() {
            return Err(anyhow!("{name:?} can't roll here"));
        }
        out.push(Target {
            name: feature_label(name, mods),
            side: good[0].side,
            good: good.iter().map(|m| m.id.clone()).collect(),
            groups: good.iter().flat_map(|m| m.groups.iter().cloned()).collect(),
            notable: false,
        });
    }
    Ok(out)
}

/// Fill a passive jewel: one open slot takes any of `features` ("Group@tier" small-passive mods,
/// what its wearers put there), the others any mod at all, so the jewel has all four. Features
/// that can't roll here are skipped; with nothing to fill a slot, the jewel is only made full.
pub fn add_filler(problem: &Problem, features: &[String]) -> Problem {
    if features.is_empty() {
        return problem.clone();
    }
    let mut p = problem.to_spec();
    let wanted: BTreeSet<&String> = p.targets.iter().flat_map(|t| t.good.iter()).collect();
    let mut filler = BTreeSet::new();
    for f in features {
        if let Ok(t) = resolve(&p.pool, std::slice::from_ref(f)) {
            filler.extend(t[0].good.iter().filter(|m| !wanted.contains(m)).cloned());
        }
    }
    let limit = problem.limit();
    let open = |side: Side| {
        let used = p.targets.iter().filter(|t| t.side == side).count() + p.fixed.iter().filter(|m| m.side == side).count();
        let fillable = p.pool.iter().any(|m| m.side == side && filler.contains(&m.id));
        if fillable { limit.saturating_sub(used) } else { 0 }
    };
    let (pre, suf) = (open(Side::Prefix), open(Side::Suffix));
    p.min_mods = 2 * limit;
    if pre + suf == 0 {
        return p.compile();
    }
    p.filler = filler;
    p.filler_sides = Some(if suf >= pre { (0, 1) } else { (1, 0) });
    p.filler_need = 1;
    p.compile()
}

/// Fractured mods: notables by name, small-passive mods at exactly their tier.
pub fn resolve_fixed(mods: &[Mod], names: &[String]) -> Result<Vec<Mod>> {
    let ranked = tiers(mods);
    let mut out = Vec::new();
    for name in names {
        if !is_feature(name) {
            out.extend(resolve_notables(mods, std::slice::from_ref(name))?);
            continue;
        }
        let (group, tier) = parse(name)?;
        let hit = mods
            .iter()
            .find(|m| m.groups.first() == Some(&group) && ranked.get(&m.id) == Some(&tier));
        out.push(
            hit.ok_or_else(|| anyhow!("{name:?} can't roll here"))?
                .clone(),
        );
    }
    Ok(out)
}

pub fn resolve_notables(mods: &[Mod], names: &[String]) -> Result<Vec<Mod>> {
    let mut by_name: HashMap<String, &Mod> = HashMap::new();
    for m in mods.iter().filter(|m| m.notable) {
        by_name.insert(m.label.to_lowercase(), m);
    }
    names
        .iter()
        .map(|name| {
            by_name
                .get(name.trim().to_lowercase().as_str())
                .map(|m| (*m).clone())
                .ok_or_else(|| {
                    let mut options: Vec<&str> = by_name.keys().map(String::as_str).collect();
                    options.sort_unstable();
                    anyhow!(
                        "notable {name:?} can't roll here; options: {}",
                        options.join(", ")
                    )
                })
        })
        .collect()
}

/// Cheapest plan for `need` of `notables`, over basic currency and Harvest reforges.
///
/// `fractured` notables are already on the base and survive every reroll.
pub fn cost(
    size: &str,
    enchant: &str,
    notables: &[String],
    pricing: Pricing,
    need: Option<usize>,
    item_level: i64,
    fractured: &[String],
) -> Result<(Problem, Plan)> {
    let p = problem(size, enchant, notables, need, item_level, fractured)?;
    let plan = solve(&p, pricing.prices, pricing.settings)?;
    Ok((p, plan))
}

/// The crafting problem `cost` solves, for solving elsewhere.
pub fn problem(
    size: &str,
    enchant: &str,
    notables: &[String],
    need: Option<usize>,
    item_level: i64,
    fractured: &[String],
) -> Result<Problem> {
    let mods = pool(size, &find_enchant(size, enchant)?, item_level);
    let targets = resolve(&mods, notables)?;
    let fixed = resolve_fixed(&mods, fractured)?;
    let need = need.filter(|&n| n != 0).unwrap_or(targets.len());
    let cap = notable_cap(size);
    if targets.iter().filter(|t| t.notable).count() > cap {
        return Err(anyhow!("a {size} cluster holds at most {cap} notables"));
    }
    let (harvest, profiles) = actions::harvest(&mods);
    let mut all = actions::basic();
    all.extend(harvest);
    let mut p = ponder_core::ProblemSpec::new(mods, targets, need, all, Kind::Jewel);
    p.notable_cap = cap;
    p.fixed = fixed;
    p.profiles = profiles;
    Ok(p.compile())
}
