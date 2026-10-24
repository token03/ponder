//! What invested characters actually wear: mod and tier demand from ladder rares.
//!
//! Two views, both per base group (bases that roll the same mods, see `gear::group`):
//!
//! - `family_stats`: for each mod family, how many characters wear it, which
//!   tiers, and the *tier lift*: the ladder's share at a tier or better divided
//!   by the chance a roll of that family lands there. A lift far above 1 means
//!   people paid for the top tiers (e.g. T1 ES on ES bases); near 1 means tier
//!   barely matters to them.
//! - `archetypes`: mod-family combinations that many characters share, with
//!   resistances folded into one "resistances" token since they're filler.
//!   These are the crafting candidates: "Gloves (ES): flat ES + %ES + res".

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::Hash;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use indexmap::IndexMap;
use ponder_game::data;
use ponder_game::item::ModLine;
use ponder_game::mods::{IdMods, ItemMatch, ModDB, ModMatch};
use ponder_net::ladder::{Lite, RareItem};
use rayon::prelude::*;
use regex::Regex;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};
use serde_json::{Value, json};

use crate::mine::combinations;

pub const RES: &str = "resistances";
pub const NATURAL_LEVEL: i64 = 86;

// -- Counter ------------------------------------------------------------------------------

/// Python's `collections.Counter`: counts in insertion order, so `most_common` breaks ties as Python does.
#[derive(Debug, Clone, PartialEq)]
pub struct Counter<K: Hash + Eq>(pub IndexMap<K, i64>);

impl<K: Hash + Eq> Default for Counter<K> {
    fn default() -> Self {
        Counter(IndexMap::new())
    }
}

impl<K: Hash + Eq + Clone> Counter<K> {
    pub fn new() -> Self {
        Self::default()
    }

    /// `Counter({k: 1 for k in keys})` for a single-entry counter.
    pub fn of(key: K, n: i64) -> Self {
        let mut c = Counter::new();
        c.0.insert(key, n);
        c
    }

    /// `counter[key] += 1`
    pub fn add(&mut self, key: K) {
        *self.0.entry(key).or_insert(0) += 1;
    }

    /// `counter[key]`: 0 for a key never seen.
    pub fn get<Q: Hash + Eq + ?Sized>(&self, key: &Q) -> i64
    where
        K: std::borrow::Borrow<Q>,
    {
        self.0.get(key).copied().unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&K, i64)> {
        self.0.iter().map(|(k, v)| (k, *v))
    }

    /// `sum(counter.values())`
    pub fn total(&self) -> i64 {
        self.0.values().sum()
    }

    /// Highest count first; ties keep insertion order (a stable sort, as Python's).
    pub fn most_common(&self) -> Vec<(&K, i64)> {
        let mut v: Vec<(&K, i64)> = self.iter().collect();
        v.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        v
    }
}

impl<K: Hash + Eq + Clone + Ord> Counter<K> {
    /// The keys, smallest first.
    pub fn sorted_keys(&self) -> Vec<K> {
        let mut keys: Vec<K> = self.0.keys().cloned().collect();
        keys.sort();
        keys
    }
}

impl Counter<i64> {
    /// The tier a `share` of the items reach or beat (the worst tier that at least `share` of them have or beat).
    pub fn usual(&self, share: f64) -> i64 {
        let total = self.total();
        let mut seen = 0;
        for tier in self.sorted_keys() {
            seen += self.0[&tier];
            if seen as f64 >= share * total as f64 {
                return tier;
            }
        }
        *self.0.keys().max().expect("usual of an empty counter")
    }
}

/// `setdefault(key, Counter())` then `+= 1` on a dict of Counters.
fn tally<K: Hash + Eq + Clone, V: Hash + Eq + Clone>(map: &mut IndexMap<K, Counter<V>>, key: &K, value: V) {
    if !map.contains_key(key) {
        map.insert(key.clone(), Counter::new());
    }
    map[key].add(value);
}

// -- worn items ----------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Worn {
    /// account/name
    pub character: String,
    pub skill: String,
    pub match_: Arc<ItemMatch>,
    pub group: String,
}

#[derive(Debug, Clone)]
pub struct FamilyStat {
    pub group: String,
    pub family: String,
    pub side: String,
    /// e.g. "+# to maximum Energy Shield"
    pub text: String,
    pub characters: usize,
    /// tier -> items
    pub tiers: Counter<i64>,
    /// tier -> chance a roll of the family lands on it
    pub natural: IndexMap<i64, f64>,
}

impl FamilyStat {
    pub fn share_at_or_above(&self, tier: i64) -> f64 {
        let total = self.tiers.total();
        if total == 0 {
            return 0.0;
        }
        let n: i64 = self.tiers.iter().filter(|&(&t, _)| t <= tier).map(|(_, n)| n).sum();
        n as f64 / total as f64
    }

    pub fn natural_at_or_above(&self, tier: i64) -> f64 {
        // Python's sum() starts from the int 0 and adds in dict order.
        self.natural.iter().filter(|&(&t, _)| t <= tier).fold(0.0, |s, (_, p)| s + p)
    }

    pub fn lift(&self, tier: i64) -> f64 {
        let nat = self.natural_at_or_above(tier);
        if nat != 0.0 { self.share_at_or_above(tier) / nat } else { 0.0 }
    }
}

