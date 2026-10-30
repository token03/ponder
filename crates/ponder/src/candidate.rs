//! Candidate mining and finished-item requirements.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::Hash;
use std::sync::{Arc, LazyLock, Mutex, OnceLock};

use indexmap::IndexMap;
use ponder_game::mods::{ItemMatch, ModDB, ModMatch};
use ponder_game::{data, item::split_lines};
use rayon::prelude::*;
use regex::Regex;
use rustc_hash::{FxBuildHasher, FxHashMap, FxHashSet};

use crate::combinations::for_each as each_combination;
use crate::item::{Cluster as Jewel, Item};
use crate::pattern::combinations;
use serde::{Deserialize, Serialize};

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

    /// Specific candidate: mods worn together far more than chance (one build's item), with real volume.
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

pub fn family_stats(db: &ModDB, worn: &[Item], min_characters: usize) -> Vec<FamilyStat> {
    let mut stats: IndexMap<(&str, &str), FamilyStat> = IndexMap::new();
    let mut who: HashMap<(&str, &str), HashSet<&str>> = HashMap::new();
    let mut example: HashMap<&str, &ItemMatch> = HashMap::new();
    for w in worn {
        example.entry(w.group.as_str()).or_insert(&w.match_);
        for m in &w.match_.mods {
            let Some(tier) = m.tier.filter(|&t| t != 0) else {
                continue;
            };
            if m.source != "natural" {
                continue;
            }
            let key = (w.group.as_str(), m.family.as_str());
            let s = stats.entry(key).or_insert_with(|| FamilyStat {
                group: w.group.clone(),
                family: m.family.clone(),
                side: m.kind.clone(),
                text: template(db, &m.mod_id),
                characters: 0,
                tiers: Counter::new(),
                natural: IndexMap::new(),
            });
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
pub fn merge(worn: &[Item], min_characters: usize, max_size: usize, min_size: usize, implied: f64) -> Vec<Archetype> {
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
    let hits_of: Vec<Vec<usize>> = (0..worn.len())
        .into_par_iter()
        .map(|i| combos_of[i].iter().filter_map(|&c| found_at.get(&(group_of[i], c)).copied()).collect())
        .collect();
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
            let useful: Vec<(&ModMatch, &'static str)> = worn_mods(&w.match_)
                .filter(|m| family_items[&(g, m.family.as_str())] as f64 >= USEFUL_SHARE * group_n)
                .map(|m| (m, stat_key(&m.mod_id)))
                .collect();
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
    m.source == "crafted"
}

static STAT_KEYS: OnceLock<HashMap<String, String>> = OnceLock::new();

/// What a mod is "for": its first stat. A hybrid "%phys / accuracy" keys as %phys, and
/// "%ES / stun recovery" as %ES; flat ES and %ES stay apart (different stats).
///
/// Reads the default repoe data (as Python does); panics on an unknown mod id.
pub fn stat_key(mod_id: &str) -> &'static str {
    let table = STAT_KEYS.get_or_init(|| {
        let mods = data::mods().unwrap_or_else(|e| panic!("{e}"));
        mods.iter().map(|(id, m)| (id.to_string(), m.stats.first().map_or(id.as_str(), |s| s.id.as_str()).to_string())).collect()
    });
    table.get(mod_id).unwrap_or_else(|| panic!("unknown mod id {mod_id:?}"))
}

/// The item's natural and essence mods by main stat, resistances folded into one token.
pub fn stat_keys(m: &ItemMatch) -> Vec<&'static str> {
    let mut keys: Vec<&'static str> =
        m.mods.iter().filter(|x| !crafted(x)).map(|x| if is_filler(&x.family) { RES } else { stat_key(&x.mod_id) }).collect();
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
    let mut fams: Vec<String> =
        m.mods.iter().filter(|x| !crafted(x)).map(|x| if is_filler(&x.family) { RES.to_string() } else { x.family.clone() }).collect();
    fams.sort();
    fams.dedup();
    fams
}

static NUMBERS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\(?-?\d+(\.\d+)?(-\d+(\.\d+)?)?\)?").unwrap());

/// Readable names for an archetype's main stats ("+# to maximum Life", "resistances"): the text
/// of a worn mod with that stat, a plain one when its wearers have one.
pub fn names(db: &ModDB, a: &Archetype) -> Vec<String> {
    a.families
        .iter()
        .map(|key| {
            if key == RES {
                return "resistances".to_string();
            }
            let mods: Vec<&ModMatch> =
                a.examples.iter().flat_map(|ex| &ex.mods).filter(|m| !crafted(m) && stat_key(&m.mod_id) == key).collect();
            let plain = mods.iter().find(|m| !template(db, &m.mod_id).contains('/'));
            plain.or(mods.first()).map_or_else(|| key.clone(), |m| template(db, &m.mod_id))
        })
        .collect()
}

/// A mod's text with numbers as #, every line (hybrids show both halves).
fn template(db: &ModDB, mod_id: &str) -> String {
    let text = db.mods[mod_id].text.as_deref().unwrap_or(mod_id);
    let joined = split_lines(text).join(" / ");
    NUMBERS.replace_all(&joined, "#").into_owned()
}

// -- cluster jewels ---------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct Cluster {
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
    /// passive jewels: the other groups their wearers fill the open slots with -> tiers worn
    pub others: IndexMap<String, Counter<i64>>,
    /// passive jewels: "Group@tier" mods worn in the open slots by `FILLER_SHARE` of the items
    pub filler: Vec<String>,
}

impl Cluster {
    pub fn new(size: &str, enchant: &str, tag: &str, notables: Vec<String>) -> Cluster {
        Cluster {
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
            others: IndexMap::new(),
            filler: Vec::new(),
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

type ClusterKey<'a> = (&'a str, &'a str, Vec<&'a str>);

/// Notable combinations per (size, enchant) that several characters wear.
///
/// Only full jewels count: as many notables as the size holds (3 on a large, 2 on a medium).
/// Fewer is a step on the way to one of those, not a product: the ladder wears 2-notable
/// mediums thirty times as often as 1-notable ones. A combination is dropped when a bigger
/// one covers `implied` of its characters.
pub fn clusters(worn: &[Jewel], min_characters: usize, implied: f64) -> Vec<Cluster> {
    let (who, mut rec) = cluster_sets(worn, false);
    let mut out = keep(&who, &mut rec, min_characters, implied);
    out.extend(passives(worn, min_characters, implied));
    out.sort_by(|a, b| b.characters.cmp(&a.characters).then(b.notables.len().cmp(&a.notables.len())));
    out
}

/// Combos worn by enough characters, minus those a bigger combo covers `implied` of.
fn keep<'a>(
    who: &IndexMap<ClusterKey<'a>, HashSet<&str>>,
    rec: &mut HashMap<ClusterKey<'a>, Cluster>,
    min_characters: usize,
    implied: f64,
) -> Vec<Cluster> {
    let keys: Vec<&ClusterKey> = who.iter().filter(|(_, v)| v.len() >= min_characters).map(|(k, _)| k).collect();
    let mut bigger: HashMap<(&str, &str), Vec<&ClusterKey>> = HashMap::new();
    for k in &keys {
        bigger.entry((k.0, k.1)).or_default().push(k);
    }
    let mut out = Vec::new();
    for k in keys {
        let n = who[k].len();
        let covered = bigger[&(k.0, k.1)]
            .iter()
            .any(|o| o.2.len() > k.2.len() && k.2.iter().all(|x| o.2.contains(x)) && who[*o].len() as f64 >= implied * n as f64);
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
pub fn passives(worn: &[Jewel], min_characters: usize, implied: f64) -> Vec<Cluster> {
    let (who, mut rec) = cluster_sets(worn, true);
    let mut out = keep(&who, &mut rec, min_characters, implied);
    for a in &mut out {
        a.notables = a.notables.iter().map(|g| format!("{g}@{}", a.tiers[g].usual(TARGET_SHARE))).collect();
        a.filler = a
            .others
            .iter()
            .filter(|(_, tiers)| tiers.total() as f64 >= FILLER_SHARE * a.items as f64)
            .map(|(g, tiers)| format!("{g}@{}", tiers.usual(0.8))) // filler: lenient
            .collect();
    }
    out
}

fn cluster_sets(worn: &[Jewel], passive: bool) -> (IndexMap<ClusterKey<'_>, HashSet<&str>>, HashMap<ClusterKey<'_>, Cluster>) {
    let mut who: IndexMap<ClusterKey, HashSet<&str>> = IndexMap::new();
    let mut rec: HashMap<ClusterKey, Cluster> = HashMap::new();
    for w in worn {
        if passive != w.notables.is_empty() || (passive && w.mods.len() < PASSIVE_MIN_MODS) {
            continue;
        }
        let by_group: IndexMap<&str, i64> = w
            .mods
            .iter()
            .filter_map(|m| {
                let (group, tier) = m.rsplit_once('@')?;
                Some((group, tier.parse().expect("tier is an integer")))
            })
            .collect();
        let mut names: Vec<&str> =
            if passive { by_group.keys().copied().collect() } else { w.notables.iter().map(String::as_str).collect() };
        if passive {
            names.sort_unstable();
        }
        let minimum = if passive { PASSIVE_MIN_MODS } else { crate::cluster::notable_cap(&w.size) };
        for size in minimum..=names.len() {
            for combo in combinations(&names, size) {
                let combo: Vec<&str> = combo.into_iter().copied().collect();
                let key = (w.size.as_str(), w.tag.as_str(), combo.clone());
                let a = rec
                    .entry(key.clone())
                    .or_insert_with(|| Cluster::new(&w.size, &w.enchant, &w.tag, combo.iter().map(|s| s.to_string()).collect()));
                a.items += 1;
                a.full += (size == names.len()) as usize;
                a.skills.add(w.skill.clone());
                a.passives.add(w.passives);
                a.item_levels.add(w.item_level);
                if passive {
                    for g in &combo {
                        tally(&mut a.tiers, &g.to_string(), by_group[g]);
                    }
                    for g in names.iter().filter(|g| !combo.contains(g)) {
                        tally(&mut a.others, &g.to_string(), by_group[g]);
                    }
                }
                who.entry(key).or_default().insert(&w.character);
            }
        }
    }
    (who, rec)
}

/// The worst tier that at least `share` of wearers have or beat.
pub fn usual(tiers: &Counter<i64>, share: f64) -> i64 {
    tiers.usual(share)
}

/// A mod most of an archetype's wearers also have is part of the finished item.
pub const COMPLETE_SHARE: f64 = 0.5;
pub const FILLER_SHARE: f64 = 0.1;
/// A target's tier: the worst that this share of its wearers have or beat (a product most buyers take).
pub const TARGET_SHARE: f64 = 0.75;
/// The commonest bench craft is part of the finished item when this share of its examples have it.
pub const CRAFTED_SHARE: f64 = 0.1;

/// The archetype's mods plus those at least `COMPLETE_SHARE` of its items also carry, commonest first.
///
/// Archetypes are the combinations many characters share, so they are often
/// two mods of a five- or six-mod item; the buyer wants the rest too.
pub fn finished_keys(a: &Archetype) -> Vec<String> {
    let mut keys: Vec<String> = a.families.iter().filter(|k| *k != RES).cloned().collect();
    for (k, n) in a.others.most_common() {
        if (n as f64) < COMPLETE_SHARE * a.items as f64 {
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

fn pool_weights(db: &ModDB, base_id: &str, tags: &BTreeSet<String>, level: i64) -> Vec<(String, i64)> {
    crate::gear::pool(db, base_id, tags, level).iter().map(|m| (m.id.clone(), m.weight as i64)).collect()
}

pub fn representative(db: &ModDB, a: &Archetype) -> Option<ItemMatch> {
    let keys = finished_keys(a);
    let required: BTreeSet<&str> = keys.iter().map(String::as_str).collect();
    let useful = |m: &ModMatch| {
        let key = stat_key(&m.mod_id);
        is_filler(&m.family)
            || required.contains(key)
            || a.others.0.get(key).is_some_and(|n| *n as f64 >= FILLER_SHARE * a.items as f64)
            || crafted(m)
    };
    let representative =
        a.examples.iter().filter(|ex| a.families.iter().all(|key| stat_keys(ex).contains(&key.as_str()))).max_by_key(|ex| {
            let covered = ex.mods.iter().filter(|m| required.contains(stat_key(&m.mod_id))).count();
            let useful_count = ex.mods.iter().filter(|m| useful(m)).count();
            let distance: i64 =
                ex.mods.iter().filter_map(|m| Some((m.tier? - a.usual_tier(stat_key(&m.mod_id), TARGET_SHARE)?).abs())).sum();
            (covered, useful_count, std::cmp::Reverse(distance))
        })?;
    let limits = crate::gear::base_limits(db, representative.base_id.as_deref()?);
    let mut out = (**representative).clone();
    out.mods.retain(useful);
    for modifier in &mut out.mods {
        for line in &mut modifier.lines {
            line.markers.remove("fractured");
        }
    }
    out.item.flags.remove("fractured");
    for line in &mut out.item.explicits {
        line.markers.remove("fractured");
    }
    if ["prefix", "suffix"].iter().enumerate().any(|(i, side)| out.mods.iter().filter(|m| m.kind == *side).count() > limits[i]) {
        return None;
    }
    out.finish = None;
    Some(out)
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Need {
    pub name: String,
    pub mods: BTreeSet<String>,
    pub count: u8,
    pub fractured: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Candidate {
    pub base: String,
    pub item_level: u8,
    pub influences: BTreeSet<String>,
    pub needs: Vec<Need>,
    pub fixed: Vec<String>,
    pub min_mods: u8,
}

impl Candidate {
    pub fn fingerprint(&self) -> blake3::Hash {
        let needs: Vec<_> = self.needs.iter().map(|need| (&need.mods, need.count, need.fractured)).collect();
        blake3::hash(
            &serde_json::to_vec(&(&self.base, self.item_level, &self.influences, needs, &self.fixed, self.min_mods))
                .expect("crafting request serializes"),
        )
    }
}

pub fn select(groups: &[Archetype], top: usize, per_group: usize, niche: usize) -> Vec<usize> {
    let mut picked: Vec<usize> = Vec::new();
    let mut per: HashMap<&str, usize> = HashMap::new();
    for (i, a) in groups.iter().enumerate() {
        let n = per.entry(a.group.as_str()).or_insert(0);
        if picked.len() < top || *n < per_group {
            picked.push(i);
            *n += 1;
        }
    }
    let chosen: HashSet<usize> = picked.iter().copied().collect();
    let niche_of: Vec<f64> = groups.iter().map(Archetype::niche).collect();
    let mut by_niche: Vec<usize> = (0..groups.len()).collect();
    by_niche.sort_by(|&a, &b| niche_of[b].partial_cmp(&niche_of[a]).unwrap_or(std::cmp::Ordering::Equal));
    let mut niches: Vec<usize> = Vec::new();
    for i in by_niche {
        let a = &groups[i];
        if niches.len() >= niche {
            break;
        }
        if chosen.contains(&i) || a.families.len() < 3 {
            continue;
        }
        let variant = niches.iter().any(|&j| {
            let o = &groups[j];
            let shared = o.families.iter().filter(|f| a.families.contains(f)).count() as i64;
            o.group == a.group && shared >= o.families.len().min(a.families.len()) as i64 - 1
        });
        if !variant {
            niches.push(i);
        }
    }
    picked.extend(niches);
    picked
}
