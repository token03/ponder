//! The data types: estimates, fractures and the reports built from them.

use std::collections::BTreeSet;

use ponder_core::{self as craft, Plan, Problem, SelfFracture, Settings, Side};
use ponder_net::trade::{ItemQuery, Stat};
use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::{Value, json};

use super::finish::Finish;
use crate::gear::Row;
use crate::mine::Candidate;

pub const JEWELLERY: [&str; 3] = ["Ring", "Amulet", "Belt"]; // one implicit apart, their bases sell alike: search by category

/// Python's `x or y` on optional strings: empty counts as missing.
fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().filter(|s| !s.is_empty())
}

/// What identifies the same craft across builds (Python's `Estimate.key` tuple).
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum Key {
    #[default]
    None,
    Cluster {
        size: String,
        tag: String,
        notables: Vec<String>,
        passives: String,
        fractured: Vec<String>,
    },
    Gear {
        item_class: String,
        signature: BTreeSet<String>,
        targets: Vec<(Side, Vec<String>)>,
        fixed: Vec<String>,
        bench: Vec<String>,
        item_level: i64,
        filler_need: usize,
        filler: BTreeSet<String>,
        filler_sides: Option<(usize, usize)>,
    },
}

impl Key {
    /// `key[:3] == ("cluster", size, tag)`
    pub fn is_cluster(&self, size: &str, tag: &str) -> bool {
        matches!(self, Key::Cluster { size: s, tag: t, .. } if s == size && t == tag)
    }
}

/// How an estimate's bases are searched for: a cluster's size/enchant/passives, or a gear base or category.
#[derive(Clone, Debug, PartialEq, Default)]
pub enum Search {
    #[default]
    None,
    Base(String),
    Category(Option<String>),
    Cluster { size: String, enchant: Option<String>, passives: i64 },
}

impl Search {
    /// `item_query(**search)`: the base or category goes into the query.
    pub fn apply(&self, q: &mut ItemQuery) {
        match self {
            Search::Base(b) => q.base = Some(b.clone()),
            Search::Category(c) => q.category = c.clone(),
            _ => {}
        }
    }

    pub fn to_value(&self) -> Value {
        match self {
            Search::None => json!({}),
            Search::Base(b) => json!({"base": b}),
            Search::Category(c) => json!({"category": c}),
            Search::Cluster { size, enchant, passives } => json!({"size": size, "enchant": enchant, "passives": passives}),
        }
    }
}

impl Serialize for Search {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_value().serialize(s)
    }
}

/// A recombinator plan, as far as an estimate needs it: its cost and its JSON.
#[derive(Clone, Debug, PartialEq)]
pub struct RecombInfo {
    pub cost: Option<f64>,
    pub json: Value,
}

/// What an estimate says about another in the product tree.
#[derive(Clone, Debug, PartialEq)]
pub struct ProductRef {
    pub title: String,
    pub targets: Vec<String>,
    pub cheapest: Option<f64>,
}

impl ProductRef {
    fn to_value(&self) -> Value {
        json!({"title": self.title, "targets": self.targets, "cheapest": self.cheapest})
    }
}

/// Buy a base with one target already fractured on it, then craft the rest.
#[derive(Clone, Debug, Default)]
pub struct Fracture {
    pub target: String,
    pub mod_id: String,
    pub problem: Option<Problem>,
    pub plan: Option<Plan>,
    pub query: Option<Value>,
    pub trade_url: Option<String>,
    pub base_price: Option<(f64, i64)>,
    pub base_error: Option<String>,
    pub error: Option<String>,
    /// fracturing the mod yourself, when that's possible
    pub made: Option<SelfFracture>,
}

impl Fracture {
    pub fn new(target: impl Into<String>, mod_id: impl Into<String>, problem: Problem) -> Fracture {
        Fracture { target: target.into(), mod_id: mod_id.into(), problem: Some(problem), ..Default::default() }
    }

    pub fn craft_cost(&self) -> Option<f64> {
        self.plan.as_ref().map(|p| p.cost)
    }

    /// The cheaper of buying the fractured base and making it.
    pub fn base_cost(&self) -> Option<f64> {
        [self.base_price.map(|b| b.0), self.made.as_ref().map(|m| m.cost)].into_iter().flatten().reduce(f64::min)
    }

    pub fn self_made(&self) -> bool {
        self.made.as_ref().is_some_and(|m| self.base_cost() == Some(m.cost))
    }