#[derive(Debug, Clone)]
pub struct Archetype {
    pub group: String,
    pub families: Vec<String>,
    pub characters: usize,
    pub items: usize,
    pub skills: Counter<String>,
    pub examples: Vec<Arc<ItemMatch>>,
    /// which bases characters wear it on
    pub bases: Counter<String>,
    /// main stat -> tiers worn
    pub tiers: IndexMap<String, Counter<i64>>,
    /// resistance mods per item
    pub resistances: Counter<usize>,
    /// other main stats on the same items: what finishes them
    pub others: Counter<String>,
    /// (family, side) of the other useful mods on its items
    pub fillers: Counter<(String, String)>,
    /// family -> tiers worn
    pub filler_tiers: IndexMap<String, Counter<i64>>,
    /// side -> useful mods per item ("prefix", "suffix")
    pub filled: IndexMap<String, Counter<usize>>,
    /// items wearing all of it / items expected if its mods were worn independently
    pub lift: f64,
}

impl Archetype {
    pub fn new(group: &str, families: Vec<String>, characters: usize, items: usize) -> Archetype {
        Archetype {
            group: group.to_string(),
            families,
            characters,
            items,
            skills: Counter::new(),
            examples: Vec::new(),
            bases: Counter::new(),
            tiers: IndexMap::new(),
            resistances: Counter::new(),
            others: Counter::new(),
            fillers: Counter::new(),
            filler_tiers: IndexMap::new(),
            filled: IndexMap::from([("prefix".to_string(), Counter::new()), ("suffix".to_string(), Counter::new())]),
            lift: 0.0,
        }
    }

    /// Specific demand: mods worn together far more than chance (one build's item), with real volume.
    pub fn niche(&self) -> f64 {
        self.lift.max(1.0).ln() * (self.characters as f64).sqrt()
    }

    /// The tier at least `share` of the items reach: what a buyer of this archetype expects.
    pub fn usual_tier(&self, key: &str, share: f64) -> Option<i64> {
        let c = self.tiers.get(key).filter(|c| !c.is_empty())?;
        let total = c.total();
        let mut run = 0;
        for t in c.sorted_keys() {
            run += c.0[&t];
            if run as f64 >= share * total as f64 {
                return Some(t);
            }
        }
        c.0.keys().max().copied()
    }
}

/// Every rare gear item and jewel on `characters` (poe.ninja characters, as `Lite`), matched by mod id.
///
/// Characters are matched in parallel; the result is in character order.
pub fn wear(db: &ModDB, characters: &[Lite], skill_of: Option<&(dyn Fn(&Lite) -> String + Sync)>) -> Vec<Worn> {
    let per_character: Vec<Vec<Worn>> = characters
        .par_iter()
        .map(|data| {
            let who = format!("{}/{}", py_str(&data.account), py_str(&data.name));
            let skill = match skill_of {
                Some(f) => f(data),
                None if data.skill.is_empty() => "?".to_string(), // `main_skill` of a character with no skill groups
                None => data.skill.clone(),
            };
            let mut out = Vec::new();
            for it in &data.rares {
                let m = match_rare(db, it);
                if m.base_id.is_none() || m.mods.is_empty() || m.item.base.ends_with("Cluster Jewel") {
                    continue;
                }
                let group = group_label(db, &m);
                out.push(Worn { character: who.clone(), skill: skill.clone(), match_: Arc::new(m), group });
            }
            out
        })
        .collect();
    per_character.into_iter().flatten().collect()
}

fn py_str(s: &Option<String>) -> &str {
    s.as_deref().unwrap_or("None")
}

/// Python truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// A poe.ninja item's mods by id, matched against the database.
fn match_rare(db: &ModDB, it: &RareItem) -> ItemMatch {
    let influences: BTreeSet<String> = match it.influences.as_ref().and_then(|v| v.as_ref()) {
        Some(Value::Object(o)) => o.iter().filter(|(_, v)| truthy(v)).map(|(k, _)| k.clone()).collect(),
        _ => BTreeSet::new(),
    };
    let ids = |kind: &str| -> Vec<String> { it.mods_of(kind).iter().map(|e| e.id().unwrap_or("").to_string()).collect() };
    let mods = IdMods { explicit: ids("explicit"), fractured: ids("fractured"), crafted: ids("crafted") };
    let ilvl = it.ilvl.flatten();
    db.match_ids(it.base_type().unwrap_or(""), &mods, ilvl, &influences, "RARE", "", &BTreeMap::new())
}

/// The character's main skill: the first active gem of the first skill group with DPS numbers.
pub fn main_skill(data: &Value) -> String {
    ponder_net::ladder::main_skill(data)
}

pub fn family_stats(db: &ModDB, worn: &[Worn], min_characters: usize) -> Vec<FamilyStat> {
    let mut stats: IndexMap<(&str, &str), FamilyStat> = IndexMap::new();
    let mut who: HashMap<(&str, &str), HashSet<&str>> = HashMap::new();
    let mut example: HashMap<&str, &ItemMatch> = HashMap::new();
    for w in worn {
        example.entry(w.group.as_str()).or_insert(&w.match_);
        for m in &w.match_.mods {
            let Some(tier) = m.tier.filter(|&t| t != 0) else { continue };
            if m.source != "natural" {
                continue;
            }
            let key = (w.group.as_str(), m.family.as_str());
            let s = stats
                .entry(key)
                .or_insert_with(|| FamilyStat { group: w.group.clone(), family: m.family.clone(), side: m.kind.clone(), text: template(db, &m.mod_id), characters: 0, tiers: Counter::new(), natural: IndexMap::new() });
            s.tiers.add(tier);
            who.entry(key).or_default().insert(&w.character);
        }
    }
    let mut out = Vec::new();
    for (key, mut s) in stats {
        s.characters = who[&key].len();
        if s.characters < min_characters {
            continue;
        }
        s.natural = natural_tiers(db, example[s.group.as_str()], &s.family);
        out.push(s);
    }
    out.sort_by(|a, b| a.group.cmp(&b.group).then(b.characters.cmp(&a.characters)));
    out
}

