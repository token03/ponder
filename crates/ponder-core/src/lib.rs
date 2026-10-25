//! Crafting solver: lumped item states, currency outcomes, policy iteration. No I/O.
//!
//! The item is lumped into a small
//! state (rarity, each target missing / good / blocked, other mods per slot); every
//! currency maps a state to a distribution over states; policy iteration finds, for every
//! state, the action minimising the expected cost to finish.
//!
mod action;
mod fracture;
mod linear;
mod mechanics;
mod problem;
pub mod search;
mod simulate;
mod solve;
mod steps;
mod transition;
mod types;

pub use action::{Action, ActionKind, Effect, FOSSIL_SECONDS, action_cost, step_cost};
pub use fracture::{FRACTURE_MIN_MODS, FRACTURE_ORB, FRACTURE_SECONDS, SelfFracture, self_fracture};
pub use linear::Backend;
pub use mechanics::Rng;
pub use problem::{Problem, ProblemSpec, RollKey};
pub use search::Options as SolveOptions;
pub use simulate::{replay, replay_with_meta, simulate};
pub use solve::{
    Bounds, Plan, Preservation, Recovery, SaleRecovery, SolveError, SolveLimits, StateMap, evaluate, evaluate_with,
    follow_after_acquisition, follow_from, follow_with_recovery, solve, solve_after_acquisition, solve_from, solve_preserving_side,
    solve_with, solve_with_recovery,
};
pub use steps::{Exit, Step, describe, describe_parts, recipe, recipe_states, steps};
pub use transition::{Distribution, ProbabilityRange, TransitionBuffer, TransitionResult};
pub use types::*;

#[cfg(test)]
mod tests;