    pub fn total(&self) -> Option<f64> {
        Some(self.plan.as_ref()?.cost + self.base_cost()?)
    }

    pub fn to_dict(&self) -> Value {
        json!({
            "target": self.target,
            "craft_cost": self.craft_cost(),
            "base_price": self.base_price.map(|b| b.0),
            "base_listed": self.base_price.map(|b| b.1),
            "self_made": self.made,
            "total": self.total(),
            "trade_url": self.trade_url,
            "error": non_empty(&self.error).or(non_empty(&self.base_error)),
        })
    }
}

impl Serialize for Fracture {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_dict().serialize(s)
    }
}

/// Expected cost of crafting one item, and what it sells for when known.
#[derive(Clone, Debug)]
pub struct Estimate {
    /// "cluster" or "gear"
    pub kind: String,
    /// the base group, e.g. "Gloves (ES)", or "Medium cluster · 10% increased Area Damage · 4p"
    pub title: String,
    pub targets: Vec<String>,
    /// identifies the same craft across builds
    pub key: Key,
    /// the best base with this mod pool, which crafts and searches use, e.g. "Warlock Gloves"
    pub base: String,
    /// the base the build had, when different, e.g. "Sorcerer Gloves"
    pub used_base: String,
    /// how many bases share this mod pool
    pub bases: usize,
    pub slot: Option<String>,
    /// sources whose rares need this craft
    pub builds: Vec<String>,
    pub item_level: i64,
    pub problem: Option<Problem>,
    pub plan: Option<Plan>,
    /// every mod line, for display
    pub rows: Vec<Row>,
    /// (label, chaos) crafted last
    pub bench: Vec<(String, f64)>,
    /// mods the plan doesn't make
    pub skipped: Vec<String>,
    /// fractured mods the base must already have
    pub fixed: Vec<String>,
    pub fixed_ids: Vec<String>,
    /// (chaos, listings)
    pub base_price: Option<(f64, i64)>,
    /// "poe.ninja", "trade"
    pub base_source: String,
    /// trade search for the (fractured) base, when one is needed
    pub base_query: Option<Value>,
    pub base_url: Option<String>,
    pub base_error: Option<String>,
    /// trade search for the finished item
    pub query: Option<Value>,
    pub trade_url: Option<String>,
    /// (median of cheapest asks, total listed)
    pub market: Option<(f64, i64)>,
    pub market_error: Option<String>,
    pub fractures: Vec<Fracture>,
    /// e.g. ["ColdResistance", "FireResistance"]
    pub filler_families: Vec<String>,
    pub search: Search,
    /// why there is no plan
    pub error: Option<String>,
    /// ladder characters wearing it (ladder runs)
    pub demand: usize,
    /// the cheapest recombinator tree, when costed
    pub recomb: Option<RecombInfo>,
    /// their most common main skills
    pub skills: Vec<String>,
    /// products this one contains (ladder runs): what a brick sells as; indices into the list it was linked in
    pub smaller: Vec<usize>,
    /// products that contain this one: where it leads
    pub bigger: Vec<usize>,
    /// `smaller` and `bigger` as of the last `snapshot_products` (what `to_dict` writes)
    pub smaller_products: Vec<ProductRef>,
    pub bigger_products: Vec<ProductRef>,
    /// half-done items for sale worth finishing (`find_starts`)
    pub starts: Vec<Finish>,
    pub starts_url: Option<String>,
    /// the clicking rate its plan was solved at
    pub settings: Settings,
}

impl Estimate {
    pub fn new(kind: &str, title: impl Into<String>, targets: Vec<String>) -> Estimate {
        Estimate {
            kind: kind.into(),
            title: title.into(),
            targets,
            key: Key::None,
            base: String::new(),
            used_base: String::new(),
            bases: 1,
            slot: None,
            builds: Vec::new(),
            item_level: 84,
            problem: None,
            plan: None,
            rows: Vec::new(),
            bench: Vec::new(),
            skipped: Vec::new(),
            fixed: Vec::new(),
            fixed_ids: Vec::new(),
            base_price: None,
            base_source: String::new(),
            base_query: None,
            base_url: None,
            base_error: None,
            query: None,
            trade_url: None,
            market: None,
            market_error: None,
            fractures: Vec::new(),
            filler_families: Vec::new(),
            search: Search::None,
            error: None,
            demand: 0,
            recomb: None,
            skills: Vec::new(),
            smaller: Vec::new(),
            bigger: Vec::new(),
            smaller_products: Vec::new(),
            bigger_products: Vec::new(),
            starts: Vec::new(),
            starts_url: None,
            settings: Settings::default(),
        }
    }

