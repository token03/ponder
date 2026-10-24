//! Match item text lines back to exact mod ids, tiers and sources.
//!
//! Every repoe mod carries a display template such as `+(100-114) to maximum
//! Life`. Replacing numbers and ranges with `#` gives a key we can look item lines
//! up by; the ranges then pick out the tier. Hybrid mods span several lines and
//! are matched as a unit before single-line mods.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, LazyLock, OnceLock, RwLock};

use regex::Regex;

use crate::data::{self, BaseItems, DataError, Mod, Mods};
use crate::item::{Item, ModLine, py_splitlines};

static TOKEN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\((-?[0-9]+(?:\.[0-9]+)?)-(-?[0-9]+(?:\.[0-9]+)?)\)|([0-9]+(?:\.[0-9]+)?)")
        .unwrap()
});

/// repoe spawn tags use the old internal influence names.
pub const INFLUENCE_TAGS: &[(&str, &str)] = &[
    ("shaper", "shaper"),
    ("elder", "elder"),
    ("crusader", "crusader"),
    ("redeemer", "eyrie"),
    ("hunter", "basilisk"),
    ("warlord", "adjudicator"),
];

/// Base domains we match items for; jewels and flasks keep their mods in their own domain.
pub const BASE_DOMAINS: &[&str] = &["item", "misc", "abyss_jewel", "affliction_jewel", "flask"];
/// Mod domains that apply to ordinary (`item` domain) bases, and the source they imply.
pub const ITEM_EXTRA_DOMAINS: &[(&str, &str)] = &[
    ("crafted", "crafted"),
    ("unveiled", "veiled"),
    ("delve", "delve"),
];

fn extra_domain(domain: &str) -> Option<&'static str> {
    ITEM_EXTRA_DOMAINS
        .iter()
        .find(|(d, _)| *d == domain)
        .map(|(_, s)| *s)
}

/// Lower sorts first when several mods fit a line.
fn source_rank(source: &str) -> i32 {
    match source {
        "natural" | "crafted" => 0,
        "essence" | "veiled" | "delve" | "legacy-crafted" => 1,
        "special" => 2,
        other => panic!("unknown mod source {other:?}"),
    }
}

/// Item class -> mod ids the crafting bench can currently add.
pub type Bench = HashMap<String, HashSet<String>>;

/// One mod found on an item.
#[derive(Debug, Clone, PartialEq)]
pub struct ModMatch {
    pub mod_id: String,
    /// "prefix" or "suffix"
    pub kind: String,
    /// repoe mod `type`, shared by every tier of a mod
    pub family: String,
    /// natural | essence | veiled | delve | special | crafted | legacy-crafted
    pub source: &'static str,
    /// 1 = best natural tier on this base
    pub tier: Option<i64>,
    pub lines: Vec<ModLine>,
    /// Where `lines` sit in `item.explicits` (empty for `match_ids`, which has no text).
    pub line_ids: Vec<usize>,
    /// other mods that fit the same lines equally well
    pub alternatives: usize,
    /// shares a summed stat line with another mod
    pub merged: bool,
}

