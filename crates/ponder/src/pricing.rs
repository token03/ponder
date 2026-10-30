//! What tiers are worth: a price model fitted on trade listings.
//!
//! For an archetype (a base group plus the mods that define
//! it), search trade for items that have every core mod at a low floor, so listings span many
//! tiers. Each listing's mods are matched to ids and tiers, then
//!
//! ```text
//! log(price) ~ intercept + sum_k (T1_k, T2_k dummies) + resistances + open affixes
//! ```
//!
//! is fitted by least squares. `exp(coefficient)` is the price multiplier for having mod k at
//! T1 (or T2) instead of a lower tier, holding the rest fixed.
//!
//! Listings are cheapest-first and asking prices include fixers, so treat the multipliers as
//! directions and sizes, not exact values; more listings help.
//!
//! The trade layer is passed in: [`TradeData`] holds repoe's mods and stat translations (the
//! pieces of `ponder_net::trade` that read repoe), so nothing here touches the network.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

use ponder_game::data;
use ponder_game::mods::{ItemMatch, ModDB};
use ponder_net::trade::{self, ItemQuery, Rates, StatIds};
use serde_json::Value;

use crate::candidate::{RES, stat_key};
use crate::gear;

/// Search floor: listings at this tier or better on every core mod.
pub const FLOOR_TIER: i64 = 4;

/// Repoe's raw `mods.json` and indexed `stat_translations.json`: what trade searches read.
pub struct TradeData {
    pub mods: Arc<data::Mods>,
    translations: HashMap<(String, String), String>,
}

impl TradeData {
    pub fn new(mods: Arc<data::Mods>, stat_translations: &[data::StatTranslation]) -> TradeData {
        let mut translations = HashMap::new();
        for entry in stat_translations {
            for id in &entry.ids {
                for stat in entry.trade_stats.iter().flatten() {
                    translations.entry((id.clone(), stat.type_.clone())).or_insert_with(|| stat.id.clone());
                }
            }
        }
        TradeData { mods, translations }
    }

    pub fn stat_min(&self, mod_ids: &[&str]) -> Option<serde_json::Number> {
        mod_ids.iter().filter_map(|id| self.mods.get(*id)?.stats.first().map(|s| s.min)).min_by(f64::total_cmp).map(trade::num)
    }

    /// The default repoe data (as Python's `trade.stat_id` reads it); panics if it is missing.
    pub fn load() -> &'static TradeData {
        static T: OnceLock<TradeData> = OnceLock::new();
        T.get_or_init(|| {
            let mods = data::mods().unwrap_or_else(|e| panic!("{e}"));
            let translations = data::stat_translations().unwrap_or_else(|e| panic!("{e}"));
            TradeData::new(mods, &translations)
        })
    }
}

