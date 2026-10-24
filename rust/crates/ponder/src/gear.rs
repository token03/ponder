//! Port of src/ponder/gear.py: turn a matched rare item into a crafting problem.
//!
//! Each natural mod on the item becomes a target that any tier at least as good
//! satisfies (`settle` loosens that by N tiers). Fractured mods are kept fixed.
//! Essence-only mods become targets only an essence can hit. Crafted mods the
//! bench still offers become a last bench step: the plan keeps a slot open on
//! their side and pays the bench cost. Veiled and other special mods are left
//! for the user.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Result, anyhow};
use ponder_core::{Action, Kind, Mod, Plan, Prices, Problem, Settings, Side, Target, solve};
use ponder_game::data;
use ponder_game::mods::{ItemMatch, ModDB};
use regex::Regex;
use serde_json::Value;

use crate::actions::{self, Cost, Profiles, loaded};

pub const JEWEL_CLASSES: [&str; 2] = ["Jewel", "AbyssJewel"];
pub const DEFAULT_ITEM_LEVEL: i64 = 84;

pub fn defence_name(tag: &str) -> Option<&'static str> {
    match tag {
        "str" => Some("Armour"),
        "dex" => Some("Evasion"),
        "int" => Some("ES"),
        _ => None,
    }
}
/// Resistances are filler: buyers want enough of them, rarely a particular one. Sorted, like Python's `sorted(FILLER_FAMILIES)`.
pub const FILLER_FAMILIES: [&str; 5] = ["AllResistances", "ChaosResistance", "ColdResistance", "FireResistance", "LightningResistance"];
pub const FILLER_TIER: i64 = 3; // filler at this tier or better counts (or the item's own tier, if worse)
/// 1-4 fossil resonators, pre-scored (`actions::fossil_combos`); false: single fossils only.
pub const FOSSIL_COMBOS: bool = true;

pub fn is_filler_family(family: &str) -> bool {
    FILLER_FAMILIES.contains(&family)
}

fn defence_tag() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^((?:str|dex|int)(?:_(?:str|dex|int))*)_(armour|shield)$").unwrap())
}

/// What the market charges: prices of every currency, and what an hour of clicking is worth.
#[derive(Clone, Copy)]
pub struct Pricing<'a> {
    pub prices: &'a Prices,
    pub settings: &'a Settings,
}

impl<'a> Pricing<'a> {
    pub fn new(prices: &'a Prices, settings: &'a Settings) -> Pricing<'a> {
        Pricing { prices, settings }
    }
}

/// One mod line of a craft, for display.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// target | essence | fractured | bench | skipped | filler
    pub role: &'static str,
    pub side: Side,
    pub text: String,
    /// e.g. "T2+"
    pub tier: String,
}

impl Row {
    fn new(role: &'static str, side: Side, text: impl Into<String>, tier: impl Into<String>) -> Row {
        Row { role, side, text: text.into(), tier: tier.into() }
    }
}

/// Bases that roll exactly the same mods with the same weights: one craft for all of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseGroup {
    pub item_class: String,
    /// the base's tags that some mod's spawn weight uses
    pub signature: BTreeSet<String>,
    /// e.g. "Gloves (ES)", "Ring", "Cobalt Jewel"
    pub label: String,
    /// names, best first (see [`group`])
    pub bases: Vec<String>,
}

impl BaseGroup {
    /// The base to craft on and sell: the same mods roll on it, and it sells for more.
    pub fn best(&self) -> &str {
        &self.bases[0]
    }
}

#[derive(Clone, Debug)]
pub struct GearProblem {
    pub problem: Problem,
    pub item_level: i64,
    /// mods we don't try to craft
    pub skipped: Vec<String>,
    /// (label, cost) crafted last
    pub bench: Vec<(String, Cost)>,
    pub rows: Vec<Row>,
    pub bench_ids: Vec<String>,
    /// families of the item's filler mods
    pub filler_families: Vec<String>,
}

impl GearProblem {
    pub fn bench_cost(&self, prices: &Prices) -> f64 {
        self.bench.iter().map(|(_, cost)| action_cost_of(cost, prices)).sum()
    }
}

fn side_of(generation_type: &str) -> Side {
    match generation_type {
        "prefix" => Side::Prefix,
        "suffix" => Side::Suffix,
        other => panic!("{other:?} is not an affix"),
    }
}

