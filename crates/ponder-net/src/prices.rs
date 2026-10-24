//! Prices in chaos from poe.ninja, keyed by poe.ninja's ids (e.g. "alt", "regal").
//!
//! `Prices::fetch` saves the current league's currency, essence, fossil,
//! resonator and cluster jewel prices under .cache/ninja. Without a fetch, a
//! small snapshot (Allflame, October 2026) keeps the numbers on the right scale.

use crate::env::Clock;
use crate::http::{self, Transport};
use crate::pyjson::{self, Dumps, quote};
use anyhow::{Context, Result, anyhow};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

pub const API: &str = "https://poe.ninja/poe1/api/economy";
pub const USER_AGENT: &str = "ponder/0.1 (crafting cost research)";
pub const CACHE: &str = ".cache/ninja";
pub const EXCHANGE_TYPES: &[&str] = &["Currency", "Essence", "Fossil", "Resonator"];
/// poe.ninja's id for the cluster jewel enchant stat on the trade site.
const ENCHANT_STAT: &str = "enchant.stat_3948993189";

#[rustfmt::skip]
pub const SNAPSHOT: &[(&str, f64)] = &[
    ("chaos", 1.0), ("divine", 332.3), ("transmute", 0.004), ("aug", 0.017), ("alt", 0.074), ("regal", 0.093),
    ("alch", 0.032), ("scour", 0.35), ("exalted", 5.09), ("annul", 11.33), ("fracturing-orb", 240.7),
    ("wild-lifeforce", 0.017), ("vivid-lifeforce", 0.175), ("primal-lifeforce", 0.047),
    ("primitive-chaotic-resonator", 0.56),
];

/// poe.ninja prices and the `.cache/ninja` files they are saved in.
#[derive(Clone)]
pub struct Prices {
    pub http: Arc<dyn Transport>,
    pub cache: PathBuf,
    pub clock: Clock,
}

impl Default for Prices {
    fn default() -> Self {
        Prices::new()
    }
}

impl Prices {
    pub fn new() -> Prices {
        Prices::with(http::real(), CACHE.into(), Clock::real())
    }

    pub fn with(http: Arc<dyn Transport>, cache: PathBuf, clock: Clock) -> Prices {
        Prices { http, cache, clock }
    }

    pub fn leagues(&self) -> Result<Vec<String>> {
        let body = self.get(&format!("{API}/leagues"))?;
        body.as_array().context("leagues is not a list")?.iter().map(|e| Ok(e["id"].as_str().context("league without id")?.to_string())).collect()
    }

    /// Download the prices of `league` (default: the first one listed); returns the league and how many prices.
    pub fn fetch(&self, league: Option<&str>) -> Result<(String, usize)> {
        let league = match league {
            Some(l) => l.to_string(),
            None => self.leagues()?.into_iter().next().ok_or_else(|| anyhow!("no leagues"))?,
        };
        std::fs::create_dir_all(&self.cache)?;
        let mut table = Map::new();
        table.insert("chaos".into(), json!(1.0));
        for kind in EXCHANGE_TYPES {
            let body = self.get(&format!("{API}/exchange/current/overview?league={}&type={kind}", quote(&league)))?;
            for line in body["lines"].as_array().context("no lines")? {
                table.insert(line["id"].as_str().context("line without id")?.to_string(), line["primaryValue"].clone());
            }
            self.clock.sleep(1.0); // be gentle; the data only refreshes every ~15 minutes anyway
        }
        let clusters = self.get(&format!("{API}/stash/current/item/overview?league={}&type=ClusterJewel", quote(&league)))?;
        let saved = json!({"league": league, "fetched": self.clock.now(), "prices": table});
        std::fs::write(self.cache.join("prices.json"), pyjson::dumps_with(&saved, Dumps { indent: Some(1), ..Dumps::default() }))?;
        std::fs::write(self.cache.join("clusters.json"), pyjson::dumps(&clusters["lines"]))?;
        Ok((league, table.len()))
    }

    /// Prices plus a note on where they came from.
    pub fn load(&self) -> Result<(HashMap<String, f64>, String)> {
        let path = self.cache.join("prices.json");
        let mut prices: HashMap<String, f64> = SNAPSHOT.iter().map(|(k, v)| (k.to_string(), *v)).collect();
        if !path.is_file() {
            return Ok((prices, "built-in Allflame snapshot (run `ponder prices fetch`)".into()));
        }
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
        let age = (self.clock.now() - saved["fetched"].as_f64().context("prices.json has no fetch time")?) / 3600.0;
        for (k, v) in saved["prices"].as_object().context("prices.json has no prices")? {
            if let Some(f) = v.as_f64() {
                prices.insert(k.clone(), f);
            }
        }
        Ok((prices, format!("poe.ninja {}, {age:.0}h old", saved["league"].as_str().unwrap_or(""))))
    }