type NaturalKey = (Option<String>, BTreeSet<String>);
type Natural = Arc<HashMap<String, IndexMap<i64, f64>>>;
static NATURAL: LazyLock<Mutex<HashMap<NaturalKey, Natural>>> = LazyLock::new(Default::default);

/// Chance that a roll of `family` lands on each tier, on this item's mod pool at ilvl 86.
pub fn natural_tiers(db: &ModDB, m: &ItemMatch, family: &str) -> IndexMap<i64, f64> {
    let key = (m.item_class.clone(), m.tags.clone());
    let cached = NATURAL.lock().unwrap().get(&key).cloned();
    let table = match cached {
        Some(t) => t,
        None => {
            // one pool scan per base group, every family at once
            let mut weights: IndexMap<String, Counter<i64>> = IndexMap::new();
            for (id, weight) in pool_weights(db, m.base_id.as_deref().expect("matched item has a base"), &m.tags, NATURAL_LEVEL) {
                if let Some(t) = db.tier(&id, &m.tags).filter(|&t| t != 0) {
                    let family = db.mods[&id].type_.clone();
                    *weights.entry(family).or_default().0.entry(t).or_insert(0) += weight;
                }
            }
            let table: HashMap<String, IndexMap<i64, f64>> = weights
                .into_iter()
                .map(|(f, c)| {
                    let total = c.total() as f64;
                    let probs = c.0.iter().map(|(&t, &w)| (t, w as f64 / total)).collect();
                    (f, probs)
                })
                .collect();
            let table = Arc::new(table);
            NATURAL.lock().unwrap().entry(key).or_insert(table).clone()
        }
    };
    table.get(family).cloned().unwrap_or_default()
}

