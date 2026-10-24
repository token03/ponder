//! The whole pipeline as a library, builds in, priced crafting
//! estimates out.
//!
//! `run` loads each build, matches its rares to mod ids, and turns each rare into a craft: the
//! target mods plus the group of bases that roll exactly the same mods (so Warlock and Sorcerer
//! Gloves are one "Gloves (ES)" craft). Crafts that several builds need are merged. Every craft
//! gets the cheapest plan, and optionally fracture options and trade prices. `ladder` does the
//! same for what invested characters wear, and links the products into a tree.
//!
//! Nothing here prints; render the `Report` or `LadderReport`, or serialise them: `to_dict()`
//! (and `serde::Serialize`) give the JSON the CLI prints. Everything outside the process (poe.ninja, the
//! trade site, share links, the plan cache) comes through a [`Context`].
//!
//! The solver's answers are cached on disk under `.cache/plans-rs` (see [`PlanCache`]).

mod build;
mod cache;
mod context;
mod finish;
mod market;
mod recombs;
mod run;
mod solving;
mod tree;
mod types;

pub use build::{ClusterOpts, GearOpts, add_fractures, estimate_cluster, estimate_gear, estimate_item};
pub use cache::{PLAN_VERSION, PlanCache, plan_key, round_sig};
pub use context::{Context, Progress, settings_from_env};
pub use finish::{Finish, find_starts, finish, finishes, listing_to_item};
pub use market::{Lookup, lookup, lookup_market, price_fractures, price_on_trade};
pub use recombs::add_recombs;
pub use run::{LadderOpts, RunOpts, build_name, current_league, expand_sources, ladder, load_build, run};
pub use solving::{SolveSlot, Solvable, slots_of, solve_all, solve_one, solve_slots};
pub use tree::{MAX_SALES, MIN_SALE, SALE_SHARE, add_sales, covers, link_products, short_name};
pub use types::{
    BuildInfo, Estimate, Fracture, JEWELLERY, Key, LadderReport, ProductRef, RecombInfo, Report, Search, Shared, Slot, cmp_ladder, cmp_priority, priority,
    snapshot_products,
};
