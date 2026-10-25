//! Turn a matched rare item into a crafting problem.
//!
//! Each natural mod on the item becomes a target that any tier at least as good
//! satisfies (`settle` loosens that by N tiers). Fractured mods are kept fixed.
//! Essence-only mods become targets only an essence can hit. Crafted mods the
//! bench still offers become a last bench step: the plan keeps a slot open on
//! their side and pays the bench cost. Veiled and other special mods are left
//! for the user.

use crate::memo::{DatabaseKey, Memo};

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use ponder_core::{Action, Kind, Mod, Plan, Prices, Problem, Settings, Side, Target, solve};
use ponder_game::data;
use ponder_game::mods::{ItemMatch, ModDB};
use regex::Regex;

use crate::actions::{self, Cost, Profiles, loaded};

mod bases;
mod catalog;

pub use bases::{BaseGroup, base_limits, defence_name, group, pool, shared_pool, spawn_tags};
use catalog::{CatalogOptions, configure_actions, mods_conflict};

pub const JEWEL_CLASSES: [&str; 2] = ["Jewel", "AbyssJewel"];
pub const DEFAULT_ITEM_LEVEL: i64 = 84;
/// Resistances are filler: buyers want enough of them, rarely a particular one. Sorted, like Python's `sorted(FILLER_FAMILIES)`.
pub const FILLER_FAMILIES: [&str; 3] = ["ColdResistance", "FireResistance", "LightningResistance"];
pub const FILLER_TIER: i64 = 3;
/// 1-4 fossil resonators, pre-scored (`actions::fossil_combos`); false: single fossils only.
pub const FOSSIL_COMBOS: bool = true;

pub fn is_filler_family(family: &str) -> bool {
    FILLER_FAMILIES.contains(&family)
}

/// What the market charges: prices of every currency, and what an hour of clicking is worth.
#[derive(Clone, Copy)]
pub struct Pricing<'a> {
    pub prices: &'a Prices,
    pub settings: &'a Settings,
    price_key: blake3::Hash,
}