    pub fn craft_cost(&self) -> Option<f64> {
        self.plan.as_ref().map(|p| p.cost)
    }

    pub fn bench_cost(&self) -> f64 {
        self.bench.iter().map(|(_, c)| c).sum()
    }

    /// A fractured base costs real money; without its price the total is incomplete.
    pub fn needs_base_price(&self) -> bool {
        !self.fixed.is_empty() && self.base_price.is_none()
    }

    pub fn total(&self) -> Option<f64> {
        let plan = self.plan.as_ref()?;
        Some(plan.cost + self.bench_cost() + self.base_price.map_or(0.0, |b| b.0))
    }

    pub fn margin(&self) -> Option<f64> {
        let market = self.market?;
        if self.total().is_none() || self.needs_base_price() {
            return None;
        }
        Some(market.0 - self.cheapest()?)
    }

    /// How much of a market hole this is: margin times ladder wearers per listing.
    ///
    /// Many wearers, few listings and a fat margin rank first; a crowded market
    /// (hundreds listed) shrinks even a good margin.
    pub fn hole(&self) -> Option<f64> {
        let margin = self.margin()?;
        if self.demand == 0 {
            return None;
        }
        Some(margin * self.demand as f64 / (self.market?.1 as f64 + 5.0))
    }

    pub fn best_fracture(&self) -> Option<&Fracture> {
        let mut best: Option<(&Fracture, f64)> = None;
        for f in &self.fractures {
            if let Some(t) = f.total()
                && best.is_none_or(|(_, b)| t < b)
            {
                best = Some((f, t));
            }
        }
        best.map(|b| b.0)
    }

    /// The most a fractured base is worth: crafting from a plain base (base included) less the cheapest finish from a fractured one.
    pub fn fracture_worth(&self) -> Option<(f64, &Fracture)> {
        let mut best: Option<&Fracture> = None;
        for f in self.fractures.iter().filter(|f| f.plan.is_some()) {
            if best.is_none_or(|b| f.craft_cost() < b.craft_cost()) {
                best = Some(f);
            }
        }
        let f = best?;
        Some((self.total()? - f.craft_cost()?, f))
    }

    /// Recombining clean donors instead of crafting in one go (then the same bench crafts).
    pub fn recomb_total(&self) -> Option<f64> {
        Some(self.recomb.as_ref()?.cost? + self.bench_cost())
    }

    /// The cheapest of crafting from scratch, the best priced fracture and recombining.
    pub fn cheapest(&self) -> Option<f64> {
        [self.total(), self.best_fracture().and_then(Fracture::total), self.recomb_total()].into_iter().flatten().reduce(f64::min)
    }

    pub fn recipe(&self, rows: usize) -> Vec<(String, String, f64)> {
        match (&self.problem, &self.plan) {
            (Some(p), Some(plan)) => craft::recipe(p, plan, 0.01).into_iter().take(rows).collect(),
            _ => Vec::new(),
        }
    }

    /// The plan from scratch as numbered steps (`craft::steps`).
    pub fn steps(&self) -> Vec<craft::Step> {
        match (&self.problem, &self.plan) {
            (Some(p), Some(plan)) => craft::steps(p, plan, 0.02),
            _ => Vec::new(),
        }
    }

    pub fn recipe_states(&self, rows: usize) -> Vec<(craft::State, String, f64)> {
        self.plan.as_ref().map_or_else(Vec::new, |plan| craft::recipe_states(plan, 0.01).into_iter().take(rows).collect())
    }