impl ModMatch {
    pub fn fractured(&self) -> bool {
        self.lines.iter().any(ModLine::fractured)
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ItemMatch {
    pub item: Item,
    pub base_id: Option<String>,
    pub item_class: Option<String>,
    pub tags: BTreeSet<String>,
    pub mods: Vec<ModMatch>,
    /// look like mods, but no mod fits
    pub unmatched: Vec<ModLine>,
    /// text no mod has, e.g. author notes
    pub notes: Vec<ModLine>,
    /// What else a finished item needs (ladder archetypes): `{"families": {family: worst tier},
    /// "sides": [prefix, suffix] filler count, "order": families commonest first}`; see gear.build.
    pub finish: Option<serde_json::Value>,
}

impl ItemMatch {
    pub fn prefixes(&self) -> impl Iterator<Item = &ModMatch> {
        self.mods.iter().filter(|m| m.kind == "prefix")
    }
    pub fn suffixes(&self) -> impl Iterator<Item = &ModMatch> {
        self.mods.iter().filter(|m| m.kind == "suffix")
    }
}

/// Numbers and ranges replaced by `#`, lowercased: the key lines are looked up by.
pub fn normalize(text: &str) -> String {
    TOKEN.replace_all(text, "#").to_lowercase()
}

/// The numbers of a template as (low, high); a plain number is its own range.
pub fn template_ranges(text: &str) -> Vec<(f64, f64)> {
    TOKEN
        .captures_iter(text)
        .map(|m| match m.get(3) {
            Some(n) => {
                let v: f64 = n.as_str().parse().unwrap();
                (v, v)
            }
            None => {
                let lo: f64 = m[1].parse().unwrap();
                let hi: f64 = m[2].parse().unwrap();
                (lo.min(hi), lo.max(hi))
            }
        })
        .collect()
}

/// Numbers on an item line; ranges are resolved with PoB's roll position.
pub fn line_values(line: &ModLine) -> Vec<f64> {
    let roll = line.roll().unwrap_or(0.5);
    template_ranges(&line.text)
        .into_iter()
        .map(|(lo, hi)| lo + (hi - lo) * roll)
        .collect()
}

/// Do all `values` sit in their `ranges` (with half a point of rounding slack)?
pub fn fits(values: &[f64], ranges: &[(f64, f64)]) -> bool {
    values.len() == ranges.len()
        && values
            .iter()
            .zip(ranges)
            .all(|(v, (lo, hi))| lo - 0.5 <= *v && *v <= hi + 0.5)
}

/// What `match_ids` takes: mod ids by how the character wears them
/// (poe.ninja's `mods`: "explicit", "fractured", "crafted", the rest ignored).
#[derive(Debug, Clone, Default)]
pub struct IdMods {
    pub explicit: Vec<String>,
    pub fractured: Vec<String>,
    pub crafted: Vec<String>,
}

impl IdMods {
    /// From poe.ninja's `{"explicit": [{"id": ...}, ...], ...}`; entries without an id are kept as "" (never a mod).
    pub fn from_json(mods: &serde_json::Value) -> IdMods {
        let ids = |kind: &str| -> Vec<String> {
            mods.get(kind)
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .map(|e| {
                            e.get("id")
                                .and_then(|i| i.as_str())
                                .unwrap_or("")
                                .to_string()
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        IdMods {
            explicit: ids("explicit"),
            fractured: ids("fractured"),
            crafted: ids("crafted"),
        }
    }
}

struct Ctx<'a> {
    item: &'a Item,
    base_domain: &'a str,
    item_class: &'a str,
    tags: &'a BTreeSet<String>,
    pob_ids: HashSet<&'a str>,
}

/// A mod's template split into lines, each with its number ranges; parsed on first use.
struct Template {
    lines: Vec<String>,
    ranges: Vec<Vec<(f64, f64)>>,
}

/// Normalized template lines -> mods, in order of first appearance (ties break by it).
struct Index {
    keys: Vec<(Vec<String>, Vec<usize>)>,
    pos: HashMap<Vec<String>, usize>,
    /// a normalized line -> the hybrid keys (several lines) containing it
    hybrids_by_line: HashMap<String, Vec<usize>>,
}

/// tag set -> tier key -> required levels of the family, best first
type TierCache = HashMap<BTreeSet<String>, HashMap<u32, Arc<Vec<i64>>>>;

pub struct ModDB {
    pub mods: Arc<Mods>,
    pub bases: Arc<BaseItems>,
    bench: Bench,
    /// per mod position: is it a prefix/suffix we can match?
    is_affix: Vec<bool>,
    templates: Vec<OnceLock<Template>>,
    index: Index,
    /// mod position -> small id of its (type, generation_type): the tier cache key
    tier_key: Vec<u32>,
    /// (domain, type, generation_type) -> positions of its affixes
    families: HashMap<(String, String, String), Vec<usize>>,
    bases_by_name: data::OrderedMap<String>,
    tiers: RwLock<TierCache>,
}

impl ModDB {
    /// The default database: repoe mods and bases from the data directory, and the current bench.
    pub fn load() -> Result<ModDB, DataError> {
        Ok(ModDB::new(
            Some(data::mods()?),
            Some(data::base_items()?),
            None,
        ))
    }

    /// A database over the given data; `None` loads the data directory's.
    ///
    /// Panics if a `None` file cannot be loaded; use [`ModDB::load`] to get the error.
    pub fn new(
        mods: Option<Arc<Mods>>,
        bases: Option<Arc<BaseItems>>,
        bench: Option<Bench>,
    ) -> ModDB {
        let mods = mods.unwrap_or_else(|| data::mods().unwrap_or_else(|e| panic!("{e}")));
        let bases = bases.unwrap_or_else(|| data::base_items().unwrap_or_else(|e| panic!("{e}")));
        let bench = bench.unwrap_or_else(|| load_bench().unwrap_or_else(|e| panic!("{e}")));

        let is_affix: Vec<bool> = mods
            .values()
            .map(|m| {
                (BASE_DOMAINS.contains(&m.domain.as_str()) || extra_domain(&m.domain).is_some())
                    && (m.generation_type == "prefix" || m.generation_type == "suffix")
                    && !m.text_or_empty().is_empty()
            })
            .collect();

        let mut index = Index {
            keys: Vec::new(),
            pos: HashMap::new(),
            hybrids_by_line: HashMap::new(),
        };
        for (i, (_, m)) in mods.iter().enumerate() {
            if !is_affix[i] {
                continue;
            }
            let key: Vec<String> = py_splitlines(m.text_or_empty())
                .into_iter()
                .map(normalize)
                .collect();
            match index.pos.get(&key) {
                Some(&k) => index.keys[k].1.push(i),
                None => {
                    index.pos.insert(key.clone(), index.keys.len());
                    index.keys.push((key, vec![i]));
                }
            }
        }
        for (k, (key, _)) in index.keys.iter().enumerate() {
            if key.len() > 1 {
                for line in key {
                    index
                        .hybrids_by_line
                        .entry(line.clone())
                        .or_default()
                        .push(k);
                }
            }
        }

        let mut families: HashMap<(String, String, String), Vec<usize>> = HashMap::new();
        let mut tier_ids: HashMap<(&str, &str), u32> = HashMap::new();
        let mut tier_key = Vec::with_capacity(mods.len());
        for (i, (_, m)) in mods.iter().enumerate() {
            let next = tier_ids.len() as u32;
            tier_key.push(
                *tier_ids
                    .entry((m.type_.as_str(), m.generation_type.as_str()))
                    .or_insert(next),
            );
            if is_affix[i] {
                families
                    .entry((m.domain.clone(), m.type_.clone(), m.generation_type.clone()))
                    .or_default()
                    .push(i);
            }
        }

        let mut bases_by_name: data::OrderedMap<String> = data::OrderedMap::default();
        for (base_id, base) in bases.iter() {
            if !BASE_DOMAINS.contains(&base.domain.as_str()) || base.name.is_empty() {
                continue;
            }
            if !bases_by_name.contains_key(&base.name) || base.release_state == "released" {
                bases_by_name.insert(base.name.clone(), base_id.to_string());
            }
        }

        let templates = (0..mods.len()).map(|_| OnceLock::new()).collect();
        ModDB {
            mods,
            bases,
            bench,
            is_affix,
            templates,
            index,
            tier_key,
            families,
            bases_by_name,
            tiers: RwLock::new(HashMap::new()),
        }
    }

    /// The mod `mod_id` if it is one we match (a prefix or suffix with text).
    pub fn affix(&self, mod_id: &str) -> Option<&Mod> {
        let i = self.mods.position(mod_id)?;
        self.is_affix[i].then(|| self.mods.entry(i).1)
    }

    fn template(&self, i: usize) -> &Template {
        self.templates[i].get_or_init(|| {
            let lines: Vec<String> = py_splitlines(self.mods.entry(i).1.text_or_empty())
                .into_iter()
                .map(String::from)
                .collect();
            let ranges = lines.iter().map(|l| template_ranges(l)).collect();
            Template { lines, ranges }
        })
    }

    /// The base item id for a base name; magic and normal items carry affixes around it.
    pub fn find_base(&self, name: &str) -> Option<&str> {
        if let Some(id) = self.bases_by_name.get(name) {
            return Some(id);
        }
        // max by length, the first of equals (as Python's `max`)
        let mut best: Option<&str> = None;
        let mut best_len = 0;
        for n in self.bases_by_name.keys() {
            if name.contains(n) && (best.is_none() || n.chars().count() > best_len) {
                best = Some(n);
                best_len = n.chars().count();
            }
        }
        best.map(|n| self.bases_by_name[n].as_str())
    }

    /// The tags an item of this base spawns mods with: the base's, plus a `_<influence>` copy of each.
    pub fn item_tags(&self, base_id: &str, influences: &BTreeSet<String>) -> BTreeSet<String> {
        let mut tags: BTreeSet<String> = self.bases[base_id].tags.iter().cloned().collect();
        // Python walks the influences in set order; sorted keeps ours deterministic.
        for influence in influences {
            let Some((_, suffix)) = INFLUENCE_TAGS.iter().find(|(k, _)| k == influence) else {
                continue;
            };
            let more: Vec<String> = tags.iter().map(|t| format!("{t}_{suffix}")).collect();
            tags.extend(more);
        }
        tags
    }

    /// The weight of the first of the mod's spawn weights whose tag the item has, else 0.
    pub fn spawn_weight(m: &Mod, tags: &BTreeSet<String>) -> i64 {
        m.spawn_weights
            .iter()
            .find(|e| tags.contains(&e.tag))
            .map_or(0, |e| e.weight)
    }

    /// Where a mod comes from on an item with these tags.
    pub fn source(&self, m: &Mod, tags: &BTreeSet<String>) -> &'static str {
        if let Some(s) = extra_domain(&m.domain) {
            return s;
        }
        if Self::spawn_weight(m, tags) > 0 {
            return "natural";
        }
        if m.is_essence_only {
            "essence"
        } else {
            "special"
        }
    }

    /// Tier of a mod among its family's natural rolls on these tags (1 = highest required level).
    pub fn tier(&self, mod_id: &str, tags: &BTreeSet<String>) -> Option<i64> {
        self.tier_at(self.mods.position(mod_id)?, tags)
    }

    fn tier_at(&self, i: usize, tags: &BTreeSet<String>) -> Option<i64> {
        let m = self.mods.entry(i).1;
        let key = self.tier_key[i];
        let cached = self
            .tiers
            .read()
            .unwrap()
            .get(tags)
            .and_then(|t| t.get(&key).cloned());
        // The cache is keyed by (type, generation type, tags) like Python's, so the
        // first caller's domain decides which family it holds.
        let levels = match cached {
            Some(l) => l,
            None => {
                let family = self.families.get(&(
                    m.domain.clone(),
                    m.type_.clone(),
                    m.generation_type.clone(),
                ));
                let mut levels: Vec<i64> = family
                    .into_iter()
                    .flatten()
                    .map(|&j| self.mods.entry(j).1)
                    .filter(|f| Self::spawn_weight(f, tags) > 0)
                    .map(|f| f.required_level)
                    .collect();
                levels.sort_unstable_by(|a, b| b.cmp(a));
                levels.dedup();
                let levels = Arc::new(levels);
                let mut w = self.tiers.write().unwrap();
                w.entry(tags.clone())
                    .or_default()
                    .entry(key)
                    .or_insert(levels)
                    .clone()
            }
        };
        if Self::spawn_weight(m, tags) <= 0 {
            return None;
        }
        levels
            .iter()
            .position(|&l| l == m.required_level)
            .map(|p| p as i64 + 1)
    }

    /// Match an item's explicit mod lines to mods. (Python `ModDB.match`.)
    pub fn match_item(&self, item: &Item) -> ItemMatch {
        let Some(base_id) = self.find_base(&item.base) else {
            return ItemMatch {
                item: item.clone(),
                unmatched: item.explicits.clone(),
                ..Default::default()
            };
        };
        let base = &self.bases[base_id];
        let tags = self.item_tags(base_id, &item.influences);
        let mut result = ItemMatch {
            item: item.clone(),
            base_id: Some(base_id.to_string()),
            item_class: Some(base.item_class.clone()),
            tags,
            ..Default::default()
        };
        let tags = result.tags.clone();
        let ctx = Ctx {
            item,
            base_domain: &base.domain,
            item_class: &base.item_class,
            tags: &tags,
            pob_ids: item.pob_affixes.iter().map(|(_, i)| i.as_str()).collect(),
        };
        let norms: Vec<String> = item.explicits.iter().map(|l| normalize(&l.text)).collect();

        // Lines still unexplained, as positions in item.explicits (Python compared lines by identity).
        let mut remaining: Vec<usize> = (0..item.explicits.len()).collect();
        // Hybrids first, so their lines aren't claimed one by one.
        for line in remaining.clone() {
            if !remaining.contains(&line) {
                continue;
            }
            let Some(keys) = self.index.hybrids_by_line.get(&norms[line]) else {
                continue;
            };
            for &k in keys {
                let key = &self.index.keys[k].0;
                let Some(claimed) = claim(key, &remaining, &norms) else {
                    continue;
                };
                if let Some(found) = self.best(key, &claimed, &ctx, &norms) {
                    result.mods.push(found);
                    remaining.retain(|r| !claimed.contains(r));
                    break;
                }
            }
        }

        for line in remaining.clone() {
            if let Some(found) =
                self.best(std::slice::from_ref(&norms[line]), &[line], &ctx, &norms)
            {
                result.mods.push(found);
                remaining.retain(|&r| r != line);
            }
        }

        for line in remaining.clone() {
            if self.split(line, &mut result, &ctx, &remaining, &norms) {
                remaining.retain(|&r| r != line);
            }
        }
        for &line in &remaining {
            let l = item.explicits[line].clone();
            if self.known(&norms[line]) {
                result.unmatched.push(l);
            } else {
                result.notes.push(l);
            }
        }
        result
    }

    /// An item whose explicit mods are known by id (e.g. from poe.ninja), no text matching.
    ///
    /// Unknown ids are skipped. Mirrors Python's signature, hence the many arguments.
    #[allow(clippy::too_many_arguments)]
    pub fn match_ids(
        &self,
        base: &str,
        mods: &IdMods,
        item_level: Option<i64>,
        influences: &BTreeSet<String>,
        rarity: &str,
        name: &str,
        header: &std::collections::BTreeMap<String, String>,
    ) -> ItemMatch {
        let mut item = Item::new(rarity, name, base);
        item.item_level = item_level;
        item.influences = influences.clone();
        item.header = header.clone();
        let Some(base_id) = self.find_base(base) else {
            return ItemMatch {
                item,
                ..Default::default()
            };
        };
        let b = &self.bases[base_id];
        let tags = self.item_tags(base_id, &item.influences);
        let mut found = Vec::new();
        for (marker, ids) in [
            (None, &mods.explicit),
            (Some("fractured"), &mods.fractured),
            (Some("crafted"), &mods.crafted),
        ] {
            for id in ids {
                let Some(i) = self.mods.position(id).filter(|&i| self.is_affix[i]) else {
                    continue;
                };
                let m = self.mods.entry(i).1;
                let text = m.text.as_deref().unwrap_or(id).split('\n').next().unwrap();
                let line = match marker {
                    Some(k) => ModLine::with_marker(text, k),
                    None => ModLine::new(text),
                };
                let source = self.source_at(i, &b.item_class, &tags);
                let tier = if source == "natural" {
                    self.tier_at(i, &tags)
                } else {
                    None
                };
                found.push(ModMatch {
                    mod_id: id.clone(),
                    kind: m.generation_type.clone(),
                    family: m.type_.clone(),
                    source,
                    tier,
                    lines: vec![line],
                    line_ids: vec![],
                    alternatives: 0,
                    merged: false,
                });
            }
        }
        ItemMatch {
            item,
            base_id: Some(base_id.to_string()),
            item_class: Some(b.item_class.clone()),
            tags,
            mods: found,
            ..Default::default()
        }
    }

    fn known(&self, norm: &str) -> bool {
        self.index
            .pos
            .contains_key(std::slice::from_ref(&norm.to_string()))
            || self.index.hybrids_by_line.contains_key(norm)
    }

    /// Explain a line the game summed from a hybrid mod plus a single mod.
    ///
    /// E.g. `140% increased Energy Shield` = hybrid `(39-42)% ES / Stun
    /// Recovery` + `(92-100)% ES`. The hybrid's other lines must be on the item;
    /// if they were matched to a single mod already, the hybrid takes them over.
    fn split(
        &self,
        line: usize,
        result: &mut ItemMatch,
        ctx: &Ctx,
        remaining: &[usize],
        norms: &[String],
    ) -> bool {
        let norm = &norms[line];
        let line_ref = &ctx.item.explicits[line];
        let value = line_values(line_ref);
        let Some(keys) = self.index.hybrids_by_line.get(norm) else {
            return false;
        };
        for &k in keys {
            let key = &self.index.keys[k].0;
            let mut others: Vec<String> = key.clone();
            if let Some(p) = others.iter().position(|o| o == norm) {
                others.remove(p);
            }
            // lines already explained by a single-line mod, by line position
            let taken: HashSet<usize> = result
                .mods
                .iter()
                .filter(|m| m.line_ids.len() == 1)
                .map(|m| m.line_ids[0])
                .collect();
            let pool: Vec<usize> = (0..ctx.item.explicits.len())
                .filter(|&l| l != line && (taken.contains(&l) || remaining.contains(&l)))
                .collect();
            let Some(claimed) = claim(&others, &pool, norms) else {
                continue;
            };
            for hybrid_pos in self.candidates(key, ctx, line_ref.crafted()) {
                let h = self.template(hybrid_pos);
                let h_other: Vec<usize> = (0..h.lines.len())
                    .filter(|&j| normalize(&h.lines[j]) != *norm)
                    .collect();
                if !claimed
                    .iter()
                    .zip(&h_other)
                    .all(|(&l, &j)| fits(&line_values(&ctx.item.explicits[l]), &h.ranges[j]))
                {
                    continue;
                }
                let Some(own) = (0..h.lines.len()).find(|&j| normalize(&h.lines[j]) == *norm)
                else {
                    continue;
                };
                let h_range = &h.ranges[own];
                for single_pos in
                    self.candidates(std::slice::from_ref(norm), ctx, line_ref.crafted())
                {
                    let s_range = template_ranges(self.mods.entry(single_pos).1.text_or_empty());
                    let summed: Vec<(f64, f64)> = h_range
                        .iter()
                        .zip(&s_range)
                        .map(|(a, b)| (a.0 + b.0, a.1 + b.1))
                        .collect();
                    if !fits(&value, &summed) {
                        continue;
                    }
                    for &l in &claimed {
                        if let Some(p) = result.mods.iter().position(|m| {
                            m.line_ids.len() == 1 && m.line_ids[0] == l && taken.contains(&l)
                        }) {
                            result.mods.remove(p);
                        }
                    }
                    // the hybrid's lines in template order; a repeated template line takes the first claimed twin
                    let ordered: Vec<usize> = (0..h.lines.len())
                        .map(|j| {
                            if normalize(&h.lines[j]) == *norm {
                                line
                            } else {
                                let first = h_other
                                    .iter()
                                    .position(|&o| h.lines[o] == h.lines[j])
                                    .unwrap();
                                claimed[first]
                            }
                        })
                        .collect();
                    let hybrid = self.make(hybrid_pos, &ordered, ctx, 0, true);
                    let single = self.make(single_pos, &[line], ctx, 0, true);
                    result.mods.push(hybrid);
                    result.mods.push(single);
                    return true;
                }
            }
        }
        false
    }

    /// Mods with this template that could be on the item, best first.
    fn candidates(&self, key: &[String], ctx: &Ctx, crafted: bool) -> Vec<usize> {
        let Some(&k) = self.index.pos.get(key) else {
            return vec![];
        };
        let mut out: Vec<((bool, bool, i32, i64), usize)> = Vec::new();
        for &i in &self.index.keys[k].1 {
            let m = self.mods.entry(i).1;
            if (m.domain == "crafted") != crafted {
                continue;
            }
            if m.domain != ctx.base_domain
                && !(extra_domain(&m.domain).is_some() && ctx.base_domain == "item")
            {
                continue;
            }
            out.push((self.rank(i, ctx), i));
        }
        out.sort_by_key(|(r, _)| *r);
        out.into_iter().map(|(_, i)| i).collect()
    }

    fn source_at(&self, i: usize, item_class: &str, tags: &BTreeSet<String>) -> &'static str {
        let (id, m) = self.mods.entry(i);
        let src = self.source(m, tags);
        if src == "crafted" && !self.bench.get(item_class).is_some_and(|b| b.contains(id)) {
            return "legacy-crafted"; // an old master craft, not on today's bench
        }
        src
    }

    fn rank(&self, i: usize, ctx: &Ctx) -> (bool, bool, i32, i64) {
        let (id, m) = self.mods.entry(i);
        let too_high = ctx.item.item_level.is_some_and(|l| m.required_level > l);
        (
            !ctx.pob_ids.contains(id),
            too_high,
            source_rank(self.source_at(i, ctx.item_class, ctx.tags)),
            -m.required_level,
        )
    }

    fn best(
        &self,
        key: &[String],
        lines: &[usize],
        ctx: &Ctx,
        _norms: &[String],
    ) -> Option<ModMatch> {
        let values: Vec<Vec<f64>> = lines
            .iter()
            .map(|&l| line_values(&ctx.item.explicits[l]))
            .collect();
        let crafted = lines.iter().any(|&l| ctx.item.explicits[l].crafted());
        let fitting: Vec<usize> = self
            .candidates(key, ctx, crafted)
            .into_iter()
            .filter(|&i| {
                let t = self.template(i);
                values.iter().zip(&t.ranges).all(|(v, r)| fits(v, r))
            })
            .collect();
        let first = *fitting.first()?;
        let best = self.rank(first, ctx);
        let ties = fitting[1..].iter().filter(|&&i| {
            let r = self.rank(i, ctx);
            (r.0, r.1, r.2) == (best.0, best.1, best.2)
        });
        Some(self.make(first, lines, ctx, ties.count(), false))
    }

    fn make(
        &self,
        i: usize,
        lines: &[usize],
        ctx: &Ctx,
        alternatives: usize,
        merged: bool,
    ) -> ModMatch {
        let (id, m) = self.mods.entry(i);
        let source = self.source_at(i, ctx.item_class, ctx.tags);
        ModMatch {
            mod_id: id.to_string(),
            kind: m.generation_type.clone(),
            family: m.type_.clone(),
            source,
            tier: if source == "natural" {
                self.tier_at(i, ctx.tags)
            } else {
                None
            },
            lines: lines
                .iter()
                .map(|&l| ctx.item.explicits[l].clone())
                .collect(),
            line_ids: lines.to_vec(),
            alternatives,
            merged,
        }
    }
}

/// The lines (positions) that carry each of `key`'s normalized lines, in key order, or None if one is missing.
fn claim(key: &[String], lines: &[usize], norms: &[String]) -> Option<Vec<usize>> {
    let mut claimed: Vec<usize> = Vec::with_capacity(key.len());
    for norm in key {
        let line = lines
            .iter()
            .copied()
            .find(|l| !claimed.contains(l) && norms[*l] == *norm)?;
        claimed.push(line);
    }
    Some(claimed)
}

/// Item class -> mod ids the crafting bench can currently add (empty if the data file is missing).
pub fn load_bench() -> Result<Bench, DataError> {
    match data::crafting_bench_options() {
        Ok(options) => Ok(bench_from(&options)),
        Err(DataError::Missing(_)) => Ok(Bench::new()),
        Err(e) => Err(e),
    }
}

/// [`load_bench`] over already loaded options.
pub fn bench_from(options: &[data::BenchOption]) -> Bench {
    let mut bench = Bench::new();
    for option in options {
        if let Some(mod_id) = option.add_explicit_mod().filter(|m| !m.is_empty()) {
            for class in &option.item_classes {
                bench
                    .entry(class.clone())
                    .or_default()
                    .insert(mod_id.to_string());
            }
        }
    }
    bench
}