/// Calls `f` with each `size`-subset of `0..n` as sorted indexes, in `itertools.combinations` order.
fn each_combination(n: usize, size: usize, mut f: impl FnMut(&[usize])) {
    if size > n {
        return;
    }
    let mut idx: Vec<usize> = (0..size).collect();
    loop {
        f(&idx);
        let mut i = size;
        loop {
            if i == 0 {
                return;
            }
            i -= 1;
            if idx[i] != i + n - size {
                break;
            }
            if i == 0 {
                return;
            }
        }
        idx[i] += 1;
        for j in i + 1..size {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

/// A combination of interned stat keys, 16 bits each (key id + 1), in a u128.
type Packed = u128;
const SLOT_BITS: u32 = 16;

fn pack(ids: impl IntoIterator<Item = u16>) -> Packed {
    ids.into_iter().fold(0, |acc, id| (acc << SLOT_BITS) | (id as Packed + 1))
}

fn unpack(mut p: Packed) -> Vec<u16> {
    let mut out = Vec::new();
    while p != 0 {
        out.push(((p & 0xffff) - 1) as u16);
        p >>= SLOT_BITS;
    }
    out.reverse();
    out
}

/// Add 1 to `counter[key]`, allocating the key only when it is new.
fn add_str(c: &mut Counter<String>, key: &str) {
    match c.0.get_mut(key) {
        Some(n) => *n += 1,
        None => {
            c.0.insert(key.to_string(), 1);
        }
    }
}

/// `tiers.setdefault(key, Counter())[tier] += 1`, allocating the key only when it is new.
fn tally_str(map: &mut IndexMap<String, Counter<i64>>, key: &str, tier: i64) {
    match map.get_mut(key) {
        Some(c) => c.add(tier),
        None => {
            map.insert(key.to_string(), Counter::of(tier, 1));
        }
    }
}

/// `(&str, &str)` looked up in a `Counter<(String, String)>` without building the owned pair.
#[derive(Hash)]
struct PairRef<'a>(&'a str, &'a str);

impl indexmap::Equivalent<(String, String)> for PairRef<'_> {
    fn equivalent(&self, key: &(String, String)) -> bool {
        self.0 == key.0 && self.1 == key.1
    }
}

/// Mod combinations shared by many characters, per base group.
///
/// Mods are keyed by main stat (`stat_key`), resistances folded into one
/// token. A combination is dropped when one bigger combination is worn by at
/// least `implied` of its characters: it is mostly that one.
pub fn archetypes(worn: &[Worn], min_characters: usize, max_size: usize, min_size: usize, implied: f64) -> Vec<Archetype> {
    assert!(max_size <= 8, "combinations of more than 8 mods are not supported");
    // pass 1: who wears what (cheap), so pass 2 only details the survivors
    let keys_of: Vec<Vec<&'static str>> = worn.par_iter().map(|w| stat_keys(&w.match_)).collect();

    // Intern characters, groups and stat keys.
    let mut char_ids: FxHashMap<&str, u32> = FxHashMap::default();
    let mut group_ids: FxHashMap<&str, u32> = FxHashMap::default();
    let mut groups: Vec<&str> = Vec::new();
    let mut key_ids: FxHashMap<&'static str, u16> = FxHashMap::default();
    let mut key_names: Vec<&'static str> = Vec::new();
    let mut char_of: Vec<u32> = Vec::with_capacity(worn.len());
    let mut group_of: Vec<u32> = Vec::with_capacity(worn.len());
    for (w, keys) in worn.iter().zip(&keys_of) {
        let n = char_ids.len() as u32;
        char_of.push(*char_ids.entry(&w.character).or_insert(n));
        let n = group_ids.len() as u32;
        group_of.push(*group_ids.entry(&w.group).or_insert_with(|| {
            groups.push(&w.group);
            n
        }));
        for &k in keys {
            let n = key_ids.len() as u16;
            key_ids.entry(k).or_insert_with(|| {
                key_names.push(k);
                n
            });
        }
    }
    // Are a character's items contiguous? Then "already counted?" is just "same as the last one".
    let contiguous = {
        let mut closed: FxHashSet<u32> = FxHashSet::default();
        char_of.windows(2).all(|p| p[0] == p[1] || closed.insert(p[0])) && char_of.last().is_none_or(|l| !closed.contains(l))
    };

    type Key = (u32, Packed);
    let combos_of: Vec<Vec<Packed>> = keys_of
        .par_iter()
        .map(|keys| {
            let ids: Vec<u16> = keys.iter().map(|k| key_ids[k]).collect();
            let mut out = Vec::new();
            for size in min_size..=max_size.min(ids.len()) {
                each_combination(ids.len(), size, |c| out.push(pack(c.iter().map(|&i| ids[i]))));
            }
            out
        })
        .collect();

    let mut who: IndexMap<Key, Vec<u32>, FxBuildHasher> = IndexMap::default();
    let mut group_items: FxHashMap<u32, usize> = FxHashMap::default();
    let mut key_items: FxHashMap<(u32, u16), usize> = FxHashMap::default();
    let mut family_items: FxHashMap<(u32, &str), usize> = FxHashMap::default();
    for (i, w) in worn.iter().enumerate() {
        let (g, ch) = (group_of[i], char_of[i]);
        *group_items.entry(g).or_default() += 1;
        for k in &keys_of[i] {
            *key_items.entry((g, key_ids[k])).or_default() += 1;
        }
        let mut fams: Vec<&str> = worn_mods(&w.match_).map(|m| m.family.as_str()).collect();
        fams.sort_unstable();
        fams.dedup();
        for f in fams {
            *family_items.entry((g, f)).or_default() += 1;
        }
        for &c in &combos_of[i] {
            let v = who.entry((g, c)).or_default();
            if if contiguous { v.last() != Some(&ch) } else { !v.contains(&ch) } {
                v.push(ch);
            }
        }
    }
    let keys: Vec<&Key> = who.iter().filter(|(_, v)| v.len() >= min_characters).map(|(k, _)| k).collect();
    // Support only falls as a combination grows, so the best-supported bigger
    // combination is always one mod bigger: check those alone.
    let mut best_super: FxHashMap<Key, usize> = FxHashMap::default();
    for k in &keys {
        let ids = unpack(k.1);
        if ids.len() > min_size {
            let n = who[*k].len();
            for sub in combinations(&ids, ids.len() - 1) {
                let e = best_super.entry((k.0, pack(sub.into_iter().copied()))).or_default();
                *e = (*e).max(n);
            }
        }
    }
    let mut found_at: FxHashMap<Key, usize> = FxHashMap::default();
    let mut found: Vec<Archetype> = Vec::new();
    for k in &keys {
        let n = who[*k].len();
        if (best_super.get(*k).copied().unwrap_or(0) as f64) < implied * n as f64 {
            let families = unpack(k.1).into_iter().map(|id| key_names[id as usize].to_string()).collect();
            found_at.insert(**k, found.len());
            found.push(Archetype::new(groups[k.0 as usize], families, n, 0));
        }
    }
    // Pass 2, archetype by archetype (each in worn order): the items an archetype is found on, then its details.
    let hits_of: Vec<Vec<usize>> = (0..worn.len()).into_par_iter().map(|i| combos_of[i].iter().filter_map(|&c| found_at.get(&(group_of[i], c)).copied()).collect()).collect();
    let mut items_of: Vec<Vec<u32>> = vec![Vec::new(); found.len()];
    for (i, hits) in hits_of.iter().enumerate() {
        for &h in hits {
            items_of[h].push(i as u32);
        }
    }
    struct Detail<'a> {
        best: HashMap<&'static str, i64>,
        n_res: usize,
        /// the item's main stats, resistances aside
        keys: Vec<&'static str>,
        /// mods often worn on this kind of item count as useful: what fills a finished item's slots
        useful: Vec<(&'a ModMatch, &'static str)>,
        filled: [usize; 2],
    }
    let details: Vec<Option<Detail>> = (0..worn.len())
        .into_par_iter()
        .map(|i| {
            if hits_of[i].is_empty() {
                return None;
            }
            let w = &worn[i];
            let g = group_of[i];
            let (best, n_res) = best_tiers(&w.match_);
            let keys: Vec<&'static str> = keys_of[i].iter().copied().filter(|&k| k != RES).collect();
            let group_n = group_items[&g] as f64;
            let useful: Vec<(&ModMatch, &'static str)> =
                worn_mods(&w.match_).filter(|m| family_items[&(g, m.family.as_str())] as f64 >= USEFUL_SHARE * group_n).map(|m| (m, stat_key(&m.mod_id))).collect();
            let filled = ["prefix", "suffix"].map(|side| useful.iter().filter(|(m, _)| m.kind == side).count());
            Some(Detail { best, n_res, keys, useful, filled })
        })
        .collect();
    found.par_iter_mut().zip(items_of.par_iter()).for_each(|(a, items)| {
        for &i in items {
            let (w, d) = (&worn[i as usize], details[i as usize].as_ref().unwrap());
            a.items += 1;
            add_str(&mut a.skills, &w.skill);
            add_str(&mut a.bases, &w.match_.item.base);
            for f in &a.families {
                if let Some(&t) = d.best.get(f.as_str()) {
                    tally_str(&mut a.tiers, f, t);
                }
            }
            a.resistances.add(d.n_res);
            for &k in d.keys.iter().filter(|k| !a.families.iter().any(|f| f == *k)) {
                add_str(&mut a.others, k);
            }
            for &k in d.keys.iter().filter(|k| !a.families.iter().any(|f| f == *k)) {
                if let Some(&t) = d.best.get(k) {
                    tally_str(&mut a.tiers, k, t);
                }
            }
            for (side, n) in ["prefix", "suffix"].iter().zip(d.filled) {
                a.filled[*side].0.entry(n).and_modify(|c| *c += 1).or_insert(1);
            }
            for (m, key) in &d.useful {
                if !a.families.iter().any(|f| f == key) {
                    match a.fillers.0.get_mut(&PairRef(&m.family, &m.kind)) {
                        Some(n) => *n += 1,
                        None => {
                            a.fillers.0.insert((m.family.clone(), m.kind.clone()), 1);
                        }
                    }
                    if let Some(t) = m.tier.filter(|&t| t != 0) {
                        tally_str(&mut a.filler_tiers, &m.family, t);
                    }
                }
            }
            if a.examples.len() < 20 {
                a.examples.push(w.match_.clone());
            }
        }
    });
    for a in &mut found {
        let g = group_ids[a.group.as_str()];
        let n = group_items[&g] as f64;
        let mut independent = n;
        for k in &a.families {
            independent *= key_items.get(&(g, key_ids[k.as_str()])).copied().unwrap_or(0) as f64 / n;
        }
        a.lift = if independent != 0.0 { a.items as f64 / independent } else { 0.0 };
    }
    found.sort_by(|a, b| b.characters.cmp(&a.characters).then(b.families.len().cmp(&a.families.len())));
    found
}

/// A mod family worn on this share of a base group's items is useful there, not junk.
pub const USEFUL_SHARE: f64 = 0.05;

fn worn_mods(m: &ItemMatch) -> impl Iterator<Item = &ModMatch> {
    m.mods.iter().filter(|m| m.source == "natural" || m.source == "essence")
}

fn crafted(m: &ModMatch) -> bool {
    m.source == "crafted" || m.source == "legacy-crafted"
}

static STAT_KEYS: OnceLock<HashMap<String, String>> = OnceLock::new();

/// What a mod is "for": its first stat. A hybrid "%phys / accuracy" keys as %phys, and
/// "%ES / stun recovery" as %ES; flat ES and %ES stay apart (different stats).
///
/// Reads the default repoe data (as Python does); panics on an unknown mod id.
pub fn stat_key(mod_id: &str) -> &'static str {
    let table = STAT_KEYS.get_or_init(|| {
        let mods = data::mods().unwrap_or_else(|e| panic!("{e}"));
        mods.iter().map(|(id, m)| (id.to_string(), m.stats.first().map_or(id, |s| s.id.as_str()).to_string())).collect()
    });
    table.get(mod_id).unwrap_or_else(|| panic!("unknown mod id {mod_id:?}"))
}

/// The item's natural and essence mods by main stat, resistances folded into one token.
pub fn stat_keys(m: &ItemMatch) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> = m.mods.iter().filter(|x| !crafted(x)).map(|x| if is_filler(&x.family) { RES } else { stat_key(&x.mod_id) }).collect();
    keys.sort_unstable();
    keys.dedup();
    keys
}

