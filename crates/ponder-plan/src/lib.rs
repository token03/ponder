//! Crafting problems for the solver (SPEC §6, planner): one item kind and goal per craft;
//! crafts feed each other through supplies.

pub mod craft;
pub mod inspect;
mod lower;
pub mod market;
pub mod recipe;
pub mod session;
pub mod supply;
