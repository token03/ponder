//! Prices in chaos, and what clicking time is worth.

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use ponder_rules::ops::Cost;

pub struct Market {
    pub prices: HashMap<String, f64>,
    /// The user's value of an hour of crafting, in chaos (SPEC §10).
    pub chaos_per_hour: f64,
}

impl Market {
    /// A `{price id: chaos}` JSON file, optionally gzipped.
    pub fn load(path: &Path) -> std::io::Result<Market> {
        let mut bytes = Vec::new();
        std::fs::File::open(path)?.read_to_end(&mut bytes)?;
        if path.extension().is_some_and(|e| e == "gz") {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut out)?;
            bytes = out;
        }
        let prices = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
        Ok(Market { prices, chaos_per_hour: 1500.0 })
    }

    /// Chaos for one use, or `None` when something is unpriced (never "free", SPEC §5.6).
    pub fn price(&self, cost: &Cost) -> Option<f64> {
        let mut total = cost.seconds * self.chaos_per_hour / 3600.0;
        for (id, amount) in &cost.items {
            total += amount * self.prices.get(id)?;
        }
        Some(total)
    }
}