/// Best natural tier per main stat on the item, and how many resistance mods it has.
pub fn best_tiers(m: &ItemMatch) -> (HashMap<&'static str, i64>, usize) {
    let mut best: HashMap<&'static str, i64> = HashMap::new();
    let mut n_res = 0;
    for x in m.mods.iter().filter(|x| !crafted(x)) {
        if is_filler(&x.family) {
            n_res += 1;
            continue;
        }
        if let Some(t) = x.tier.filter(|&t| t != 0) {
            let e = best.entry(stat_key(&x.mod_id)).or_insert(99);
            *e = (*e).min(t);
        }
    }
    (best, n_res)
}

/// The item's natural and essence mod families, resistances folded into one token.
pub fn core_families(m: &ItemMatch) -> Vec<String> {
    let mut fams: Vec<String> = m.mods.iter().filter(|x| !crafted(x)).map(|x| if is_filler(&x.family) { RES.to_string() } else { x.family.clone() }).collect();
    fams.sort();
    fams.dedup();
    fams
}

static NUMBERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(?-?\d+(\.\d+)?(-\d+(\.\d+)?)?\)?").unwrap());

/// A mod's text with numbers as #, every line (hybrids show both halves).
fn template(db: &ModDB, mod_id: &str) -> String {
    let text = db.mods[mod_id].text.as_deref().unwrap_or(mod_id);
    let joined = splitlines(text).join(" / ");
    NUMBERS.replace_all(&joined, "#").into_owned()
}

