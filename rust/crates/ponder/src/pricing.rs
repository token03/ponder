//! What tiers are worth: a price model fitted on trade listings.
//!
//! Port of `src/ponder/pricing.py`. For an archetype (a base group plus the mods that define
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

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use ponder_game::data;
use ponder_game::item::{Item, ModLine};
use ponder_game::mods::{ItemMatch, ModDB};
use ponder_net::pyjson::truthy;
use ponder_net::trade::{self, ItemQuery, Rates, StatIds, Translations};
use serde_json::Value;

use crate::demand::{RES, stat_key};
use crate::gear;

/// Search floor: listings at this tier or better on every core mod.
pub const FLOOR_TIER: i64 = 4;

/// Repoe's raw `mods.json` and indexed `stat_translations.json`: what trade searches read.
pub struct TradeData {
    pub mods: Arc<Value>,
    pub translations: Translations,
}

impl TradeData {
    pub fn new(mods: Arc<Value>, stat_translations: &Value) -> TradeData {
        TradeData {
            mods,
            translations: Translations::new(stat_translations),
        }
    }

    /// The default repoe data (as Python's `trade.stat_id` reads it); panics if it is missing.
    pub fn load() -> &'static TradeData {
        static T: OnceLock<TradeData> = OnceLock::new();
        T.get_or_init(|| {
            let mods = data::load("mods").unwrap_or_else(|e| panic!("{e}"));
            let translations = data::load("stat_translations").unwrap_or_else(|e| panic!("{e}"));
            TradeData::new(mods, &translations)
        })
    }
}

impl StatIds for TradeData {
    fn stat_id(&self, mod_id: &str, kind: &str) -> Option<String> {
        trade::stat_id(&self.mods, &self.translations, mod_id, kind)
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
/// `example` is one of the archetype's matches (`demand.archetype_match`); None when no core
/// mod has a trade stat.
pub fn archetype_query(
    db: &ModDB,
    example: &ItemMatch,
    keys: &[&str],
    floor: i64,
    td: &TradeData,
) -> Option<Value> {
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
            .filter(|m| {
                stat_key(&m.id) == *key && db.tier(&m.id, &example.tags).unwrap_or(99) <= floor
            })
            .map(|m| m.id.as_str())
            .collect();
        if let Some(sid) = mods.first().and_then(|m| td.stat_id(m, "explicit")) {
            q.stats.push((sid, trade::stat_min(&td.mods, &mods)));
        }
    }
    (!q.stats.is_empty()).then(|| trade::item_query(&q))
}

/// A trade listing's item, matched to mod ids and tiers through its mod text.
pub fn listing_match(db: &ModDB, listing: &Value) -> Option<ItemMatch> {
    let empty = Value::Null;
    let it = listing.get("item").filter(|i| truthy(i)).unwrap_or(&empty);
    let text = |key: &str| -> Vec<&str> {
        it.get(key)
            .filter(|v| truthy(v))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    };
    let mut lines: Vec<ModLine> = text("explicitMods").into_iter().map(ModLine::new).collect();
    lines.extend(
        text("fracturedMods")
            .into_iter()
            .map(|t| ModLine::with_marker(t, "fractured")),
    );
    lines.extend(
        text("craftedMods")
            .into_iter()
            .map(|t| ModLine::with_marker(t, "crafted")),
    );
    let influences: BTreeSet<String> = match it.get("influences") {
        Some(Value::Object(m)) => m
            .iter()
            .filter(|(_, v)| truthy(v))
            .map(|(k, _)| k.clone())
            .collect(),
        _ => BTreeSet::new(),
    };
    let mut item = Item::new(
        "RARE",
        it.get("name").and_then(Value::as_str).unwrap_or(""),
        it.get("baseType").and_then(Value::as_str).unwrap_or(""),
    );
    item.item_level = it.get("ilvl").and_then(Value::as_i64);
    item.influences = influences;
    item.explicits = lines;
    let m = db.match_item(&item);
    m.base_id.is_some().then_some(m)
}

/// Minimum-norm least squares (`numpy.linalg.lstsq(x, y, rcond=None)`), by one-sided Jacobi SVD.
///
/// `x` is row-major, one `Vec` per row. Singular values at or below `eps * max(rows, cols) *
/// largest` are treated as zero.
pub fn lstsq(x: &[Vec<f64>], y: &[f64]) -> Vec<f64> {
    let (m, n) = (x.len(), x.first().map_or(0, Vec::len));
    let mut a: Vec<Vec<f64>> = (0..n).map(|j| x.iter().map(|r| r[j]).collect()).collect(); // columns
    let mut v: Vec<Vec<f64>> = (0..n)
        .map(|j| (0..n).map(|i| f64::from(u8::from(i == j))).collect())
        .collect(); // columns of V
    for _ in 0..60 {
        let mut rotated = false;
        for p in 0..n {
            for q in p + 1..n {
                let alpha: f64 = a[p].iter().map(|t| t * t).sum();
                let beta: f64 = a[q].iter().map(|t| t * t).sum();
                let gamma: f64 = a[p].iter().zip(&a[q]).map(|(s, t)| s * t).sum();
                if gamma == 0.0 || gamma.abs() <= 1e-15 * (alpha * beta).sqrt() {
                    continue;
                }
                rotated = true;
                let zeta = (beta - alpha) / (2.0 * gamma);
                let t = zeta.signum() / (zeta.abs() + (1.0 + zeta * zeta).sqrt());
                let c = 1.0 / (1.0 + t * t).sqrt();
                let s = c * t;
                for cols in [&mut a, &mut v] {
                    let (lo, hi) = cols.split_at_mut(q);
                    for (u, w) in lo[p].iter_mut().zip(hi[0].iter_mut()) {
                        let (up, wq) = (*u, *w);
                        *u = c * up - s * wq;
                        *w = s * up + c * wq;
                    }
                }
            }
        }
        if !rotated {
            break;
        }
    }
    let sigma: Vec<f64> = a
        .iter()
        .map(|col| col.iter().map(|t| t * t).sum::<f64>().sqrt())
        .collect();
    let cutoff = f64::EPSILON * m.max(n) as f64 * sigma.iter().copied().fold(0.0, f64::max);
    let mut coef = vec![0.0; n];
    for j in 0..n {
        if sigma[j] > cutoff {
            let w = a[j].iter().zip(y).map(|(s, t)| s * t).sum::<f64>() / (sigma[j] * sigma[j]);
            for (c, vi) in coef.iter_mut().zip(&v[j]) {
                *c += w * vi;
            }
        }
    }
    coef
}

/// Fit the price model for an archetype from its trade listings; None if there are too few usable ones.
pub fn fit(
    db: &ModDB,
    group: &str,
    keys: &[&str],
    listings: &[Value],
    rates: &Rates,
) -> Option<PriceModel> {
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
            if matches!(mod_.source, "crafted" | "legacy-crafted") {
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
        let open_affixes =
            (6 - m.mods.iter().filter(|x| x.source != "crafted").count() as i64).max(0);
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
    let fitted: Vec<f64> = rows
        .iter()
        .map(|r| r.iter().zip(&coef).map(|(a, b)| a * b).sum())
        .collect();
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