impl<'a> Pricing<'a> {
    pub fn new(prices: &'a Prices, settings: &'a Settings) -> Pricing<'a> {
        let sorted: BTreeMap<_, _> = prices.iter().collect();
        let price_key = blake3::hash(&serde_json::to_vec(&sorted).expect("price key"));
        Pricing { prices, settings, price_key }
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
    let mods = shared_pool(db, base_id, &m.tags, ilvl);
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
    let multiple_crafts = m.mods.iter().filter(|x| x.source == "crafted").count() > 1;
    for x in &m.mods {
        let text = x.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join(" / ");
        let kind = side_of(&x.kind);
        let tier = x.tier.filter(|&t| t != 0);
        if !exact && !x.fractured() && matches!(x.source, "natural" | "essence") && is_filler_family(&x.family) {
            filler_mods.push((x, text));
        } else if x.fractured() {
            fixed.push(to_mod(&x.mod_id, dmod(&x.mod_id), 0.0));
            rows.push(Row::new("fractured", kind, text, tier.map(|t| format!("T{t}")).unwrap_or_default()));
        } else if let (true, Some(tier)) = (x.source == "natural", tier) {
            let required = dmod(&x.mod_id);
            let good: Vec<&Mod> = mods
                .iter()
                .filter(|y| {
                    dmod(&y.id).type_ == x.family
                        && y.side == kind
                        && tier_of(&y.id) <= tier + settle
                        && y.groups.len() == required.groups.len()
                        && y.groups.iter().all(|g| required.groups.contains(g))
                })
                .collect();
            let mut good_ids: BTreeSet<String> = good.iter().map(|m| m.id.clone()).collect();
            if good_ids.is_empty() {
                good_ids.insert(x.mod_id.clone());
                extra.insert(x.mod_id.clone(), to_mod(&x.mod_id, required, 0.0));
            }
            let worst = tier + settle;
            rows.push(Row::new("target", kind, text.clone(), if worst > 1 { format!("T{worst}+") } else { "T1".to_string() }));
            let groups = required.groups.iter().cloned().collect();
            let name = if worst > 1 { format!("{} (T{worst}+)", short(&text, 34)) } else { format!("{} (T1)", short(&text, 34)) };
            targets.push(Target { name, side: kind, good: good_ids, groups, notable: false });
        } else if matches!(x.source, "natural" | "essence" | "veiled") || (x.source == "crafted" && !multiple_crafts) {
            extra.insert(x.mod_id.clone(), to_mod(&x.mod_id, dmod(&x.mod_id), 0.0));
            targets.push(Target {
                name: format!("{} or better", short(&text, 34)),
                side: kind,
                good: BTreeSet::from([x.mod_id.clone()]),
                groups: dmod(&x.mod_id).groups.iter().cloned().collect(),
                notable: false,
            });
            rows.push(Row::new("target", kind, text, "or better"));
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
    let families: Vec<String> = filler_mods.iter().map(|(x, _)| x.family.clone()).collect::<BTreeSet<_>>().into_iter().collect();
    let filler_need = filler_mods.len();
    let worst = FILLER_TIER + settle;
    let filler = if filler_need == 0 {
        BTreeSet::new()
    } else {
        mods.iter()
            .filter(|y| is_filler_family(&dmod(&y.id).type_) && tier_of(&y.id) <= worst)
            .filter(|y| bench_ids.iter().all(|id| !dmod(id).groups.iter().any(|g| y.groups.contains(g))))
            .map(|y| y.id.clone())
            .collect()
    };
    for _ in 0..filler_need {
        rows.push(Row::new("target", Side::Suffix, "Elemental resistance", format!("T{worst} or better")));
    }
    let need = targets.len();
    let mut problem = ponder_core::ProblemSpec::from_shared_pool(mods, targets, need, Vec::new(), kind_of(&item_class));
    problem.affix_limits = Some(base_limits(db, base_id));
    problem.fixed = fixed;
    problem.influences = m.item.influences.clone();
    problem.extra_mods = extra;
    problem.reserve = reserve;
    problem.bench = bench_ids.iter().map(|id| to_mod(id, dmod(id), 0.0)).filter(|b| mods_conflict(&problem.pool, b)).collect();
    problem.filler_need = filler_need + problem.fixed.iter().filter(|m| filler.contains(&m.id)).count();
    problem.filler = filler;
    problem.filler_label = "elemental resistance".into();
    configure_actions(db, &mut problem, base_id, ilvl, pricing, CatalogOptions::Product);
    if let Some(target) = problem.targets.iter().find(|t| t.good.is_empty()) {
        return Err(anyhow!("the bench crafts conflict with {}", target.name));
    }
    if problem.targets.len() + problem.bench.len() > ponder_core::MAX_TARGETS {
        return Err(anyhow!("too many target and bench modifier groups"));
    }
    problem.filler_sides = (problem.filler_need > 0).then_some((0, problem.filler_need));
    Ok(GearProblem { problem: problem.compile(), item_level: ilvl, skipped, bench, rows, bench_ids, filler_families: families })
}

fn kind_of(item_class: &str) -> Kind {
    if JEWEL_CLASSES.contains(&item_class) { Kind::Jewel } else { Kind::Gear }
}

pub fn problem_spec_on(
    db: &ModDB,
    base_id: &str,
    targets: Vec<Target>,
    item_level: i64,
    influences: &BTreeSet<String>,
    junk_cap: Option<(usize, usize)>,
    pricing: Pricing,
) -> ponder_core::ProblemSpec {
    let item_class = db.bases[base_id].item_class.as_str();
    let tags = db.item_tags(base_id, influences);
    let mods = shared_pool(db, base_id, &tags, item_level);
    let need = targets.len();
    let mut problem = ponder_core::ProblemSpec::from_shared_pool(mods, targets, need, Vec::new(), kind_of(item_class));
    problem.affix_limits = Some(base_limits(db, base_id));
    problem.junk_cap = junk_cap;
    problem.influences = influences.clone();
    configure_actions(db, &mut problem, base_id, item_level, pricing, CatalogOptions::Component);
    problem
}

#[derive(Default)]
pub struct ComponentGoal {
    pub targets: Vec<Target>,
    pub filler: BTreeSet<String>,
    pub filler_need: usize,
}

pub fn component_problem_on(
    db: &ModDB,
    base_id: &str,
    goal: ComponentGoal,
    item_level: i64,
    influences: &BTreeSet<String>,
    pricing: Pricing,
) -> Problem {
    let ComponentGoal { mut targets, filler, filler_need } = goal;
    targets.sort_by(|a, b| (&a.side, &a.good, &a.groups, a.notable, &a.name).cmp(&(&b.side, &b.good, &b.groups, b.notable, &b.name)));
    static CACHE: Memo<(DatabaseKey, blake3::Hash), Problem> = Memo::new();
    let key = blake3::hash(
        &serde_json::to_vec(&(
            base_id,
            &targets,
            item_level,
            influences,
            &filler,
            filler_need,
            pricing.price_key.as_bytes(),
            pricing.settings.chaos_per_hour,
        ))
        .expect("component key"),
    );
    CACHE.get_or_init((DatabaseKey::new(db), key), || {
        let mut spec = problem_spec_on(db, base_id, targets, item_level, influences, Some((0, 0)), pricing);
        spec.filler = filler;
        spec.filler_need = filler_need;
        spec.compile()
    })
}

pub fn relevel_problem(db: &ModDB, m: &ItemMatch, original: &Problem, item_level: i64, pricing: Pricing) -> Problem {
    let base_id = m.base_id.as_deref().expect("a matched base");
    let mods = shared_pool(db, base_id, &m.tags, item_level);
    let expand = |good: &mut BTreeSet<String>| {
        let mut tiers: HashMap<String, i64> = HashMap::new();
        for id in good.iter() {
            if ModDB::generation_weight(&db.mods[id], &m.tags) > 0.0
                && let Some(tier) = db.tier(id, &m.tags)
            {
                tiers.entry(db.mods[id].type_.clone()).and_modify(|t| *t = (*t).max(tier)).or_insert(tier);
            }
        }
        for mod_ in mods.iter() {
            if tiers.get(&db.mods[&mod_.id].type_).is_some_and(|&worst| db.tier(&mod_.id, &m.tags).is_some_and(|t| t <= worst)) {
                good.insert(mod_.id.clone());
            }
        }
    };
    let mut problem = original.to_spec();
    for target in &mut problem.targets {
        expand(&mut target.good);
    }
    expand(&mut problem.filler);
    problem.pool = mods;
    configure_actions(db, &mut problem, base_id, item_level, pricing, CatalogOptions::Product);
    problem.compile()
}

/// The craft and its cheapest plan.
pub fn cost(db: &ModDB, m: &ItemMatch, pricing: Pricing, opts: BuildOpts) -> Result<(GearProblem, Plan)> {
    let gp = build(db, m, pricing, opts)?;
    let plan = solve(&gp.problem, pricing.prices, pricing.settings)?;
    Ok((gp, plan))
}

pub fn action_cost_of(cost: &[(String, f64)], prices: &Prices) -> f64 {
    cost.iter().map(|(pid, qty)| prices.get(pid).copied().unwrap_or(0.0) * qty).sum()
}

fn to_mod(mod_id: &str, m: &data::Mod, weight: f64) -> Mod {
    let text = m.text_or_empty();
    Mod {
        id: mod_id.to_string(),
        side: side_of(&m.generation_type),
        weight,
        groups: m.groups.clone(),
        notable: false,
        label: if text.is_empty() { mod_id.to_string() } else { text.lines().next().unwrap_or("").to_string() },
        tags: m.implicit_tags.clone(),
    }
}

/// `text` cut to `width` characters, ending in an ellipsis when it was longer.
pub fn short(text: &str, width: usize) -> String {
    if text.chars().count() <= width { text.to_string() } else { text.chars().take(width - 1).collect::<String>() + "…" }
}
