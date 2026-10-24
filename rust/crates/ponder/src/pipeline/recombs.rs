//! Recombinator trees for gear estimates (waiting for recomb.rs).

use ponder_core::Prices;
use ponder_game::mods::ModDB;

use super::context::{Context, Progress};
use super::types::Estimate;

pub fn add_recombs(_cx: &Context, _db: &ModDB, _estimates: &mut [Estimate], _table: &Prices, _workers: Option<usize>, _progress: Option<Progress>) {}
