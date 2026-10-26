//! What the pipeline needs from the outside world, injected: settings, the prices, trade and
//! ladder clients, the plan cache and the repoe lookups the trade queries use.

use std::sync::{Arc, OnceLock, RwLock};

use ponder_core::Settings;
use ponder_net::http::{self, Transport};
use ponder_net::ladder::Ladder;
use ponder_net::prices::Prices as PriceClient;
use ponder_net::trade::{StatIds, Trade};
use serde_json::{Number, Value};

use super::cache::PlanCache;

/// A progress callback: told what the pipeline is doing ("solving 12/340").
pub type Progress<'a> = &'a (dyn Fn(&str) + Sync);

/// Chaos an hour of clicking is worth, from `PONDER_CHAOS_PER_HOUR` (default 1500), as Python's
/// `craft.CHAOS_PER_HOUR`. The one place the library reads that setting.
pub fn settings_from_env() -> Settings {
    let per_hour = ponder_net::env::env("PONDER_CHAOS_PER_HOUR")
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|rate| rate.is_finite() && *rate >= 0.0);
    per_hour.map_or_else(Settings::default, |chaos_per_hour| Settings { chaos_per_hour })
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
    /// Plans solved in this process, shared between products (donors, preparation stages).
    pub solves: super::solving::SharedSolves,
    resale_quotes: RwLock<std::collections::BTreeMap<String, super::resale::ResaleQuote>>,
    clusters: OnceLock<Vec<Value>>,
}

impl Context {
    pub fn new(settings: Settings, prices: PriceClient, trade: Trade, ladder: Ladder, http: Arc<dyn Transport>) -> Context {
        let quotes =
            std::fs::read(".cache/recovery-quotes.json").ok().and_then(|raw| serde_json::from_slice(&raw).ok()).unwrap_or_default();
        Context {
            settings,
            prices,
            trade,
            ladder,
            http,
            plans: PlanCache::default(),
            solves: Default::default(),
            resale_quotes: RwLock::new(quotes),
            clusters: OnceLock::new(),
        }
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

    pub fn resale_quote(&self, problem: &ponder_core::Problem, core: ponder_core::Preservation) -> Option<super::resale::ResaleQuote> {
        self.resale_quotes
            .read()
            .expect("resale quotes")
            .get(&format!("{}/{}/{:?}", problem.fingerprint(), core.targets, core.filler))
            .copied()
    }

    pub fn set_resale_quote(
        &self,
        problem: &ponder_core::Problem,
        core: ponder_core::Preservation,
        quote: Option<super::resale::ResaleQuote>,
    ) -> Result<(), String> {
        if let Some(quote) = quote {
            quote.validate()?;
        }
        let mut quotes = self.resale_quotes.write().expect("resale quotes");
        let mut updated = quotes.clone();
        let key = format!("{}/{}/{:?}", problem.fingerprint(), core.targets, core.filler);
        match quote {
            Some(quote) => {
                updated.insert(key, quote);
            }
            None => {
                updated.remove(&key);
            }
        }
        let raw = serde_json::to_vec_pretty(&updated).map_err(|e| e.to_string())?;
        std::fs::create_dir_all(".cache").map_err(|e| e.to_string())?;
        std::fs::write(".cache/recovery-quotes.json.tmp", raw).map_err(|e| e.to_string())?;
        std::fs::rename(".cache/recovery-quotes.json.tmp", ".cache/recovery-quotes.json").map_err(|e| e.to_string())?;
        *quotes = updated;
        Ok(())
    }
    pub fn stat_min(&self, mod_ids: &[&str]) -> Option<Number> {
        crate::pricing::TradeData::load().stat_min(mod_ids)
    }
    pub fn stat_id(&self, mod_id: &str, kind: &str) -> Option<String> {
        crate::pricing::TradeData::load().stat_id(mod_id, kind)
    }
    pub fn mods(&self) -> &ponder_game::data::Mods {
        &crate::pricing::TradeData::load().mods
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
