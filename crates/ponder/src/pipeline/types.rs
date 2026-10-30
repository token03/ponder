use std::sync::Arc;

use ponder_net::trade::{ItemQuery, Stat};
use serde::{Serialize, Serializer};
use serde_json::{Value, json};

use crate::candidate::Candidate;
use crate::gear::{Row, Settings};
use crate::pattern::Pattern;
use crate::plan::Summary;

pub const JEWELLERY: [&str; 3] = ["Ring", "Amulet", "Belt"];

#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum Key {
    #[default]
    None,
    Cluster {
        size: String,
        tag: String,
        fingerprint: String,
    },
    Gear(String),
}

impl Key {
    pub fn is_cluster(&self, size: &str, tag: &str) -> bool {
        matches!(self, Self::Cluster { size: s, tag: t, .. } if s == size && t == tag)
    }
}

#[derive(Clone, Debug, PartialEq, Default, Serialize)]
#[serde(untagged)]
pub enum Search {
    #[default]
    None,
    Base {
        base: String,
    },
    Category {
        category: Option<String>,
    },
    Cluster {
        size: String,
        enchant: Option<String>,
        passives: i64,
    },
}

impl Search {
    pub fn apply(&self, query: &mut ItemQuery) {
        match self {
            Self::Base { base } => query.base = Some(base.clone()),
            Self::Category { category } => query.category = category.clone(),
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Estimate {
    pub kind: String,
    pub title: String,
    pub targets: Vec<String>,
    pub key: Key,
    pub base: String,
    pub used_base: String,
    pub bases: usize,
    pub slot: Option<String>,
    pub builds: Vec<String>,
    pub item_level: i64,
    pub candidate: Option<Candidate>,
    pub summary: Option<Arc<Summary>>,
    pub rows: Vec<Row>,
    pub fixed: Vec<String>,
    pub fixed_ids: Vec<String>,
    pub base_price: Option<(f64, i64)>,
    pub base_source: String,
    pub base_query: Option<Value>,
    pub base_url: Option<String>,
    pub base_error: Option<String>,
    pub query: Option<Value>,
    pub trade_url: Option<String>,
    pub market: Option<(f64, i64)>,
    pub market_error: Option<String>,
    pub search: Search,
    pub error: Option<String>,
    pub demand: usize,
    pub skills: Vec<String>,
    pub settings: Settings,
}

impl Estimate {
    pub fn new(kind: &str, title: impl Into<String>, targets: Vec<String>) -> Self {
        Self { kind: kind.into(), title: title.into(), targets, bases: 1, item_level: 84, ..Self::default() }
    }

    pub fn craft_cost(&self) -> Option<f64> {
        self.summary.as_ref().map(|plan| plan.cost)
    }
    pub fn total(&self) -> Option<f64> {
        Some(self.craft_cost()? + self.base_price?.0 * self.summary.as_ref()?.bases)
    }
    pub fn cheapest(&self) -> Option<f64> {
        self.total()
    }
    pub fn failure(&self) -> Option<&str> {
        self.error.as_deref().filter(|s| !s.is_empty())
    }
    pub fn clicking_hours(&self) -> Option<f64> {
        self.summary.as_ref().map(|plan| plan.hours)
    }
    pub fn currency_cost(&self) -> Option<f64> {
        Some(self.craft_cost()? - self.clicking_hours()? * self.settings.chaos_per_hour)
    }
    pub fn margin(&self) -> Option<f64> {
        Some(self.market?.0 - self.total()?)
    }
    pub fn hole(&self) -> Option<f64> {
        Some(self.margin()? * self.demand as f64 / self.market?.1.max(1) as f64)
    }

    pub fn query_with(&self, stats: Vec<Stat>) -> ItemQuery {
        let mut query = ItemQuery { stats, ..Default::default() };
        self.search.apply(&mut query);
        query
    }

    pub fn to_dict(&self) -> Value {
        json!({
            "kind": self.kind, "title": self.title, "targets": self.targets, "base": self.base,
            "used_base": self.used_base, "bases": self.bases, "item_level": self.item_level,
            "slot": self.slot, "builds": self.builds, "demand": self.demand, "skills": self.skills,
            "craft_cost": self.craft_cost(), "total": self.total(), "cheapest": self.cheapest(),
            "currency_chaos": self.currency_cost(), "clicking_hours": self.clicking_hours(),
            "market": self.market.map(|x| x.0), "listed": self.market.map(|x| x.1), "margin": self.margin(), "hole": self.hole(),
            "base_price": self.base_price.map(|x| x.0), "base_listed": self.base_price.map(|x| x.1), "base_source": self.base_source,
            "base_query": self.base_query, "base_url": self.base_url, "base_error": self.base_error,
            "trade_url": self.trade_url, "query": self.query, "market_error": self.market_error, "error": self.error,
            "fixed": self.fixed, "fixed_ids": self.fixed_ids, "search": self.search,
            "rows": self.rows.iter().map(|row| json!({"role": row.role, "side": row.side, "text": row.text, "tier": row.tier})).collect::<Vec<_>>(),
            "plan": self.summary.as_deref(), "goal": self.candidate, "model": "ponder-rules", "precision": "declared model; some outcome weights are estimated"
        })
    }
}

impl Serialize for Estimate {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_dict().serialize(serializer)
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
    pub pattern: Pattern,
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
                let c = &s.pattern;
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
        self.to_dict().serialize(s)
    }
}

/// `priority`: best margin first, then crafts more builds need, then the dearest.
pub fn priority(e: &Estimate) -> (bool, f64, i64, bool, f64) {
    (e.margin().is_none(), -e.margin().unwrap_or(0.0), -(e.builds.len() as i64), e.cheapest().is_none(), -e.cheapest().unwrap_or(0.0))
}

fn cmp_f(a: f64, b: f64) -> std::cmp::Ordering {
    a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
}

/// Orders estimates by `priority`.
pub fn cmp_priority(a: &Estimate, b: &Estimate) -> std::cmp::Ordering {
    let (x, y) = (priority(a), priority(b));
    x.0.cmp(&y.0).then(cmp_f(x.1, y.1)).then(x.2.cmp(&y.2)).then(x.3.cmp(&y.3)).then(cmp_f(x.4, y.4))
}

/// `LadderReport::ranked`'s order: market holes first (margin x wearers per listing), then
/// unpriced ones by ladder demand.
pub fn cmp_ladder(a: &Estimate, b: &Estimate) -> std::cmp::Ordering {
    (a.hole().is_none())
        .cmp(&b.hole().is_none())
        .then(cmp_f(-a.hole().unwrap_or(0.0), -b.hole().unwrap_or(0.0)))
        .then(b.demand.cmp(&a.demand))
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
        v.sort_by(|a, b| cmp_ladder(a, b));
        v
    }

    pub fn to_dict(&self) -> Value {
        self.to_dict_with_estimates(self.ranked().into_iter().map(Estimate::to_dict))
    }

    pub fn to_dict_with_estimates(&self, estimates: impl IntoIterator<Item = Value>) -> Value {
        json!({
            "league": self.league,
            "prices": self.prices_source,
            "chaos_per_divine": self.divine,
            "characters": self.characters,
            "items": self.items,
            "clusters": self.clusters,
            "archetypes": self.archetypes,
            "cluster_archetypes": self.cluster_archetypes,
            "estimates": estimates.into_iter().collect::<Vec<_>>(),
        })
    }
}

impl Serialize for LadderReport {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.to_dict().serialize(s)
    }
}
