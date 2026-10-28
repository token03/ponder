//! Goals compiled against game data: the predicate the solver and simulator test items with.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use crate::game::{BaseId, Game, Influences, Kind, ModId, Side};
use crate::goal::{Goal, Rarity, Source};
use crate::item::Item;

/// One compiled `ModReq`: `count` distinct affixes from `ok`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Need {
    pub ok: HashSet<ModId>,
    pub count: u8,
    pub fractured: bool,
}

impl Hash for Need {
    fn hash<H: Hasher>(&self, state: &mut H) {
        let mut mods: Vec<_> = self.ok.iter().copied().collect();
        mods.sort_unstable();
        mods.hash(state);
        self.count.hash(state);
        self.fractured.hash(state);
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Target {
    pub base: Option<BaseId>,
    pub ilvl: u8,
    pub quality: u8,
    pub influences: Influences,
    pub rarity: Option<Rarity>,
    pub needs: Vec<Need>,
    pub open: [u8; 2],
    pub max_side: [Option<u8>; 2],
    pub max_mods: Option<u8>,
    pub min_mods: u8,
}

impl Target {
    /// Resolve a goal's mod templates and tiers. The goal must name its base (guides fill it in).
    pub fn compile(game: &Game, goal: &Goal) -> Result<Target, String> {
        let name = goal.base.as_deref().ok_or("goal has no base")?;
        let base = game.base(name).ok_or_else(|| format!("unknown base {name:?}"))?;
        let influences = Influences::of(&goal.influences);
        let kind = Kind { base, ilvl: goal.ilvl.unwrap_or(100), influences };
        let mut needs = Vec::new();
        for req in &goal.mods {
            let mut ok = HashSet::new();
            for text in &req.text {
                let mods = game.with_template(text);
                if mods.is_empty() {
                    return Err(format!("no mod has template {text:?}"));
                }
                for &m in mods {
                    let source = game.def(m).source;
                    let fits = match (req.source, req.tier) {
                        (Some(s), _) if s != source => false,
                        (None, _) if source == Source::Crafted => false,
                        (_, Some(t)) if matches!(source, Source::Natural | Source::Influence | Source::Essence) => {
                            game.tier(kind, m).is_some_and(|x| x <= t)
                        }
                        (None, Some(_)) => false,
                        _ => true,
                    };
                    if fits {
                        ok.insert(m);
                    }
                }
            }
            if ok.is_empty() {
                return Err(format!("nothing satisfies {:?} on {name}", req.text));
            }
            needs.push(Need { ok, count: req.count, fractured: req.fractured });
        }
        Ok(Target {
            base: Some(base),
            ilvl: goal.ilvl.unwrap_or(0),
            quality: goal.quality.unwrap_or(0),
            influences,
            rarity: goal.rarity,
            needs,
            open: [goal.open_prefix, goal.open_suffix],
            max_side: [goal.max_prefix, goal.max_suffix],
            max_mods: goal.max_mods,
            min_mods: goal.min_mods.unwrap_or(0),
        })
    }

    pub fn holds(&self, game: &Game, item: &Item) -> bool {
        let count = Side::BOTH.map(|s| item.count(game, s));
        let open = Side::BOTH.map(|s| item.open(game, s));
        self.base.is_none_or(|b| b == item.kind.base)
            && item.kind.ilvl >= self.ilvl
            && item.quality >= self.quality
            && item.kind.influences.0 & self.influences.0 == self.influences.0
            && self.shape(item.rarity, count, open)
            && matches(&self.needs, item.affixes.len(), &|need, i| {
                let (need, a) = (&self.needs[need], &item.affixes[i]);
                need.ok.contains(&a.id) && (!need.fractured || a.fractured)
            })
    }

    /// The plainest item meeting the target, as bought: rare (unless the target says otherwise)
    /// with the lowest acceptable tier of each needed mod the base can roll, and nothing else.
    pub fn witness(&self, game: &Game, kind: Kind) -> Option<Item> {
        let mut item = Item::new(kind);
        item.rarity = self.rarity.unwrap_or(if self.needs.is_empty() { Rarity::Normal } else { Rarity::Rare });
        item.quality = self.quality;
        let pool = game.pool(kind);
        for need in &self.needs {
            let rolls = |m: &ModId| {
                matches!(game.def(*m).source, Source::Crafted | Source::Essence | Source::Legacy | Source::Elevated)
                    || pool.mods.iter().any(|(id, _)| id == m)
            };
            let mut ok: Vec<ModId> = need.ok.iter().copied().filter(rolls).collect();
            ok.sort_by_key(|m| (game.def(*m).level, m.0));
            for _ in 0..need.count {
                let m = ok.iter().copied().find(|&m| item.open(game, game.def(m).side) > 0 && !item.blocks(game, m))?;
                let crafted = game.def(m).source == Source::Crafted;
                item.affixes.push(crate::item::Affix { id: m, crafted, fractured: need.fractured });
            }
        }
        self.holds(game, &item).then_some(item)
    }

    /// The rarity and affix-count conditions.
    pub fn shape(&self, rarity: Rarity, count: [usize; 2], open: [usize; 2]) -> bool {
        let n = count[0] + count[1];
        self.rarity.is_none_or(|r| r == rarity)
            && self.max_mods.is_none_or(|m| n <= m as usize)
            && n >= self.min_mods as usize
            && (0..2).all(|s| open[s] >= self.open[s] as usize && self.max_side[s].is_none_or(|m| count[s] <= m as usize))
    }
}

/// Can every need take its count of distinct affixes, where `fits(need, affix)` says whether an
/// affix serves a need? Backtracking: items have at most six affixes.
pub fn matches(needs: &[Need], affixes: usize, fits: &impl Fn(usize, usize) -> bool) -> bool {
    fn go(needs: &[Need], at: usize, left: u8, from: usize, used: &mut [bool], fits: &impl Fn(usize, usize) -> bool) -> bool {
        if at == needs.len() {
            return true;
        }
        if left == 0 {
            return at + 1 == needs.len() || go(needs, at + 1, needs[at + 1].count, 0, used, fits);
        }
        for i in from..used.len() {
            if !used[i] && fits(at, i) {
                used[i] = true;
                if go(needs, at, left - 1, i + 1, used, fits) {
                    return true;
                }
                used[i] = false;
            }
        }
        false
    }
    needs.is_empty() || go(needs, 0, needs[0].count, 0, &mut vec![false; affixes], fits)
}