    /// Expected uses of each currency action, most used first (those used 0.05 times or more).
    pub fn usage(&self) -> Vec<(String, f64)> {
        let Some(plan) = &self.plan else { return Vec::new() };
        let mut v: Vec<(String, f64)> = plan.usage.iter().filter(|(_, n)| **n >= 0.05).map(|(a, n)| (a.clone(), *n)).collect();
        v.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then_with(|| a.0.cmp(&b.0)));
        v
    }

    /// This estimate as the other estimates see it.
    pub fn product(&self) -> ProductRef {
        ProductRef { title: self.title.clone(), targets: self.targets.clone(), cheapest: self.cheapest() }
    }

    /// Python's `to_dict`: plain JSON for the CLI and the apps.
    pub fn to_dict(&self) -> Value {
        let mods: Vec<Value> = self.rows.iter().map(|r| json!({"role": r.role, "side": r.side.name(), "text": r.text, "tier": r.tier})).collect();
        let usage: serde_json::Map<String, Value> = self.usage().into_iter().map(|(a, n)| (a, json!(n))).collect();
        let recipe: Vec<Value> = self.recipe(12).into_iter().map(|(state, action, share)| json!({"state": state, "action": action, "share": share})).collect();
        let fracture_worth = self.fracture_worth().map(|(chaos, f)| json!({"chaos": chaos, "target": f.target}));
        json!({
            "kind": self.kind,
            "title": self.title,
            "base": self.base,
            "build_base": if self.used_base.is_empty() { &self.base } else { &self.used_base },
            "bases_in_group": self.bases,
            "mods": mods,
            "targets": self.targets,
            "slot": self.slot,
            "builds": self.builds,
            "item_level": self.item_level,
            "craft_cost": self.craft_cost(),
            "bench": self.bench.iter().map(|(label, c)| json!({"mod": label, "chaos": c})).collect::<Vec<_>>(),
            "base_price": self.base_price.map(|b| b.0),
            "base_source": if self.base_source.is_empty() { None } else { Some(&self.base_source) },
            "base_url": self.base_url,
            "total": self.total(),
            "market": self.market.map(|m| m.0),
            "listed": self.market.map(|m| m.1),
            "margin": self.margin(),
            "currency": usage,
            "clicking_hours": self.plan.as_ref().map(|p| p.hours),
            "currency_chaos": self.plan.as_ref().map(|p| p.currency(&self.settings)),
            "recipe": recipe,
            "steps": self.steps(),
            "fractures": self.fractures.iter().map(Fracture::to_dict).collect::<Vec<_>>(),
            "not_crafted": self.skipped,
            "trade_url": self.trade_url,
            "recomb": self.recomb.as_ref().map(|r| r.json.clone()),
            "recomb_total": self.recomb_total(),
            "demand": self.demand,
            "skills": self.skills,
            "hole": self.hole(),
            "fracture_worth": fracture_worth,
            "smaller_products": self.smaller_products.iter().map(ProductRef::to_value).collect::<Vec<_>>(),
            "bigger_products": self.bigger_products.iter().map(ProductRef::to_value).collect::<Vec<_>>(),
            "half_done_for_sale": self.starts.iter().map(Finish::to_dict).collect::<Vec<_>>(),
            "error": non_empty(&self.error).or(non_empty(&self.market_error)).or(non_empty(&self.base_error)),
        })
    }

    /// `item_query` for this estimate's base (or category) with these stats.
    pub(crate) fn query_with(&self, stats: Vec<Stat>) -> ItemQuery {
        let mut q = ItemQuery { stats, ..ItemQuery::default() };
        self.search.apply(&mut q);
        q
    }
}

impl Serialize for Estimate {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_dict().serialize(s)
    }
}

