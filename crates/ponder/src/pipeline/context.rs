//! What the pipeline needs from the outside world, injected: settings, the prices, trade and
//! ladder clients, the plan cache and the repoe lookups the trade queries use.

use std::sync::{Arc, OnceLock};

use ponder_core::Settings;
use ponder_net::http::{self, Transport};
use ponder_net::ladder::Ladder;
use ponder_net::prices::Prices as PriceClient;
use ponder_net::trade::{StatIds, Trade, Translations};
use serde_json::{Number, Value};

use super::cache::PlanCache;

/// A progress callback: told what the pipeline is doing ("solving 12/340").
pub type Progress<'a> = &'a (dyn Fn(&str) + Sync);

/// Chaos an hour of clicking is worth, from `PONDER_CHAOS_PER_HOUR` (default 1500), as Python's
/// `craft.CHAOS_PER_HOUR`. The one place the library reads that setting.
pub fn settings_from_env() -> Settings {
    let per_hour = ponder_net::env::env("PONDER_CHAOS_PER_HOUR").and_then(|v| v.trim().parse::<f64>().ok());
    per_hour.map_or_else(Settings::default, |chaos_per_hour| Settings { chaos_per_hour })
}

struct Stats {
    mods: Arc<Value>,
    translations: Translations,
}

/// Everything a pipeline run talks to. `Context::real()` is the real thing; tests build one with
/// `Context::new` and fake transports, so nothing here touches the network on its own.
pub struct Context {
    /// What an hour of clicking is worth.
    pub settings: Settings,
    /// poe.ninja prices (and the cluster jewel listings saved with them).
    pub prices: PriceClient,
    /// The trade site, with its disk cache and rate limits.
    pub trade: Trade,
    /// poe.ninja's ladder, with its disk cache.
    pub ladder: Ladder,
    /// Downloads of build share links (pobb.in, pastebin).
    pub http: Arc<dyn Transport>,
    /// Solved plans, by problem.
    pub plans: PlanCache,
    stats: OnceLock<Stats>,
    clusters: OnceLock<Vec<Value>>,
}

impl Context {
    pub fn new(settings: Settings, prices: PriceClient, trade: Trade, ladder: Ladder, http: Arc<dyn Transport>) -> Context {
        Context { settings, prices, trade, ladder, http, plans: PlanCache::default(), stats: OnceLock::new(), clusters: OnceLock::new() }
    }

    /// The real network and caches under the working directory; settings from the environment.
    pub fn real() -> Context {
        Context::new(settings_from_env(), PriceClient::new(), Trade::new(), Ladder::new(), http::real())
    }

    /// Keep plans somewhere else (or nowhere: `PlanCache::disabled()`).
    pub fn with_plans(mut self, plans: PlanCache) -> Context {
        self.plans = plans;
        self
    }

    fn stats(&self) -> &Stats {
        self.stats.get_or_init(|| {
            let mods = ponder_game::data::load("mods").unwrap_or_else(|e| panic!("{e}"));
            let translations = ponder_game::data::load("stat_translations").unwrap_or_else(|e| panic!("{e}"));
            Stats { mods, translations: Translations::new(&translations) }
        })
    }

    /// `trade.stat_min(mod_ids)`
    pub fn stat_min(&self, mod_ids: &[&str]) -> Option<Number> {
        ponder_net::trade::stat_min(&self.stats().mods, mod_ids)
    }

    /// `trade.stat_id(mod_id, kind)`
    pub fn stat_id(&self, mod_id: &str, kind: &str) -> Option<String> {
        let s = self.stats();
        ponder_net::trade::stat_id(&s.mods, &s.translations, mod_id, kind)
    }

    /// repoe's `mods.json`, untyped.
    pub fn mods_json(&self) -> &Value {
        &self.stats().mods
    }

    /// poe.ninja's cluster jewel listings, read once.
    fn cluster_lines(&self) -> &[Value] {
        self.clusters.get_or_init(|| self.prices.cluster_lines())
    }

    /// `prices.cluster_base`: the cheapest listed base with this enchant and passive count at or above `item_level`.
    pub fn cluster_base(&self, size: &str, enchant_text: &str, passives: i64, item_level: i64) -> Option<(f64, i64)> {
        let variant = format!("{passives} passives");
        let best = self
            .cluster_lines()
            .iter()
            .filter(|l| {
                l["baseType"].as_str().is_some_and(|b| b.to_lowercase().starts_with(size))
                    && l["name"] == enchant_text
                    && l["variant"] == variant.as_str()
                    && l["levelRequired"].as_f64().unwrap_or(0.0) >= item_level as f64
                    && l["count"].as_f64().unwrap_or(0.0) >= 3.0
            })
            .min_by(|a, b| a["chaosValue"].as_f64().unwrap_or(f64::NAN).total_cmp(&b["chaosValue"].as_f64().unwrap_or(f64::NAN)))?;
        Some((best["chaosValue"].as_f64()?, best["count"].as_i64()?))
    }

    /// `prices.enchant_option`: the trade-site option id of a cluster enchant.
    pub fn enchant_option(&self, enchant_text: &str) -> Option<String> {
        for line in self.cluster_lines() {
            if line["name"] == enchant_text {
                for info in line["tradeInfo"].as_array().into_iter().flatten() {
                    if info["mod"] == ponder_net::trade::ENCHANT_STAT {
                        return info["option"].as_str().map(str::to_string);
                    }
                }
            }
        }
        None
    }
}

impl StatIds for Context {
    fn stat_id(&self, mod_id: &str, kind: &str) -> Option<String> {
        Context::stat_id(self, mod_id, kind)
    }
}