/// `str.splitlines()`.
fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(c, '\n' | '\r' | '\x0b' | '\x0c' | '\x1c' | '\x1d' | '\x1e' | '\u{85}' | '\u{2028}' | '\u{2029}') {
            out.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && matches!(chars.peek(), Some((_, '\n'))) {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

// -- cluster jewels ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct WornCluster {
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

#[derive(Debug, Clone, PartialEq)]
pub struct ClusterArchetype {
    pub size: String,
    pub enchant: String,
    pub tag: String,
    pub notables: Vec<String>,
    pub characters: usize,
    pub items: usize,
    pub skills: Counter<String>,
    /// passive counts worn
    pub passives: Counter<i64>,
    pub item_levels: Counter<i64>,
    /// items carrying exactly these notables and no other
    pub full: usize,
    /// passive jewels: group -> tiers worn
    pub tiers: IndexMap<String, Counter<i64>>,
}

impl ClusterArchetype {
    pub fn new(size: &str, enchant: &str, tag: &str, notables: Vec<String>) -> ClusterArchetype {
        ClusterArchetype {
            size: size.into(),
            enchant: enchant.into(),
            tag: tag.into(),
            notables,
            characters: 0,
            items: 0,
            skills: Counter::new(),
            passives: Counter::new(),
            item_levels: Counter::new(),
            full: 0,
            tiers: IndexMap::new(),
        }
    }

    pub fn title(&self) -> String {
        let mut size = self.size.to_lowercase();
        if let Some(c) = size.get_mut(0..1) {
            c.make_ascii_uppercase();
        }
        format!("{size} \u{b7} {}", self.enchant)
    }

    /// A "passive point" jewel: small-passive mods, no notables.
    pub fn passive(&self) -> bool {
        !self.notables.is_empty() && self.notables.iter().all(|n| n.contains('@'))
    }

    pub fn usual_passives(&self) -> i64 {
        *self.passives.most_common().first().expect("archetype with no passives").0
    }
}

/// Every rare cluster jewel on `characters` with its enchant, passive count and notables.
pub fn wear_clusters(characters: &[Lite], skill_of: Option<&(dyn Fn(&Lite) -> String + Sync)>) -> Vec<WornCluster> {
    let ctx = ClusterCtx::new();
    let per_character: Vec<Vec<WornCluster>> = characters
        .par_iter()
        .map(|c| {
            let who = format!("{}/{}", py_str(&c.account), py_str(&c.name));
            let skill = match skill_of {
                Some(f) => f(c),
                None if c.skill.is_empty() => "?".to_string(),
                None => c.skill.clone(),
            };
            c.rares.iter().filter_map(|it| ctx.worn(&who, &skill, it)).collect()
        })
        .collect();
    per_character.into_iter().flatten().collect()
}

const ENCHANT_PREFIX: &str = "Added Small Passive Skills grant: ";

type TierTables = HashMap<(String, String), Arc<HashMap<String, i64>>>;

/// Enchants by size and display text, and tier tables by (size, tag): computed once per `wear_clusters`.
struct ClusterCtx {
    mods: Arc<data::Mods>,
    by_text: HashMap<&'static str, HashMap<BTreeSet<String>, (String, String)>>,
    tiers: Mutex<TierTables>,
}

impl ClusterCtx {
    fn new() -> ClusterCtx {
        ClusterCtx {
            mods: data::mods().unwrap_or_else(|e| panic!("{e}")),
            by_text: cluster_enchants().into_iter().map(|(size, list)| (size, list.into_iter().map(|(short, tag)| (short.split(", ").map(str::to_string).collect(), (short, tag))).collect())).collect(),
            tiers: Mutex::new(HashMap::new()),
        }
    }

    fn tiers(&self, size: &str, tag: &str) -> Arc<HashMap<String, i64>> {
        let key = (size.to_string(), tag.to_string());
        if let Some(t) = self.tiers.lock().unwrap().get(&key) {
            return t.clone();
        }
        let t = Arc::new(cluster_mod_tiers(size, tag));
        self.tiers.lock().unwrap().entry(key).or_insert(t).clone()
    }

    fn worn(&self, who: &str, skill: &str, it: &RareItem) -> Option<WornCluster> {
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
            .flat_map(|text| splitlines(text))
            .filter_map(|ln| ln.strip_prefix(ENCHANT_PREFIX)) // line order varies
            .map(str::to_string)
            .collect();
        let (short, tag) = self.by_text.get(size.as_str())?.get(&lines)?;
        let passives = it
            .mods_of("enchant")
            .iter()
            .find_map(|m| m.0.get("stats").map(|s| s.get("local_jewel_expansion_passive_node_count").cloned()))
            .flatten()
            .filter(truthy)?;
        let passives = passives.as_i64().or_else(|| passives.as_f64().map(|f| f as i64))?;
        let (mut notables, mut fractured, mut small) = (Vec::new(), Vec::new(), Vec::new());
        let tiers = self.tiers(&size, tag);
        for source in ["explicit", "fractured"] {
            for m in it.mods_of(source) {
                let mod_ = m.id().and_then(|id| self.mods.get(id));
                let is_notable = mod_.is_some_and(cluster_is_notable);
                let added = match (mod_, m.id()) {
                    (Some(md), _) if is_notable => {
                        notables.push(cluster_notable_name(md));
                        notables.last().cloned()
                    }
                    (Some(_), Some(id)) if tiers.contains_key(id) => {
                        small.push(cluster_feature(id, tiers[id]));
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
        Some(WornCluster {
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

type ClusterKey<'a> = (&'a str, &'a str, Vec<&'a str>);

/// Notable combinations per (size, enchant) that several characters wear.
///
/// Every subset of a jewel's notables counts (a buyer of "Feed the Fury +
/// Martial Prowess" takes a third notable too) down to one fewer than the
/// size holds (2 of 3 on a large, 1 on a small); a subset is dropped when a
/// bigger combination covers `implied` of its characters.
pub fn cluster_archetypes(worn: &[WornCluster], min_characters: usize, implied: f64) -> Vec<ClusterArchetype> {
    let mut who: IndexMap<ClusterKey, HashSet<&str>> = IndexMap::new();
    let mut rec: HashMap<ClusterKey, ClusterArchetype> = HashMap::new();
    for w in worn {
        if w.notables.is_empty() {
            continue;
        }
        let names: Vec<&str> = w.notables.iter().map(String::as_str).collect();
        for size in 1.max(notable_cap(&w.size).saturating_sub(1))..=names.len() {
            for combo in combinations(&names, size) {
                let combo: Vec<&str> = combo.into_iter().copied().collect();
                let key = (w.size.as_str(), w.tag.as_str(), combo);
                let a = rec.entry(key.clone()).or_insert_with(|| ClusterArchetype::new(&w.size, &w.enchant, &w.tag, key.2.iter().map(|s| s.to_string()).collect()));
                a.items += 1;
                a.full += (size == names.len()) as usize;
                a.skills.add(w.skill.clone());
                a.passives.add(w.passives);
                a.item_levels.add(w.item_level);
                who.entry(key).or_default().insert(&w.character);
            }
        }
    }
    let mut out = keep(&who, &mut rec, min_characters, implied);
    out.extend(passive_archetypes(worn, min_characters, implied));
    out.sort_by(|a, b| b.characters.cmp(&a.characters).then(b.notables.len().cmp(&a.notables.len())));
    out
}

/// Combos worn by enough characters, minus those a bigger combo covers `implied` of.
fn keep<'a>(who: &IndexMap<ClusterKey<'a>, HashSet<&str>>, rec: &mut HashMap<ClusterKey<'a>, ClusterArchetype>, min_characters: usize, implied: f64) -> Vec<ClusterArchetype> {
    let keys: Vec<&ClusterKey> = who.iter().filter(|(_, v)| v.len() >= min_characters).map(|(k, _)| k).collect();
    let mut bigger: HashMap<(&str, &str), Vec<&ClusterKey>> = HashMap::new();
    for k in &keys {
        bigger.entry((k.0, k.1)).or_default().push(k);
    }
    let mut out = Vec::new();
    for k in keys {
        let n = who[k].len();
        let covered = bigger[&(k.0, k.1)].iter().any(|o| o.2.len() > k.2.len() && k.2.iter().all(|x| o.2.contains(x)) && who[*o].len() as f64 >= implied * n as f64);
        if covered {
            continue;
        }
        let mut a = rec.remove(k).expect("every counted combo has a record");
        a.characters = n;
        out.push(a);
    }
    out
}

/// A passive jewel craft wants at least this many small-passive mods.
pub const PASSIVE_MIN_MODS: usize = 2;

/// "Passive point" jewels: no notables, valued for Effect and "also grant" mods.
///
/// Mods combine by group (any tier); each archetype then asks for the tier at
/// least half its wearers have, so "GrantAttributes" worn at T1 by most
/// becomes "GrantAttributes@1".
pub fn passive_archetypes(worn: &[WornCluster], min_characters: usize, implied: f64) -> Vec<ClusterArchetype> {
    let mut who: IndexMap<ClusterKey, HashSet<&str>> = IndexMap::new();
    let mut rec: HashMap<ClusterKey, ClusterArchetype> = HashMap::new();
    for w in worn {
        if !w.notables.is_empty() || w.mods.len() < PASSIVE_MIN_MODS {
            continue;
        }
        let mut by_group: IndexMap<&str, i64> = IndexMap::new();
        for m in &w.mods {
            let (group, tier) = m.rsplit_once('@').expect("small-passive mods are Group@tier");
            by_group.insert(group, tier.parse().expect("tier is an integer"));
        }
        let mut groups: Vec<&str> = by_group.keys().copied().collect();
        groups.sort_unstable();
        for size in PASSIVE_MIN_MODS..=groups.len() {
            for combo in combinations(&groups, size) {
                let combo: Vec<&str> = combo.into_iter().copied().collect();
                let key = (w.size.as_str(), w.tag.as_str(), combo.clone());
                let a = rec.entry(key.clone()).or_insert_with(|| ClusterArchetype::new(&w.size, &w.enchant, &w.tag, combo.iter().map(|s| s.to_string()).collect()));
                a.items += 1;
                a.full += (size == groups.len()) as usize;
                a.skills.add(w.skill.clone());
                a.passives.add(w.passives);
                a.item_levels.add(w.item_level);
                for g in &combo {
                    tally(&mut a.tiers, &g.to_string(), by_group[g]);
                }
                who.entry(key).or_default().insert(&w.character);
            }
        }
    }
    let mut out = keep(&who, &mut rec, min_characters, implied);
    for a in &mut out {
        a.notables = a.notables.iter().map(|g| format!("{g}@{}", a.tiers[g].usual(0.5))).collect();
    }
    out
}

/// The worst tier that at least `share` of wearers have or beat.
pub fn usual(tiers: &Counter<i64>, share: f64) -> i64 {
    tiers.usual(share)
}

/// A mod most of an archetype's wearers also have is part of the finished item.
pub const COMPLETE_SHARE: f64 = 0.5;
/// Other mods on this share of its items are acceptable filler for the open slots.
pub const FILLER_SHARE: f64 = 0.1;
/// Wanted mods per item, filler aside.
pub const MAX_KEYS: usize = 4;

/// The archetype's mods plus those at least `COMPLETE_SHARE` of its items also carry, commonest first.
///
/// Archetypes are the combinations many characters share, so they are often
/// two mods of a five- or six-mod item; the buyer wants the rest too.
pub fn finished_keys(a: &Archetype) -> Vec<String> {
    let mut keys: Vec<String> = a.families.iter().filter(|k| *k != RES).cloned().collect();
    for (k, n) in a.others.most_common() {
        if keys.len() >= MAX_KEYS || (n as f64) < COMPLETE_SHARE * a.items as f64 {
            break;
        }
        keys.push(k.clone());
    }
    keys
}

// -- the parts that live in gear and cluster ---------------------------------------------

fn is_filler(family: &str) -> bool {
    crate::gear::is_filler_family(family)
}

fn group_label(db: &ModDB, m: &ItemMatch) -> String {
    crate::gear::group(db, m).label.clone()
}

fn pool_weights(db: &ModDB, base_id: &str, tags: &BTreeSet<String>, level: i64) -> Vec<(String, i64)> {
    crate::gear::pool(db, base_id, tags, level).iter().map(|m| (m.id.clone(), m.weight as i64)).collect()
}

/// Python's `round()` of a half-integer median: ties go to the even number.
fn round_half_even(x: f64) -> i64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r as i64 % 2 != 0 { r as i64 - x.signum() as i64 } else { r as i64 }
}

/// `statistics.median` of the counter's elements.
fn median(c: &Counter<usize>) -> f64 {
    let mut v: Vec<usize> = c.iter().flat_map(|(&k, n)| std::iter::repeat_n(k, n as usize)).collect();
    v.sort_unstable();
    let n = v.len();
    if n % 2 == 1 { v[n / 2] as f64 } else { (v[n / 2 - 1] + v[n / 2]) as f64 / 2.0 }
}

/// The finished item an archetype's wearers have, as `pipeline.estimate_gear` costs it.
///
/// Its wanted mods are the archetype's plus those most of its wearers also
/// have (`finished_keys`), each at the tier most of them reach. The open
/// slots are filler: on each side, as many as its wearers' items usually
/// have useful mods there, from any mod at least `FILLER_SHARE` of them wear
/// (resistances always count), at their usual tier. That goes in
/// `ItemMatch::finish` for `gear::build`.
pub fn archetype_match(db: &ModDB, a: &Archetype) -> Option<ItemMatch> {
    let ex = &a.examples[0];
    let mut pool: HashMap<(String, Option<i64>), String> = HashMap::new();
    for (id, _) in pool_weights(db, ex.base_id.as_deref()?, &ex.tags, NATURAL_LEVEL) {
        pool.insert((db.mods[&id].type_.clone(), db.tier(&id, &ex.tags)), id);
    }
    let mut mods: Vec<ModMatch> = Vec::new();
    for key in finished_keys(a) {
        let found: Vec<&ModMatch> = a
            .examples
            .iter()
            .flat_map(|e| e.mods.iter())
            .filter(|m| !crafted(m) && m.tier.is_some_and(|t| t != 0) && stat_key(&m.mod_id) == key)
            .collect();
        if found.is_empty() {
            if a.families.contains(&key) {
                return None;
            }
            continue; // an extra mod the examples happen not to show
        }
        // the plain mod when there is one (first of equals, as Python's `min`)
        let mut m = found[0];
        let rank = |m: &ModMatch| (template(db, &m.mod_id).contains('/'), m.tier);
        for &c in &found[1..] {
            if rank(c) < rank(m) {
                m = c;
            }
        }
        if !a.families.contains(&key) && mods.iter().filter(|x| x.kind == m.kind).count() >= 3 {
            continue;
        }
        let tier = a.usual_tier(&key, 0.5).or(m.tier).expect("a found mod has a tier");
        let mod_id = pool.get(&(m.family.clone(), Some(tier))).cloned().unwrap_or_else(|| m.mod_id.clone());
        let lines: Vec<ModLine> = splitlines(db.mods[&mod_id].text_or_empty()).into_iter().map(ModLine::new).collect();
        let mut new = m.clone();
        new.mod_id = mod_id;
        new.tier = Some(tier);
        if !lines.is_empty() {
            new.lines = lines;
        }
        mods.push(new);
    }
    let side_count = |side: &str| mods.iter().filter(|m| m.kind == side).count();
    let sides = [side_count("prefix"), side_count("suffix")];
    if sides[0] > 3 || sides[1] > 3 {
        return None;
    }
    let taken: HashSet<&str> = mods.iter().map(|m| m.family.as_str()).collect();
    let mut families: IndexMap<String, i64> = crate::gear::FILLER_FAMILIES.iter().map(|f| (f.to_string(), crate::gear::FILLER_TIER)).collect();
    let mut order: Vec<String> = Vec::new();
    for ((family, _side), n) in a.fillers.most_common() {
        if (n as f64) < FILLER_SHARE * a.items as f64 || taken.contains(family.as_str()) {
            continue;
        }
        if !families.contains_key(family) {
            let tiers = a.filler_tiers.get(family).filter(|c| !c.is_empty()).cloned().unwrap_or_else(|| Counter::of(crate::gear::FILLER_TIER, 1));
            families.insert(family.clone(), tiers.usual(0.8)); // filler: lenient
        }
        if !order.contains(family) {
            order.push(family.clone());
        }
    }
    let mut need = [0i64; 2];
    for (i, side) in ["prefix", "suffix"].iter().enumerate() {
        let c = &a.filled[*side];
        let usual = if c.is_empty() { 0 } else { round_half_even(median(c)) };
        need[i] = (usual.min(3) - sides[i] as i64).max(0);
    }
    let finish = json!({"families": families, "sides": need, "order": order});
    let mut out = (**ex).clone();
    out.mods = mods;
    out.unmatched = Vec::new();
    out.notes = Vec::new();
    out.finish = Some(finish);
    Some(out)
}

fn cluster_enchants() -> Vec<(&'static str, Vec<(String, String)>)> {
    ["small", "medium", "large"].into_iter().map(|size| (size, crate::cluster::enchants(size).into_iter().map(|e| (e.short, e.tag)).collect())).collect()
}

fn cluster_mod_tiers(size: &str, tag: &str) -> HashMap<String, i64> {
    crate::cluster::mod_tiers(size, tag).into_iter().collect()
}

fn cluster_is_notable(m: &data::Mod) -> bool {
    crate::cluster::is_notable(m)
}

fn cluster_notable_name(m: &data::Mod) -> String {
    crate::cluster::notable_name(m)
}

fn cluster_feature(mod_id: &str, tier: i64) -> String {
    crate::cluster::feature(mod_id, tier)
}

fn notable_cap(size: &str) -> usize {
    crate::cluster::notable_cap(size)
}
