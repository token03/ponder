//! Pricing every route to an item, per item, cheapest-looking first.
//!
//! A route is one way to make the finished item: craft it directly, finish a fractured base,
//! prepare one side and protect it, recombine clean donors, or join two influenced donors with
//! an Awakener's Orb. Each item's routes are priced in one task, in an order that finds a good
//! complete route early; every later search carries that route's cost as a ceiling, so a route
//! that cannot win stops (or never starts) instead of being solved in full. Shared components
//! (donors, preparation stages) are solved once per process through `Context::solves`.

use ponder_core::Prices;
use ponder_game::mods::ModDB;

use super::context::{Context, Progress};
use super::solving::{for_each_largest_first, report_solved, solve_direct_and_fractures};
use super::types::Estimate;

/// Price every route of every estimate. Routes already priced are kept, so this can run again
/// after trade prices arrive (a fractured base's price changes its ceiling).
pub fn price_routes(
    cx: &Context,
    db: &ModDB,
    estimates: &mut [Estimate],
    table: &Prices,
    league: &str,
    workers: Option<usize>,
    progress: Option<Progress>,
) {
    if let Some(p) = progress {
        p(&format!("pricing {} crafts: direct, fractured, protected, recombined and Awakener routes", estimates.len()));
    }
    for_each_largest_first(estimates, workers, |estimate| {
        let started = std::time::Instant::now();
        let searched = solve_direct_and_fractures(cx, estimate, table);
        super::recombs::price_donor_routes(cx, db, estimate, table, league);
        if searched {
            report_solved(estimate, started, progress);
        }
    });
    cx.solves.clear();
}