impl StatIds for TradeData {
    fn stat_id(&self, mod_id: &str, kind: &str) -> Option<String> {
        let first = &self.mods.get(mod_id)?.stats.first()?.id;
        self.translations.get(&(first.clone(), kind.into())).cloned()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TierPrice {
    /// main stat
    pub key: String,
    pub text: String,
    /// price multiplier for T1 vs below T2
    pub t1: Option<f64>,
    pub t2: Option<f64>,
    /// listings with it at T1
    pub n_t1: i64,
    pub n_t2: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PriceModel {
    pub group: String,
    pub listings: usize,
    /// fitted price with every core mod below T2, no res, no open affix
    pub base_price: f64,
    pub tiers: Vec<TierPrice>,
    /// multiplier per resistance mod
    pub per_res: Option<f64>,
    pub per_open_affix: Option<f64>,
    pub r2: f64,
}

/// A trade search for the archetype's core mods at `floor` tier or better, on its base group.
///
/// `example` is one of the archetype's matches (`candidate.archetype_match`); None when no core
/// mod has a trade stat.
pub fn archetype_query(db: &ModDB, example: &ItemMatch, keys: &[&str], floor: i64, td: &TradeData) -> Option<Value> {
    let grp = gear::group(db, example);
    let base_id = example.base_id.as_deref().expect("an example with a base");
    let mut q = ItemQuery::default();
    if matches!(grp.item_class.as_str(), "Ring" | "Amulet" | "Belt") && grp.bases.len() > 1 {
        q.category = trade::category(&grp.item_class).map(String::from);
    } else {
        q.base = Some(grp.best().to_string());
    }
    let pool = gear::pool(db, base_id, &example.tags, 86);
    for key in keys {
        if *key == RES {
            continue;
        }
        let mods: Vec<&str> = pool
            .iter()
            .filter(|m| stat_key(&m.id) == *key && db.tier(&m.id, &example.tags).unwrap_or(99) <= floor)
            .map(|m| m.id.as_str())
            .collect();
        if let Some(sid) = mods.first().and_then(|m| td.stat_id(m, "explicit")) {
            q.stats.push((sid, td.stat_min(&mods)));
        }
    }
    (!q.stats.is_empty()).then(|| trade::item_query(&q))
}

/// A trade listing's item, matched to mod ids and tiers through its mod text.
pub fn listing_match(db: &ModDB, listing: &Value) -> Option<ItemMatch> {
    let item = crate::item::listing_to_item(&trade::listing_item(listing));
    let m = db.match_item(&item);
    m.base_id.is_some().then_some(m)
}

pub fn lstsq(x: &[Vec<f64>], y: &[f64]) -> Vec<f64> {
    let (m, n) = (x.len(), x.first().map_or(0, Vec::len));
    if m == 0 || n == 0 {
        return vec![0.0; n];
    }
    let a = faer::Mat::from_fn(m, n, |i, j| x[i][j]);
    let y = faer::Mat::from_fn(m, 1, |i, _| y[i]);
    let fitted = a.thin_svd().expect("finite price features").pseudoinverse() * y;
    (0..n).map(|i| fitted[(i, 0)]).collect()
}

/// Fit the price model for an archetype from its trade listings; None if there are too few usable ones.
pub fn fit(db: &ModDB, group: &str, keys: &[&str], listings: &[Value], rates: &Rates) -> Option<PriceModel> {
    let core: Vec<&str> = keys.iter().copied().filter(|k| *k != RES).collect();
    let (mut rows, mut prices): (Vec<Vec<f64>>, Vec<f64>) = (Vec::new(), Vec::new());
    let mut counts: HashMap<&str, [i64; 2]> = core.iter().map(|k| (*k, [0, 0])).collect();
    for listing in listings {
        let price = trade::chaos_price(listing, rates);
        let m = listing_match(db, listing);
        let (Some(price), Some(m)) = (price.filter(|p| *p != 0.0), m) else {
            continue;
        };
        let mut best: HashMap<&str, i64> = HashMap::new();
        let mut res = 0;
        for mod_ in &m.mods {
            if mod_.source == "crafted" {
                continue;
            }
            if gear::is_filler_family(&mod_.family) {
                res += 1;
            }
            let k = stat_key(&mod_.mod_id);
            if counts.contains_key(k) && mod_.tier.is_some_and(|t| t != 0) {
                let t = best.entry(k).or_insert(99);
                *t = (*t).min(mod_.tier.unwrap());
            }
        }
        let open_affixes = (6 - m.mods.iter().filter(|x| x.source != "crafted").count() as i64).max(0);
        let mut row = vec![1.0];
        for k in &core {
            let t = best.get(k).copied().unwrap_or(99);
            row.push(f64::from(u8::from(t == 1)));
            row.push(f64::from(u8::from(t == 2)));
            let c = counts.get_mut(k).unwrap();
            c[0] += i64::from(t == 1);
            c[1] += i64::from(t == 2);
        }
        row.push(f64::from(res));
        row.push(open_affixes as f64);
        rows.push(row);
        prices.push(price.ln());
    }
    if rows.len() < 2 * core.len() + 4 {
        return None;
    }
    let coef = lstsq(&rows, &prices);
    let y = prices;
    let fitted: Vec<f64> = rows.iter().map(|r| r.iter().zip(&coef).map(|(a, b)| a * b).sum()).collect();
    let mean = y.iter().sum::<f64>() / y.len() as f64;
    let ss_res: f64 = y.iter().zip(&fitted).map(|(a, b)| (a - b) * (a - b)).sum();
    let ss_tot: f64 = y.iter().map(|a| (a - mean) * (a - mean)).sum();
    let r2 = 1.0 - ss_res / ss_tot.max(1e-12);
    let mut model = PriceModel {
        group: group.to_string(),
        listings: rows.len(),
        base_price: coef[0].exp(),
        tiers: Vec::new(),
        per_res: None,
        per_open_affix: None,
        r2,
    };
    for (i, k) in core.iter().enumerate() {
        let (t1, t2) = (coef[1 + 2 * i], coef[2 + 2 * i]);
        let [n1, n2] = counts[k];
        model.tiers.push(TierPrice {
            key: k.to_string(),
            text: k.to_string(),
            t1: (n1 > 0).then(|| t1.exp()),
            t2: (n2 > 0).then(|| t2.exp()),
            n_t1: n1,
            n_t2: n2,
        });
    }
    model.per_res = Some(coef[coef.len() - 2].exp());
    model.per_open_affix = Some(coef[coef.len() - 1].exp());
    Some(model)
}
