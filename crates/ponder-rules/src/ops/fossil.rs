//! Fossils in a resonator: reforge as rare with reweighted mods, plus any mods the fossils
//! add or force. Lucky values (Sanctified) do not change tiers and are ignored.

use std::sync::Arc;

use super::{Bias, Cost, Ctx, Operation, Pick, Pool, Sides, Step, View};
use crate::game::{BaseId, Game, Influences, Kind, ModId};
use crate::goal::Rarity;

const RESONATORS: [&str; 4] = ["primitive-chaotic-resonator", "potent-chaotic-resonator", "powerful-chaotic-resonator", "prime-chaotic-resonator"];

pub struct Fossils {
    name: String,
    steps: Vec<Step>,
    cost: Cost,
}

impl Fossils {
    /// `jagged+metallic+shuddering+corroded`: one to four fossils. Forced mods (Hollow's
    /// abyssal socket) apply only where the item class can spawn them.
    pub fn new(game: &Game, arg: &str, class: &str) -> Option<Fossils> {
        let names: Vec<&str> = arg.split('+').collect();
        if names.is_empty() || names.len() > RESONATORS.len() {
            return None;
        }
        let mut tags: Vec<(crate::game::TagId, f64, f64)> = Vec::new();
        let mut added = Vec::new();
        let mut forced = Vec::new();
        let mut items = vec![(RESONATORS[names.len() - 1].to_string(), 1.0)];
        for n in &names {
            let fossil = game.fossils.iter().find(|f| f.name.eq_ignore_ascii_case(&format!("{n} fossil")))?;
            for w in fossil.positive_mod_weights.iter().chain(&fossil.negative_mod_weights) {
                let factor = w.weight as f64 / 100.0;
                let t = game.tag_id(&w.tag)?;
                let entry = match tags.iter_mut().position(|e| e.0 == t) {
                    Some(i) => &mut tags[i],
                    None => {
                        tags.push((t, 0.0, 1.0));
                        tags.last_mut().unwrap()
                    }
                };
                if factor > 1.0 {
                    entry.1 += factor;
                } else {
                    entry.2 *= factor;
                }
            }
            added.extend(fossil.added_mods.iter().filter_map(|k| game.mod_id(k)));
            forced.extend(fossil.forced_mods.iter().filter_map(|k| game.mod_id(k)).filter(|m| spawns(game, class, *m)));
            items.push((format!("{n}-fossil"), 1.0));
        }
        let bias = Arc::new(Bias { name: arg.to_string(), tags, added });
        let mut steps = vec![Step::Clear { sides: Sides::Both, respect_lock: false }, Step::Rarity(Rarity::Rare), Step::RollCount];
        steps.extend(forced.into_iter().map(Step::Force));
        steps.push(Step::Fill(Pick { sides: Sides::Both, tag: None, pool: Pool::Natural(Some(bias)) }));
        Some(Fossils { name: arg.to_string(), steps, cost: Cost { items, seconds: 4.0 } })
    }
}

/// Can some base of `class` spawn `m`?
fn spawns(game: &Game, class: &str, m: ModId) -> bool {
    (0..game.bases.len()).filter(|&b| game.bases[b].class == class).any(|b| {
        let kind = Kind { base: BaseId(b as u32), ilvl: 100, influences: Influences::default() };
        game.pool(kind).mods.iter().any(|(id, _)| *id == m)
    })
}

impl Operation for Fossils {
    fn name(&self) -> String {
        format!("fossil:{}", self.name)
    }

    /// Normal or rare items without a bench metamod (SPEC §4.7), unless a fractured mod
    /// blocks a forced one.
    fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
        let blocked = self.steps.iter().any(|s| matches!(s, Step::Force(m) if item.fixed_blocks(*m)));
        matches!(item.rarity(), Rarity::Normal | Rarity::Rare) && !item.has_metamod() && !blocked
    }

    fn steps(&self) -> &[Step] {
        &self.steps
    }

    fn cost(&self) -> Cost {
        self.cost.clone()
    }
}
