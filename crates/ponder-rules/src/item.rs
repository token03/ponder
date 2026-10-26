//! A concrete item: what the game would show.

use crate::game::{Game, Kind, ModId, Side};
use crate::goal::Rarity;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Affix {
    pub id: ModId,
    pub crafted: bool,
    pub fractured: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Item {
    pub kind: Kind,
    pub rarity: Rarity,
    pub quality: u8,
    pub affixes: Vec<Affix>,
    /// The side the dominant Eldritch implicit makes Eldritch currency work on.
    pub eldritch: Option<Side>,
}

impl Item {
    pub fn new(kind: Kind) -> Item {
        Item { kind, rarity: Rarity::Normal, quality: 0, affixes: Vec::new(), eldritch: None }
    }

    pub fn count(&self, game: &Game, side: Side) -> usize {
        self.affixes.iter().filter(|a| game.def(a.id).side == side).count()
    }

    /// Most affixes a side can hold at the item's rarity (SPEC §4.1).
    pub fn limit(&self, game: &Game) -> usize {
        let jewel = game.bases[self.kind.base.0 as usize].class.contains("Jewel");
        match self.rarity {
            Rarity::Normal => 0,
            Rarity::Magic => 1,
            Rarity::Rare if jewel => 2,
            Rarity::Rare => 3,
        }
    }

    pub fn open(&self, game: &Game, side: Side) -> usize {
        self.limit(game).saturating_sub(self.count(game, side))
    }

    /// Does any affix share a mod group with `m` (SPEC §4.2)?
    pub fn blocks(&self, game: &Game, m: ModId) -> bool {
        let groups = &game.def(m).groups;
        self.affixes.iter().any(|a| game.def(a.id).groups.iter().any(|g| groups.contains(g)))
    }

    /// The side a "Cannot Be Changed" metamod protects.
    pub fn locked(&self, game: &Game) -> Option<Side> {
        self.affixes.iter().find_map(|a| match game.metamods.iter().position(|m| *m == a.id) {
            Some(0) => Some(Side::Prefix),
            Some(1) => Some(Side::Suffix),
            _ => None,
        })
    }

    pub fn crafted(&self) -> usize {
        self.affixes.iter().filter(|a| a.crafted).count()
    }
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Prefix, Side::Suffix];
}