/// Every prefix or suffix that can roll on a base with these tags at this item level, in `mods.json` order.
pub fn pool(db: &ModDB, base_id: &str, tags: &BTreeSet<String>, item_level: i64) -> Vec<Mod> {
    let domain = &db.bases[base_id].domain;
    let mut out = Vec::new();
    for (mod_id, m) in db.mods.iter() {
        if m.domain != *domain || (m.generation_type != "prefix" && m.generation_type != "suffix") || m.text_or_empty().is_empty() {
            continue;
        }
        if m.required_level > item_level {
            continue;
        }
        let weight = ModDB::spawn_weight(m, tags);
        if weight > 0 {
            out.push(to_mod(mod_id, m, weight));
        }
    }
    out
}

/// Every tag that some item mod's spawn weight mentions.
pub fn spawn_tags() -> &'static BTreeSet<String> {
    static TAGS: OnceLock<BTreeSet<String>> = OnceLock::new();
    TAGS.get_or_init(|| {
        loaded(data::mods())
            .values()
            .filter(|m| matches!(m.domain.as_str(), "item" | "misc" | "abyss_jewel") && matches!(m.generation_type.as_str(), "prefix" | "suffix"))
            .flat_map(|m| m.spawn_weights.iter().filter(|w| w.weight > 0).map(|w| w.tag.clone()))
            .collect()
    })
}

type GroupKey = (usize, String, BTreeSet<String>, BTreeSet<String>);

struct GroupCache {
    groups: HashMap<GroupKey, Arc<BaseGroup>>,
    /// Keeps the databases the keys point at alive, so an address is never reused.
    _dbs: Vec<Arc<data::BaseItems>>,
}

/// The bases sharing this item's mod pool: same class, same spawn-relevant tags (influences included).
pub fn group(db: &ModDB, m: &ItemMatch) -> Arc<BaseGroup> {
    static CACHE: OnceLock<Mutex<GroupCache>> = OnceLock::new();
    let used = spawn_tags();
    let signature: BTreeSet<String> = m.tags.intersection(used).cloned().collect();
    let item_class = m.item_class.clone().unwrap_or_default();
    let key = (Arc::as_ptr(&db.bases) as usize, item_class, signature, m.item.influences.clone());
    let cache = CACHE.get_or_init(|| Mutex::new(GroupCache { groups: HashMap::new(), _dbs: Vec::new() }));
    if let Some(g) = cache.lock().unwrap().groups.get(&key) {
        return g.clone();
    }
    let made = Arc::new(make_group(db, m, &key.2, &key.3));
    let mut c = cache.lock().unwrap();
    if !c._dbs.iter().any(|d| Arc::ptr_eq(d, &db.bases)) {
        c._dbs.push(db.bases.clone());
    }
    c.groups.entry(key).or_insert(made).clone()
}