/// Refresh every estimate's `smaller_products` and `bigger_products` from the current costs.
///
/// `to_dict` writes the snapshot, so call this after anything that changes costs (solving,
/// trade prices, recombinators). `ladder` and `run` do.
pub fn snapshot_products(estimates: &mut [Estimate]) {
    let products: Vec<ProductRef> = estimates.iter().map(Estimate::product).collect();
    for e in estimates.iter_mut() {
        e.smaller_products = e.smaller.iter().map(|&i| products[i].clone()).collect();
        e.bigger_products = e.bigger.iter().map(|&i| products[i].clone()).collect();
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct BuildInfo {
    pub source: String,
    /// e.g. "Occultist lvl 95: Winter Orb"
    pub name: String,
    pub rares: usize,
}

/// Where a shared combination's estimate lives in its report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Slot {
    /// `Report::estimates[i]`
    Main(usize),
    /// `Report::extra[i]`
    Extra(usize),
}

/// A mod combination several builds' rares have in common.
#[derive(Clone, Debug)]
pub struct Shared {
    pub candidate: Candidate,
    /// cluster combos are costed; gear combos aren't yet
    pub estimate: Option<Slot>,
}

#[derive(Clone, Debug)]
pub struct Report {
    pub builds: Vec<BuildInfo>,
    pub estimates: Vec<Estimate>,
    /// costed combinations that are not among `estimates`
    pub extra: Vec<Estimate>,
    pub shared: Vec<Shared>,
    pub prices_source: String,
    pub league: String,
    /// sources that failed to load
    pub errors: Vec<String>,
    /// chaos per divine, for display
    pub divine: f64,
}

impl Report {
    /// Best margin first, then crafts more builds need, then the dearest.
    pub fn ranked(&self) -> Vec<&Estimate> {
        let mut v: Vec<&Estimate> = self.estimates.iter().collect();
        v.sort_by(|a, b| cmp_priority(a, b));
        v
    }

    pub fn shared_estimate(&self, s: &Shared) -> Option<&Estimate> {
        match s.estimate? {
            Slot::Main(i) => self.estimates.get(i),
            Slot::Extra(i) => self.extra.get(i),
        }
    }

    pub fn to_dict(&self) -> Value {
        json!({
            "league": self.league,
            "prices": self.prices_source,
            "chaos_per_divine": self.divine,
            "builds": self.builds,
            "estimates": self.ranked().into_iter().map(Estimate::to_dict).collect::<Vec<_>>(),
            "shared": self.shared.iter().map(|s| {
                let c = &s.candidate;
                json!({
                    "item_class": c.item_class,
                    "mods": c.families,
                    "builds": c.builds,
                    "items": c.count,
                    "estimate": self.shared_estimate(s).map(Estimate::to_dict),
                })
            }).collect::<Vec<_>>(),
            "errors": self.errors,
        })
    }
}

impl Serialize for Report {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let Value::Object(map) = self.to_dict() else { unreachable!() };
        let mut m = s.serialize_map(Some(map.len()))?;
        for (k, v) in &map {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}

/// `priority`: best margin first, then crafts more builds need, then the dearest.
pub fn priority(e: &Estimate) -> (bool, f64, i64, bool, f64) {
    (e.margin().is_none(), -e.margin().unwrap_or(0.0), -(e.builds.len() as i64), e.total().is_none(), -e.total().unwrap_or(0.0))
}

fn cmp_f(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

/// Orders estimates by `priority`.
pub fn cmp_priority(a: &Estimate, b: &Estimate) -> std::cmp::Ordering {
    let (x, y) = (priority(a), priority(b));
    x.0.cmp(&y.0).then(cmp_f(x.1, y.1)).then(x.2.cmp(&y.2)).then(x.3.cmp(&y.3)).then(cmp_f(x.4, y.4))
}

#[derive(Clone, Debug)]
pub struct LadderReport {
    pub characters: usize,
    /// rare gear and jewels read
    pub items: usize,
    /// rare cluster jewels read
    pub clusters: usize,
    pub estimates: Vec<Estimate>,
    /// gear combinations found, before costing
    pub archetypes: usize,
    pub cluster_archetypes: usize,
    pub prices_source: String,
    pub league: String,
    pub divine: f64,
}

impl LadderReport {
    /// Market holes first (margin x wearers per listing), then unpriced ones by ladder demand.
    pub fn ranked(&self) -> Vec<&Estimate> {
        let mut v: Vec<&Estimate> = self.estimates.iter().collect();
        v.sort_by(|a, b| {
            (a.hole().is_none())
                .cmp(&b.hole().is_none())
                .then(cmp_f(-a.hole().unwrap_or(0.0), -b.hole().unwrap_or(0.0)))
                .then(b.demand.cmp(&a.demand))
        });
        v
    }

    pub fn to_dict(&self) -> Value {
        json!({
            "league": self.league,
            "prices": self.prices_source,
            "chaos_per_divine": self.divine,
            "characters": self.characters,
            "items": self.items,
            "clusters": self.clusters,
            "archetypes": self.archetypes,
            "cluster_archetypes": self.cluster_archetypes,
            "estimates": self.ranked().into_iter().map(Estimate::to_dict).collect::<Vec<_>>(),
        })
    }
}

impl Serialize for LadderReport {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let Value::Object(map) = self.to_dict() else { unreachable!() };
        let mut m = s.serialize_map(Some(map.len()))?;
        for (k, v) in &map {
            m.serialize_entry(k, v)?;
        }
        m.end()
    }
}
