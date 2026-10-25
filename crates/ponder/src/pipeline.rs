//! The whole pipeline as a library, builds in, priced crafting
//! estimates out.
//!
//! `run` loads each build, matches its rares to mod ids, and turns each rare into a craft: the
//! target mods plus the group of bases that roll exactly the same mods (so Warlock and Sorcerer
//! Gloves are one "Gloves (ES)" craft). Crafts that several builds need are merged. Every craft
//! gets the cheapest plan, and optionally fracture options and trade prices. `ladder` does the
//! same for what invested characters wear, and finds the pivots between products of a base group.
//!
//! Nothing here prints; render the `Report` or `LadderReport`, or serialise them: `to_dict()`
//! (and `serde::Serialize`) give the JSON the CLI prints. Everything outside the process (poe.ninja, the
//! trade site, share links, the plan cache) comes through a [`Context`].
//!
//! The solver's answers are cached on disk under `.cache/plans` (see [`PlanCache`]).

mod build;
mod cache;
mod context;
mod donors;
mod finish;
mod market;
mod pivots;
mod protected;
mod recombs;
mod resale;
mod routes;
mod run;
mod solving;
mod types;

pub use build::{ClusterOpts, GearOpts, add_fractures, estimate_cluster, estimate_gear, estimate_item};
pub use cache::{PLAN_VERSION, PlanCache, plan_key, round_sig};
pub use context::{Context, Progress, settings_from_env};
pub use donors::{donor_mod_range, donor_query};
pub use finish::{Finish, find_starts, finish, finishes};
pub use market::{Lookup, lookup, lookup_market, price_fractures, price_on_trade};
pub use pivots::{
    PIVOT_CLICKS, PIVOT_MIN_CHAOS, PIVOT_MIN_PER_CRAFT, PIVOT_SHARE, PIVOTS_PER_STEP, Pivot, add_pivots, pivots_of, short_name,
};
pub use protected::{ProtectedRecipe, add_protected_routes};
pub use recombs::{add_recombs, add_recombs_in_league, price_donor_routes};
pub use routes::price_routes;
pub use resale::{ResaleQuote, SaleThreshold};
pub use run::{LadderOpts, RunOpts, build_name, current_league, expand_sources, ladder, load_build, run};
pub use solving::{Solvable, SolveSlot, slots_of, solve_all, solve_estimates, solve_one, solve_slots};
pub use types::{
    BuildInfo, CraftRoute, Estimate, Fracture, JEWELLERY, Key, LadderReport, ProductRef, RecombInfo, Report, Search, Shared, Slot,
    cmp_ladder, cmp_priority, priority,
};

pub use crate::item::listing_to_item;