fn make_group(db: &ModDB, m: &ItemMatch, signature: &BTreeSet<String>, influences: &BTreeSet<String>) -> BaseGroup {
    let used = spawn_tags();
    let item_class = m.item_class.clone().unwrap_or_default();
    let base_id = m.base_id.as_deref().expect("group needs a known base");
    let domain = &db.bases[base_id].domain; // jewels and abyss jewels have their own
    let mut ranked: Vec<(bool, bool, i64, String)> = Vec::new();
    for (id, base) in db.bases.iter() {
        if base.item_class != item_class || base.domain != *domain || base.release_state != "released" {
            continue;
        }
        let tags: BTreeSet<String> = db.item_tags(id, influences).intersection(used).cloned().collect();
        if tags == *signature {
            let has = |t: &str| base.tags.iter().any(|x| x == t);
            // Best first: real drops over experimental bases, end-game bases, then highest drop level.
            ranked.push((!has("experimental_base"), has("top_tier_base_item_type"), base.drop_level, base.name.clone()));
        }
    }
    ranked.sort_by(|a, b| b.cmp(a));
    let mut names: Vec<String> = Vec::new();
    for (.., n) in ranked {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    if names.is_empty() {
        names.push(m.item.base.clone());
    }
    let mut label = if names.len() == 1 {
        names[0].clone()
    } else {
        match signature.iter().find(|t| defence_tag().is_match(t)) {
            Some(t) => format!("{item_class} ({})", defence(t)),
            None => item_class.clone(),
        }
    };
    if !influences.is_empty() {
        let mut titled: Vec<String> = influences.iter().map(|i| title(i)).collect();
        titled.sort();
        label += &format!(" · {}", titled.join("/"));
    }
    BaseGroup { item_class, signature: signature.clone(), label, bases: names }
}

/// Python's `str.title()`.
fn title(s: &str) -> String {
    let mut out = String::new();
    let mut prev_letter = false;
    for c in s.chars() {
        if c.is_alphabetic() {
            if prev_letter {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            prev_letter = true;
        } else {
            out.push(c);
            prev_letter = false;
        }
    }
    out
}

fn defence(tag: &str) -> String {
    let caps = defence_tag().captures(tag).expect("a defence tag");
    caps[1].split('_').map(|p| defence_name(p).unwrap_or("?")).collect::<Vec<_>>().join("/")
}

/// What else a finished item needs (ladder archetypes): `{"families": {family: worst tier},
/// "sides": [prefix, suffix] filler count, "order": families commonest first}`.
struct Finish {
    families: HashMap<String, i64>,
    sides: (usize, usize),
    order: Vec<String>,
}

impl Finish {
    fn from_json(v: &Value) -> Finish {
        let families = v["families"].as_object().map(|o| o.iter().map(|(k, t)| (k.clone(), t.as_i64().unwrap_or(0))).collect()).unwrap_or_default();
        let side = |i: usize| v["sides"][i].as_u64().unwrap_or(0) as usize;
        let order = v["order"].as_array().map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect()).unwrap_or_default();
        Finish { families, sides: (side(0), side(1)), order }
    }
}

/// Options of [`build`].
#[derive(Clone, Copy, Debug, Default)]
pub struct BuildOpts {
    /// Accept this many tiers worse than the item's own.
    pub settle: i64,
    /// Craft at this level; `None` uses the item's (raised to what its mods need).
    pub item_level: Option<i64>,
    /// Ask for every mod as it is, resistances included, instead of filler.
    pub exact: bool,
}

/// The craft for a PoB rare. Its resistances become filler ("any N resistances at
/// T3+") unless `exact`, which asks for every mod as it is.
pub fn build(db: &ModDB, m: &ItemMatch, pricing: Pricing, opts: BuildOpts) -> Result<GearProblem> {
    let (settle, exact) = (opts.settle, opts.exact);
    let Some(base_id) = m.base_id.as_deref() else {
        return Err(anyhow!("unknown base {:?}", m.item.base));
    };
    let item_class = m.item_class.clone().unwrap_or_default();
    let item_level = opts.item_level.filter(|&l| l != 0);
    let mut ilvl = item_level.or(m.item.item_level.filter(|&l| l != 0)).unwrap_or(DEFAULT_ITEM_LEVEL);
    if item_level.is_none() {
        // PoB items are often theorycraft: raise the level to what their mods need
        ilvl = m.mods.iter().filter(|x| x.source == "natural").map(|x| db.mods[&x.mod_id].required_level).fold(ilvl, i64::max);
    }
    let mods = pool(db, base_id, &m.tags, ilvl);
    let dmod = |id: &str| &db.mods[id];
    let tier_of = |id: &str| db.tier(id, &m.tags).unwrap_or(99);

    let mut targets: Vec<Target> = Vec::new();
    let mut fixed: Vec<Mod> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    let mut extra: BTreeMap<String, Mod> = BTreeMap::new();
    let mut bench: Vec<(String, Cost)> = Vec::new();
    let mut rows: Vec<Row> = Vec::new();
    let mut bench_ids: Vec<String> = Vec::new();
    let mut filler_mods: Vec<(&ponder_game::mods::ModMatch, String)> = Vec::new();
    let mut reserve: BTreeMap<Side, usize> = BTreeMap::new();
    let mut has_multimod = false;
    for x in &m.mods {
        let text = x.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" / ");
        let kind = side_of(&x.kind);
        let tier = x.tier.filter(|&t| t != 0);
        if !exact && !x.fractured() && x.source == "natural" && tier.is_some() && is_filler_family(&x.family) {
            filler_mods.push((x, text));
        } else if x.fractured() {
            fixed.push(to_mod(&x.mod_id, dmod(&x.mod_id), 0));
            rows.push(Row::new("fractured", kind, text, tier.map(|t| format!("T{t}")).unwrap_or_default()));
        } else if let (true, Some(tier)) = (x.source == "natural", tier) {
            let good: Vec<&Mod> = mods.iter().filter(|y| dmod(&y.id).type_ == x.family && y.side == kind && tier_of(&y.id) <= tier + settle).collect();
            if good.is_empty() {
                let level = dmod(&x.mod_id).required_level;
                skipped.push(format!("needs ilvl {level}: {text}"));
                rows.push(Row::new("skipped", kind, text, format!("ilvl {level}")));
                continue;
            }
            let worst = tier + settle;
            rows.push(Row::new("target", kind, text.clone(), if worst > 1 { format!("T{worst}+") } else { "T1".to_string() }));
            let groups = good.iter().flat_map(|y| y.groups.iter().cloned()).collect();
            let name = if worst > 1 { format!("{} (T{worst}+)", short(&text, 34)) } else { format!("{} (T1)", short(&text, 34)) };
            targets.push(Target { name, side: kind, good: good.iter().map(|y| y.id.clone()).collect(), groups, notable: false });
        } else if x.source == "essence" {
            extra.insert(x.mod_id.clone(), to_mod(&x.mod_id, dmod(&x.mod_id), 0));
            targets.push(Target {
                name: format!("{} (essence)", short(&text, 34)),
                side: kind,
                good: BTreeSet::from([x.mod_id.clone()]),
                groups: dmod(&x.mod_id).groups.iter().cloned().collect(),
                notable: false,
            });
            rows.push(Row::new("essence", kind, text, "essence"));
        } else if x.source == "crafted"
            && let Some(cost) = actions::bench_cost(&item_class, &x.mod_id)
        {
            bench.push((text.clone(), cost));
            bench_ids.push(x.mod_id.clone());
            rows.push(Row::new("bench", kind, text, "bench"));
            *reserve.entry(kind).or_insert(0) += 1;
            has_multimod = has_multimod || x.mod_id == actions::MULTIMOD;
        } else {
            skipped.push(format!("{}: {text}", x.source));
            rows.push(Row::new("skipped", kind, text, x.source));
        }
    }
    if bench.len() > 1
        && !has_multimod
        && let Some(cost) = actions::bench_cost(&item_class, actions::MULTIMOD)
    {
        bench.insert(0, ("Can have up to 3 Crafted Modifiers".into(), cost));
        bench_ids.insert(0, actions::MULTIMOD.into());
        rows.push(Row::new("bench", Side::Suffix, "Can have up to 3 Crafted Modifiers", "bench"));
        *reserve.entry(Side::Suffix).or_insert(0) += 1;
    }
    let mut filler: BTreeSet<String> = BTreeSet::new();
    let mut filler_sides: Option<(usize, usize)> = None;
    let mut families: Vec<String> = filler_mods.iter().map(|(x, _)| x.family.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    let filler_need;
    let finish = m.finish.as_ref().filter(|f| f.as_object().is_some_and(|o| !o.is_empty()));
    if let (Some(spec), false) = (finish, exact) {
        let spec = Finish::from_json(spec);
        let (f, mut sides, fam) = finish_filler(db, m, &spec, &mods, &mut rows);
        filler = f;
        families = fam;
        if bench.is_empty()
            && let Some((side, mod_id, text, cost)) = bench_filler(db, &item_class, &spec.families, sides)
        {
            bench.push((text.clone(), cost));
            bench_ids.push(mod_id);
            *reserve.entry(side).or_insert(0) += 1;
            sides = (sides.0 - (side == Side::Prefix) as usize, sides.1 - (side == Side::Suffix) as usize);
            let row = Row::new("bench", side, text, "bench");
            match rows.iter().position(|r| r.role == "filler" && r.side == side) {
                Some(i) => rows[i] = row,
                None => rows.push(row),
            }
        }
        filler_sides = Some(sides);
        filler_need = sides.0 + sides.1;
    } else if !filler_mods.is_empty() {
        let worst = FILLER_TIER.max(filler_mods.iter().map(|(x, _)| x.tier.unwrap_or(0)).max().unwrap_or(0) + settle);
        filler = mods.iter().filter(|y| is_filler_family(&dmod(&y.id).type_) && tier_of(&y.id) <= worst).map(|y| y.id.clone()).collect();
        for (x, text) in &filler_mods {
            rows.push(Row::new("filler", side_of(&x.kind), text.clone(), format!("any res T{worst}+")));
        }
        filler_need = filler_mods.len();
    } else {
        filler_need = 0;
    }
    let (harvest, h_profiles) = actions::harvest(&mods);
    let (fossil, f_profiles) = fossils(&mods, &targets, &filler, filler_need, &item_class, pricing);
    let (essence, forced) = actions::essences(&item_class, ilvl, 6);
    for (_, mod_id) in &forced {
        extra.entry(mod_id.clone()).or_insert_with(|| to_mod(mod_id, dmod(mod_id), 0));
    }
    let mut all = actions::basic();
    all.extend(harvest.iter().cloned());
    all.extend(fossil);
    all.extend(essence);
    all.extend(actions::metamods(&item_class, &lock_rerolls(&harvest, &h_profiles, &targets, &filler)));
    all.extend(actions::eldritch(&item_class, &m.item.influences));
    let need = targets.len();
    let mut problem = Problem::new(mods, targets, need, all, kind_of(&item_class));
    problem.fixed = fixed;
    problem.profiles = h_profiles.into_iter().chain(f_profiles).collect();
    problem.extra_mods = extra;
    problem.reserve = reserve;
    problem.filler = filler;
    problem.filler_need = filler_need;
    problem.filler_sides = filler_sides;
    Ok(GearProblem { problem, item_level: ilvl, skipped, bench, rows, bench_ids, filler_families: families })
}

fn kind_of(item_class: &str) -> Kind {
    if JEWEL_CLASSES.contains(&item_class) { Kind::Jewel } else { Kind::Gear }
}

/// Filler from `match.finish`: any of its families at its tier or better, so many per side.
fn finish_filler(db: &ModDB, m: &ItemMatch, spec: &Finish, mods: &[Mod], rows: &mut Vec<Row>) -> (BTreeSet<String>, (usize, usize), Vec<String>) {
    let ty = |id: &str| &db.mods[id].type_;
    let filler: BTreeSet<String> = mods
        .iter()
        .filter(|x| spec.families.get(ty(&x.id)).is_some_and(|&worst| db.tier(&x.id, &m.tags).unwrap_or(99) <= worst))
        .map(|x| x.id.clone())
        .collect();
    for (side, n) in [(Side::Prefix, spec.sides.0), (Side::Suffix, spec.sides.1)] {
        let on_side: Vec<&str> = spec
            .order
            .iter()
            .map(String::as_str)
            .chain(FILLER_FAMILIES)
            .filter(|f| mods.iter().any(|x| x.side == side && filler.contains(&x.id) && ty(&x.id) == f))
            .collect();
        if on_side.is_empty() {
            continue;
        }
        let mut names: Vec<String> = Vec::new();
        for f in &on_side {
            let name = family_name(db, f, mods);
            if !names.contains(&name) {
                names.push(name);
            }
        }
        let more = if names.len() > 4 { format!(" +{} more", names.len() - 4) } else { String::new() };
        let shown = format!("{}{more}", names[..names.len().min(4)].join(", "));
        for _ in 0..n {
            rows.push(Row::new("filler", side, format!("any of: {shown}"), "filler"));
        }
    }
    let on = |side: Side, n: usize| if mods.iter().any(|x| x.side == side && filler.contains(&x.id)) { n } else { 0 };
    let sides = (on(Side::Prefix, spec.sides.0), on(Side::Suffix, spec.sides.1));
    let families = spec.order.iter().take(2).cloned().collect();
    (filler, sides, families)
}

/// The bench craft that fills one open filler slot, suffix side first: (side, mod id, text, cost).
fn bench_filler(db: &ModDB, item_class: &str, families: &HashMap<String, i64>, need: (usize, usize)) -> Option<(Side, String, String, Cost)> {
    let options = loaded(data::crafting_bench_options());
    for (side, n) in [(Side::Suffix, need.1), (Side::Prefix, need.0)] {
        if n == 0 {
            continue;
        }
        let mut best: Option<&str> = None;
        for option in options.iter() {
            let Some(mod_id) = option.add_explicit_mod() else { continue };
            let Some(m) = db.mods.get(mod_id) else { continue };
            if !families.contains_key(&m.type_) || m.generation_type != side.name() || !option.item_classes.iter().any(|c| c == item_class) {
                continue;
            }
            if best.is_none_or(|b| m.required_level > db.mods[b].required_level) {
                best = Some(mod_id);
            }
        }
        if let Some(id) = best
            && let Some(cost) = actions::bench_cost(item_class, id)
        {
            let text = db.mods[id].text_or_empty().lines().next().unwrap_or("").to_string();
            return Some((side, id.to_string(), text, cost));
        }
    }
    None
}

fn family_name(db: &ModDB, family: &str, mods: &[Mod]) -> String {
    if is_filler_family(family) {
        return "res".into();
    }
    static NUMBER: OnceLock<Regex> = OnceLock::new();
    let re = NUMBER.get_or_init(|| Regex::new(r"\(?-?\d+(\.\d+)?(-\d+(\.\d+)?)?\)?").unwrap());
    let m = mods.iter().find(|x| db.mods[&x.id].type_ == family).expect("a mod of the family");
    short(&re.replace_all(&m.label, "#"), 28)
}

/// A craft of `targets` on a plain base, with every action `build` offers (no bench, no filler).
pub fn problem_on(
    db: &ModDB,
    base_id: &str,
    targets: Vec<Target>,
    item_level: i64,
    influences: &BTreeSet<String>,
    junk_cap: Option<(usize, usize)>,
    pricing: Pricing,
) -> Problem {
    let item_class = db.bases[base_id].item_class.as_str();
    let tags = db.item_tags(base_id, influences);
    let mods = pool(db, base_id, &tags, item_level);
    let (harvest, h_profiles) = actions::harvest(&mods);
    let none = BTreeSet::new();
    let (fossil, f_profiles) = fossils(&mods, &targets, &none, 0, item_class, pricing);
    let (essence, forced) = actions::essences(item_class, item_level, 6);
    let extra = forced.iter().map(|(_, id)| (id.clone(), to_mod(id, &db.mods[id.as_str()], 0))).collect();
    let mut all = actions::basic();
    all.extend(harvest.iter().cloned());
    all.extend(fossil);
    all.extend(essence);
    all.extend(actions::metamods(item_class, &lock_rerolls(&harvest, &h_profiles, &targets, &none)));
    all.extend(actions::eldritch(item_class, influences));
    let need = targets.len();
    let mut problem = Problem::new(mods, targets, need, all, kind_of(item_class));
    problem.profiles = h_profiles.into_iter().chain(f_profiles).collect();
    problem.extra_mods = extra;
    problem.junk_cap = junk_cap;
    problem
}

fn fossils(mods: &[Mod], targets: &[Target], filler: &BTreeSet<String>, filler_need: usize, item_class: &str, pricing: Pricing) -> (Vec<Action>, Profiles) {
    if !FOSSIL_COMBOS {
        return actions::fossils(mods);
    }
    let mut wants: Vec<&BTreeSet<String>> = targets.iter().map(|t| &t.good).collect();
    wants.extend(std::iter::repeat_n(filler, filler_need));
    // half the mods a rare rolls
    let per_side = if JEWEL_CLASSES.contains(&item_class) { 1.6 } else { 2.3 };
    actions::fossil_combos(mods, &wants, pricing.prices, pricing.settings, actions::FossilOpts { mods_per_side: per_side, ..Default::default() })
}

/// Harvest reforges worth pairing with a side lock: their tag can hit a target or filler mod.
fn lock_rerolls(harvest: &[Action], profiles: &Profiles, targets: &[Target], filler: &BTreeSet<String>) -> Vec<Action> {
    harvest
        .iter()
        .filter(|h| {
            let profile = &profiles[h.first_profile.as_deref().expect("harvest names its profile")];
            targets.iter().any(|t| t.good.iter().any(|m| profile.contains_key(m))) || filler.iter().any(|m| profile.contains_key(m))
        })
        .cloned()
        .collect()
}

/// The craft and its cheapest plan.
pub fn cost(db: &ModDB, m: &ItemMatch, pricing: Pricing, opts: BuildOpts) -> Result<(GearProblem, Plan)> {
    let mut gp = build(db, m, pricing, opts)?;
    let plan = solve(&mut gp.problem, pricing.prices, pricing.settings)?;
    Ok((gp, plan))
}

pub fn action_cost_of(cost: &[(String, f64)], prices: &Prices) -> f64 {
    cost.iter().map(|(pid, qty)| prices.get(pid).copied().unwrap_or(0.0) * qty).sum()
}

fn to_mod(mod_id: &str, m: &data::Mod, weight: i64) -> Mod {
    let text = m.text_or_empty();
    Mod {
        id: mod_id.to_string(),
        side: side_of(&m.generation_type),
        weight: weight as f64,
        groups: m.groups.clone(),
        notable: false,
        label: if text.is_empty() { mod_id.to_string() } else { text.lines().next().unwrap_or("").to_string() },
        tags: m.implicit_tags.clone(),
    }
}

/// `text` cut to `width` characters, ending in an ellipsis when it was longer.
pub fn short(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        text.to_string()
    } else {
        text.chars().take(width - 1).collect::<String>() + "…"
    }
}