    /// poe.ninja's cluster jewel listings, as saved by `fetch` (empty before the first fetch).
    pub fn cluster_lines(&self) -> Vec<Value> {
        let Ok(text) = std::fs::read_to_string(self.cache.join("clusters.json")) else { return Vec::new() };
        serde_json::from_str::<Value>(&text).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default()
    }

    /// Cheapest-listed price of a base with this enchant and passive count, at or above `item_level`.
    ///
    /// poe.ninja buckets bases by item level (1, 50, 68, 75, 84); a higher bucket
    /// still works for lower-level notables, so take the cheapest qualifying one
    /// with real volume. Returns (chaos, listings).
    pub fn cluster_base(&self, size: &str, enchant_text: &str, passives: i64, item_level: i64) -> Option<(f64, i64)> {
        let variant = format!("{passives} passives");
        let lines = self.cluster_lines();
        let best = lines
            .iter()
            .filter(|l| {
                l["baseType"].as_str().is_some_and(|b| b.to_lowercase().starts_with(size))
                    && l["name"] == enchant_text
                    && l["variant"] == variant.as_str()
                    && l["levelRequired"].as_f64().unwrap_or(0.0) >= item_level as f64
                    && l["count"].as_f64().unwrap_or(0.0) >= 3.0
            })
            .min_by(|a, b| a["chaosValue"].as_f64().unwrap_or(f64::NAN).total_cmp(&b["chaosValue"].as_f64().unwrap_or(f64::NAN)))?; // first of equals, like min()
        Some((best["chaosValue"].as_f64()?, best["count"].as_i64()?))
    }

    /// The trade-site option id for a cluster enchant, as poe.ninja lists it.
    pub fn enchant_option(&self, enchant_text: &str) -> Option<String> {
        for line in self.cluster_lines() {
            if line["name"] == enchant_text {
                for info in line["tradeInfo"].as_array().into_iter().flatten() {
                    if info["mod"] == ENCHANT_STAT {
                        return info["option"].as_str().map(str::to_string);
                    }
                }
            }
        }
        None
    }

    fn get(&self, url: &str) -> Result<Value> {
        http::get_json(&*self.http, url, USER_AGENT, 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Request, Response};
    use crate::testing::TempDir;
    use std::sync::Mutex;

    fn fake(urls: Arc<Mutex<Vec<String>>>) -> Arc<dyn Transport> {
        Arc::new(move |req: &Request| {
            urls.lock().unwrap().push(req.url.clone());
            let u = &req.url;
            Ok(Response::json(
                200,
                &if u.ends_with("/leagues") {
                    json!([{"id": "Test League"}, {"id": "Standard"}])
                } else if u.contains("stash/") {
                    json!({"lines": [{"name": "Added Small Passive Skills grant: X", "baseType": "Large Cluster Jewel", "variant": "8 passives", "levelRequired": 84, "count": 5, "chaosValue": 12.5,
                        "tradeInfo": [{"mod": ENCHANT_STAT, "option": 42}]}]})
                } else {
                    json!({"lines": [{"id": "alt", "primaryValue": 0.1}, {"id": "divine", "primaryValue": 300}]})
                },
            ))
        })
    }

    #[test]
    fn fetch_then_load() {
        let dir = TempDir::new("prices");
        let urls = Arc::new(Mutex::new(Vec::new()));
        let p = Prices::with(fake(urls.clone()), dir.path().into(), Clock::new(|| 100_000.0, |_| {}));
        assert_eq!(p.load().unwrap().1, "built-in Allflame snapshot (run `ponder prices fetch`)");
        assert_eq!(p.fetch(None).unwrap(), ("Test League".to_string(), 3));
        assert!(urls.lock().unwrap()[1].ends_with("league=Test%20League&type=Currency"));
        let (table, note) = p.load().unwrap();
        assert_eq!((table["alt"], table["divine"], table["scour"]), (0.1, 300.0, 0.35));
        assert_eq!(note, "poe.ninja Test League, 0h old");
        assert_eq!(std::fs::read_to_string(dir.path().join("prices.json")).unwrap().lines().next(), Some("{"));
        assert_eq!(p.cluster_base("large", "Added Small Passive Skills grant: X", 8, 75), Some((12.5, 5)));
        assert_eq!(p.cluster_base("large", "Added Small Passive Skills grant: X", 8, 85), None);
        assert_eq!(p.enchant_option("Added Small Passive Skills grant: X"), None); // option is a number here; ids are strings on the real site
    }
}
