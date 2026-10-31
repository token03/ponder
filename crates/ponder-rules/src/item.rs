//! A concrete item: what the game would show.

use crate::game::{BaseId, Game, Kind, ModId, Side};
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
    /// Memory strands, up to [`crate::game::MAX_STRANDS`]: they keep low tiers from rolling.
    pub strands: u8,
    /// The item as an unused imprint of it saved it.
    pub imprint: Option<Box<Item>>,
    /// A Hinekora's Lock shows what the next currency will do (and is used up by it).
    pub foreseen: bool,
}

impl Item {
    pub fn new(kind: Kind) -> Item {
        Item { kind, rarity: Rarity::Normal, quality: 0, affixes: Vec::new(), eldritch: None, strands: 0, imprint: None, foreseen: false }
    }

    pub fn count(&self, game: &Game, side: Side) -> usize {
        self.affixes.iter().filter(|a| game.def(a.id).side == side).count()
    }

    /// Most affixes a side can hold at the item's rarity (SPEC §4.1).
    pub fn limit(&self, game: &Game, side: Side) -> usize {
        limit(game, self.kind.base, self.rarity, side)
    }

    pub fn open(&self, game: &Game, side: Side) -> usize {
        self.limit(game, side).saturating_sub(self.count(game, side))
    }

    /// Does any affix share a mod group with `m` (SPEC §4.2)?
    pub fn blocks(&self, game: &Game, m: ModId) -> bool {
        let groups = &game.def(m).groups;
        self.affixes.iter().any(|a| game.def(a.id).groups.iter().any(|g| groups.contains(g)))
    }

    /// The sides "Cannot Be Changed" metamods protect.
    pub fn locked(&self, game: &Game) -> [bool; 2] {
        Side::BOTH.map(|side| game.metamods.get(side as usize).is_some_and(|m| self.affixes.iter().any(|a| a.id == *m)))
    }

    pub fn crafted(&self) -> usize {
        self.affixes.iter().filter(|a| a.crafted).count()
    }
}

/// Most affixes a side holds on a base at a rarity: the rarity's, changed by what the base's
/// implicit allows (SPEC §10: at every rarity, never below none).
pub fn limit(game: &Game, base: BaseId, rarity: Rarity, side: Side) -> usize {
    let def = &game.bases[base.0 as usize];
    let usual: i8 = match rarity {
        Rarity::Normal => return 0,
        Rarity::Magic => 1,
        Rarity::Rare if def.class.contains("Jewel") => 2,
        Rarity::Rare => 3,
    };
    (usual + def.extra_affixes[side as usize]).max(0) as usize
}

/// The rarity an item settles at after losing mods: the lowest that holds `counts` (per side).
pub fn settled(game: &Game, base: BaseId, counts: [usize; 2]) -> Rarity {
    [Rarity::Normal, Rarity::Magic]
        .into_iter()
        .find(|&rarity| Side::BOTH.iter().all(|&side| counts[side as usize] <= limit(game, base, rarity, side)))
        .unwrap_or(Rarity::Rare)
}

impl Side {
    pub const BOTH: [Side; 2] = [Side::Prefix, Side::Suffix];
}
