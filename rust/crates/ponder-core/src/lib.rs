//! Crafting solver: lumped item states, currency outcomes, policy iteration. No I/O.
//!
//! A port of `src/ponder/craft.py`, with the same names. The item is lumped into a small
//! state (rarity, each target missing / good / blocked, other mods per slot); every
//! currency maps a state to a distribution over states; policy iteration finds, for every
//! state, the action minimising the expected cost to finish.

mod problem;
mod simulate;
mod solve;
mod steps;
mod types;

pub use problem::{Problem, RollKey};
pub use simulate::{Rng, simulate};
pub use solve::{
    FOSSIL_SECONDS, FRACTURE_MIN_MODS, FRACTURE_ORB, FRACTURE_SECONDS, HARVEST_SECONDS, LOCK_SECONDS, Plan, SECONDS, SELL_SECONDS,
    SelfFracture, SolveError, StateMap, action_cost, evaluate, seconds, self_fracture, solve, solve_rounds, step_cost,
};
pub use steps::{Exit, Step, describe, describe_parts, recipe, recipe_states, steps};
pub use types::*;

#[cfg(test)]
mod tests;
