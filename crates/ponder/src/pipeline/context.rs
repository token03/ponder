use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow};
use ponder_net::{
    http::{self, Transport},
    ladder::Ladder,
    prices::Prices,
    trade::{StatIds, Trade},
};
use ponder_rules::{assumptions::Assumptions, data::Repoe, game::Game};
use serde_json::Number;

use super::cache::Cache;
use crate::gear::Settings;

pub type Progress<'a> = &'a (dyn Fn(&str) + Sync);

pub fn settings_from_env() -> Settings {
    let rate = ponder_net::env::env("PONDER_CHAOS_PER_HOUR")
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0);
    rate.map_or_else(Settings::default, |chaos_per_hour| Settings { chaos_per_hour })
}

pub struct Context {
    pub settings: Settings,
    pub prices: Prices,
    pub trade: Trade,
    pub ladder: Ladder,
    pub http: Arc<dyn Transport>,
    pub plans: Cache,
    pub assumptions: Assumptions,
    rules: OnceLock<Result<(Game, blake3::Hash), String>>,
}

impl Context {
    pub fn new(settings: Settings, prices: Prices, trade: Trade, ladder: Ladder, http: Arc<dyn Transport>) -> Self {
        Self { settings, prices, trade, ladder, http, plans: Cache::default(), assumptions: Assumptions::default(), rules: OnceLock::new() }
    }

    pub fn real() -> Self {
        Self::new(settings_from_env(), Prices::new(), Trade::new(), Ladder::new(), http::real())
    }

    pub fn with_plans(mut self, plans: Cache) -> Self {
        self.plans = plans;
        self
    }

    pub fn rules(&self) -> Result<(&Game, blake3::Hash)> {
        let loaded = self.rules.get_or_init(|| {
            let dir = ponder_game::data::data_dir();
            let mut hash = blake3::Hasher::new();
            for name in ["mods", "base_items", "essences", "fossils", "crafting_bench_options", "cluster_jewels"] {
                let path = dir.join(format!("{name}.json"));
                let path = if path.exists() { path } else { dir.join(format!("{name}.json.gz")) };
                hash.update(name.as_bytes());
                hash.update(&std::fs::read(path).map_err(|error| error.to_string())?);
            }
            Repoe::load(&dir).map(|repoe| (Game::new(repoe), hash.finalize())).map_err(|error| error.to_string())
        });
        loaded.as_ref().map(|(game, key)| (game, *key)).map_err(|error| anyhow!(error.clone()))
    }

    pub fn stat_min(&self, ids: &[&str]) -> Option<Number> {
        crate::pricing::TradeData::load().stat_min(ids)
    }
    pub fn stat_id(&self, id: &str, kind: &str) -> Option<String> {
        crate::pricing::TradeData::load().stat_id(id, kind)
    }
    pub fn mods(&self) -> &ponder_game::data::Mods {
        &crate::pricing::TradeData::load().mods
    }
    pub fn cluster_base(&self, size: &str, text: &str, passives: i64, level: i64) -> Option<(f64, i64)> {
        self.prices.cluster_base(size, text, passives, level)
    }
    pub fn enchant_option(&self, text: &str) -> Option<String> {
        self.prices.enchant_option(text)
    }
}

impl StatIds for Context {
    fn stat_id(&self, id: &str, kind: &str) -> Option<String> {
        self.stat_id(id, kind)
    }
}
