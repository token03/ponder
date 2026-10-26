//! Numbers the game data does not give us (SPEC §10). One place, all named.

use serde::Deserialize;

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Assumptions {
    /// Total mod count when an item becomes rare, as (count, relative weight).
    /// 1,000-chaos sample (674 / 239 / 87), Reddit 2020. Medium confidence.
    pub rare_mods: Vec<(u8, f64)>,
    /// The same for jewels. Unverified.
    pub rare_mods_jewel: Vec<(u8, f64)>,
    /// Mod count when an item becomes or rerolls magic. Unverified.
    pub magic_mods: Vec<(u8, f64)>,
    /// Recombinators, per side: indexed by how many affixes both items pool there, how many
    /// the result keeps, as (kept, relative weight). Pools past the end use the last row.
    /// Craft of Exile's simulator. Medium confidence.
    pub recombine_kept: Vec<Vec<(u8, f64)>>,
    /// Chance a recombined item with room gains one more random mod. Craft of Exile's
    /// simulator, which also makes a tenth of these a recombinator-only mod (not modelled).
    pub recombine_bonus: f64,
}

impl Default for Assumptions {
    fn default() -> Self {
        Assumptions {
            rare_mods: vec![(4, 9.0), (5, 3.0), (6, 1.0)],
            rare_mods_jewel: vec![(3, 3.0), (4, 1.0)],
            magic_mods: vec![(1, 1.0), (2, 1.0)],
            recombine_kept: vec![
                vec![(0, 1.0)],
                vec![(0, 1.0), (1, 2.0)],
                vec![(1, 2.0), (2, 1.0)],
                vec![(1, 3.0), (2, 5.0), (3, 2.0)],
                vec![(1, 2.0), (2, 11.0), (3, 7.0)],
                vec![(2, 1.0), (3, 1.0)],
                vec![(2, 3.0), (3, 7.0)],
            ],
            recombine_bonus: 0.25,
        }
    }
}
