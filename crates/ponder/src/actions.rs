//! The crafting actions available on an item, with costs in
//! poe.ninja price ids.
//!
//! Game data (essences, fossils, the bench) comes from the data directory;
//! a missing file panics with the usual "run `ponder data fetch`" message.

use ponder_core::ActionKind;
use std::collections::BTreeMap;
use std::sync::Arc;

use ponder_core::{Action, MAGIC, NORMAL, RARE};
use ponder_game::data::DataError;

/// `[(price id, quantity)]`
pub type Cost = Vec<(String, f64)>;
/// weight profile name -> mod id -> weight
pub type Profiles = BTreeMap<String, BTreeMap<String, f64>>;

pub(crate) fn loaded<T>(r: Result<Arc<T>, DataError>) -> Arc<T> {
    r.unwrap_or_else(|e| panic!("{e}"))
}

fn act(name: &str, kind: ActionKind, cost: &[(&str, f64)], from: &[u8]) -> Action {
    Action::new(name, kind, cost, from)
}

/// The plain currency actions every item has.
pub fn basic() -> Vec<Action> {
    vec![
        act("transmute", ActionKind::Transmute, &[("transmute", 1.0)], &[NORMAL]),
        act("alt", ActionKind::Alt, &[("alt", 1.0)], &[MAGIC]),
        act("aug", ActionKind::Aug, &[("aug", 1.0)], &[MAGIC]),
        act("regal", ActionKind::Regal, &[("regal", 1.0)], &[MAGIC]),
        act("exalt", ActionKind::Exalt, &[("exalted", 1.0)], &[RARE]),
        act("annul", ActionKind::Annul, &[("annul", 1.0)], &[MAGIC, RARE]),
        act("alch", ActionKind::RerollRare, &[("alch", 1.0)], &[NORMAL]),
        act("chaos", ActionKind::Reforge, &[("chaos", 1.0)], &[RARE]),
        act("scour+alch", ActionKind::RerollRare, &[("scour", 1.0), ("alch", 1.0)], &[RARE]),
        act("scour+transmute", ActionKind::RerollMagic, &[("scour", 1.0), ("transmute", 1.0)], &[RARE]),
    ]
}

/// A price id: lowercase, apostrophes dropped, runs of anything else become one dash.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.to_lowercase().chars().filter(|&c| c != '\'') {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    out
}

mod bench;
mod essence;
mod fossils;
mod harvest;

pub use bench::{BENCH_CURRENCY, LOCK_PREFIXES, LOCK_SUFFIXES, MULTIMOD, NO_ATTACK, NO_CASTER, bench_cost, eldritch, metamods};
pub use essence::{essence_profiles, essences};
pub use fossils::{
    FOSSIL_COMBOS_KEPT, FossilOpts, FossilRow, RESONATOR, RESONATORS, fossil_combos, fossil_combos_with, fossil_rows, fossils,
};
pub use harvest::{HARVEST, HARVEST_ADD_REMOVE, SACRED_LIFEFORCE, harvest, harvest_add_remove};

#[cfg(test)]
mod tests {
    use super::*;
    use ponder_core::{Mod, Prices, Settings, Side};
    use std::collections::BTreeSet;

    #[test]
    fn slugs() {
        assert_eq!(slug("Deafening Essence of Greed"), "deafening-essence-of-greed");
        assert_eq!(slug("Prismatic Fossil"), "prismatic-fossil");
        assert_eq!(slug("Orb of Horizons' (x)"), "orb-of-horizons-x");
    }

    #[test]
    fn combinations_are_lexicographic() {
        let mut seen = Vec::new();
        crate::combinations::for_each(4, 2, |c| seen.push(c.to_vec()));
        assert_eq!(seen, [[0, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 3]]);
    }

    fn pool() -> Vec<Mod> {
        let mut pool = vec![
            Mod::new("x", Side::Prefix, 100.0, &["X"], false, "x", &["fire"]),
            Mod::new("y", Side::Suffix, 100.0, &["Y"], false, "y", &["cold"]),
        ];
        for i in 0..3 {
            pool.push(Mod::new(&format!("p{i}"), Side::Prefix, 1000.0, &[&format!("P{i}")], false, "junk", &[]));
            pool.push(Mod::new(&format!("s{i}"), Side::Suffix, 1000.0, &[&format!("S{i}")], false, "junk", &[]));
        }
        pool
    }

    #[test]
    fn fossil_combos_pair_up_fossils_that_each_help_one_wanted_mod() {
        let pool = pool();
        let row = |name: &str, pid: &str, f: &dyn Fn(&Mod) -> f64| FossilRow {
            name: name.into(),
            pid: pid.into(),
            mult: pool.iter().map(f).collect(),
        };
        let rows = [
            row("hot", "hot-fossil", &|m| {
                if m.tags.iter().any(|t| t == "fire") { 10.0 } else { 1.0 }
            }),
            row("icy", "icy-fossil", &|m| {
                if m.tags.iter().any(|t| t == "cold") { 10.0 } else { 1.0 }
            }),
            row("dud", "dud-fossil", &|m| {
                if m.tags.is_empty() { 0.5 } else { 1.0 }
            }),
        ];
        let mut prices: Prices =
            [("hot-fossil", 1.0), ("icy-fossil", 1.0), ("dud-fossil", 50.0)].iter().map(|(k, v)| (k.to_string(), *v)).collect();
        prices.extend(RESONATORS[1..].iter().map(|r| (r.to_string(), 0.5)));
        let (x, y): (BTreeSet<String>, BTreeSet<String>) = (["x".to_string()].into(), ["y".to_string()].into());
        let (acts, profiles) = fossil_combos_with(&rows, &pool, &[&x, &y], &prices, &Settings::default(), FossilOpts::default());
        assert_eq!(acts[0].seconds, 6.0);
        assert_eq!(acts[0].name, "hot+icy fossils"); // both wants: the pair, cheapest per hit
        assert!(acts[0].cost.contains(&(RESONATORS[2].to_string(), 1.0))); // two fossils take a potent resonator
        assert_eq!(profiles["fossil:hot+icy"]["x"], 1000.0);
        assert_eq!(profiles["fossil:hot+icy"]["p0"], 1000.0);
        let names: Vec<&str> = acts.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"hot fossil") && names.contains(&"icy fossil")); // each want alone: its own fossil
    }

    #[test]
    fn eldritch_needs_a_plain_armour_slot() {
        assert!(eldritch("Gloves", &["shaper".to_string()].into()).is_empty());
        assert!(eldritch("Ring", &BTreeSet::new()).is_empty());
        assert_eq!(eldritch("Gloves", &BTreeSet::new()).len(), 4);
    }

    #[test]
    fn harvest_only_offers_tags_the_pool_has() {
        let (acts, profiles) = harvest(&pool());
        let names: Vec<&str> = acts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["harvest fire", "harvest cold"]);
        assert_eq!(profiles["harvest:fire"].len(), 1);
        assert_eq!(acts[0].first_profile.as_deref(), Some("harvest:fire"));
        let add = harvest_add_remove(&profiles, false);
        assert_eq!(add.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["harvest add/remove fire", "harvest add/remove cold"]);
        assert!(harvest_add_remove(&profiles, true).is_empty()); // not on influenced items
    }
}
