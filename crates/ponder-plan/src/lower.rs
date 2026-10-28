use std::collections::HashMap;

use ponder_rules::goal::Rarity;
use ponder_rules::lens::{SideRelaxation, SideState};
use ponder_rules::ops::currency::Currency;
use ponder_rules::ops::{Pick, Step};
use ponder_solve::Model;

use crate::craft::{Craft, Node};

const RESTART: usize = usize::MAX;
const RARITIES: [Rarity; 3] = [Rarity::Normal, Rarity::Magic, Rarity::Rare];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Frame {
    Market,
    Restart,
    Item(SideState),
    Run(usize, usize, SideState),
    Fill(usize, usize, u8, SideState),
    Select(usize, usize, SideState),
    Settle(usize, usize, SideState),
}

impl Frame {
    fn state(&self) -> Option<SideState> {
        match *self {
            Self::Item(s) | Self::Run(_, _, s) | Self::Fill(_, _, _, s) | Self::Settle(_, _, s) => Some(s),
            _ => None,
        }
    }

    fn with(&self, s: SideState) -> Self {
        match *self {
            Self::Item(_) => Self::Item(s),
            Self::Run(i, k, _) => Self::Run(i, k, s),
            Self::Fill(i, k, n, _) => Self::Fill(i, k, n, s),
            Self::Settle(i, k, _) => Self::Settle(i, k, s),
            _ => self.clone(),
        }
    }
}

pub(crate) struct SideBound {
    relaxation: SideRelaxation,
    values: HashMap<Frame, f64>,
    hubs: Vec<Option<(usize, usize)>>,
    market: f64,
}

impl SideBound {
    pub(crate) fn new(craft: &Craft<'_>, limit: usize) -> Option<Self> {
        if !craft.ops.iter().any(|(op, _)| op.steps().iter().any(|s| matches!(s, Step::Recombine(_)))) {
            return None;
        }
        let relaxation = SideRelaxation::new(craft.lens, craft.goal)?;
        if relaxation.required() != 3 {
            return None;
        }
        let mut donors = HashMap::new();
        for (i, (op, _)) in craft.ops.iter().enumerate() {
            for (k, step) in op.steps().iter().enumerate() {
                let Step::Recombine(partner) = step else { continue };
                let mut dist = HashMap::<SideState, f64>::new();
                for item in partner.0.items() {
                    *dist.entry(relaxation.project(&craft.lens.project(item))).or_default() += 1.0 / partner.0.items().len() as f64;
                }
                donors.insert((i, k), dist.into_iter().map(|(s, p)| (p, s)).collect());
            }
        }
        let programs = craft
            .ops
            .iter()
            .enumerate()
            .map(|(i, (op, _))| {
                (0..=op.steps().len())
                    .map(|k| {
                        if k == op.steps().len() {
                            return (0, craft.ops[0].0.steps().len());
                        }
                        (0..=i)
                            .find_map(|j| {
                                let steps = craft.ops[j].0.steps();
                                let begin = steps.len().checked_sub(op.steps().len() - k)?;
                                (steps[begin..] == op.steps()[k..]).then_some((j, begin))
                            })
                            .unwrap()
                    })
                    .collect()
            })
            .collect();
        let model = Relaxed { craft, relaxation: &relaxation, donors, programs };
        let solution = ponder_solve::solve_everywhere(&model, &[(1.0, Frame::Market)], limit).ok()?;
        let hubs = (0..craft.ops.len()).map(|i| craft.continuation(i).map(|k| model.programs[i][k])).collect();
        let market = solution.cost;
        let values = solution
            .states
            .into_iter()
            .zip(solution.values)
            .filter_map(|(s, v)| match s {
                frame @ (Frame::Item(_) | Frame::Run(..)) => Some((frame, v.max(0.0))),
                _ => None,
            })
            .collect();
        Some(Self { relaxation, values, hubs, market })
    }

    pub(crate) fn value(&self, node: &Node) -> f64 {
        let frame = match node {
            Node::Market | Node::Restart => return self.market,
            Node::Item(s) => Frame::Item(self.relaxation.project(s)),
            Node::Hub(i, s) => {
                let Some(Some((op, step))) = self.hubs.get(*i) else { return 0.0 };
                Frame::Run(*op, *step, self.relaxation.project(s))
            }
            Node::Goal => return 0.0,
        };
        self.values.get(&frame).copied().unwrap_or(0.0)
    }
}

struct Relaxed<'a, 'g> {
    craft: &'a Craft<'g>,
    relaxation: &'a SideRelaxation,
    donors: HashMap<(usize, usize), Vec<(f64, SideState)>>,
    programs: Vec<Vec<(usize, usize)>>,
}

impl Relaxed<'_, '_> {
    fn erasing(&self) -> usize {
        self.relaxation.contexts().max(self.craft.lens.classes() + 2).max(RARITIES.len())
    }

    fn unfixing(&self) -> usize {
        self.erasing() + 2 * self.craft.lens.classes()
    }

    fn operating(&self) -> usize {
        self.unfixing() + self.craft.lens.classes()
    }

    fn next(&self, i: usize, k: usize, s: SideState) -> Frame {
        if k == self.craft.ops[i].0.steps().len() {
            Frame::Item(s)
        } else {
            let (i, k) = self.programs[i][k];
            Frame::Run(i, k, s)
        }
    }

    fn settle(&self, i: usize, k: usize, s: SideState) -> Frame {
        let (i, k) = self.programs[i][k];
        Frame::Settle(i, k, s)
    }

    fn rarities(&self, s: SideState) -> impl Iterator<Item = usize> {
        let count = self.relaxation.count(s);
        (0..3).filter(move |r| count <= [0, 1, 3][*r])
    }
}

impl Model for Relaxed<'_, '_> {
    type State = Frame;

    fn is_goal(&self, frame: &Frame) -> bool {
        frame.state().is_some_and(|s| self.relaxation.holds(s))
    }

    fn is_restart(&self, frame: &Frame) -> bool {
        *frame == Frame::Restart
    }

    fn restarts_anywhere(&self) -> bool {
        true
    }

    fn actions(&self, frame: &Frame) -> Vec<(usize, f64)> {
        let Some(s) = frame.state() else {
            return match frame {
                Frame::Market => self.craft.starts.iter().enumerate().map(|(i, b)| (i, b.price())).collect(),
                Frame::Select(..) => vec![(0, 0.0), (1, 0.0), (RESTART, 0.0)],
                _ => Vec::new(),
            };
        };
        let mut actions: Vec<_> = match *frame {
            Frame::Item(_) => self
                .craft
                .ops
                .iter()
                .enumerate()
                .filter(|(_, (op, _))| match Currency::parse(&op.name()) {
                    Some(Currency::Transmutation | Currency::Alchemy) => s.state.rarity == Rarity::Normal,
                    Some(Currency::Alteration | Currency::Augmentation | Currency::Regal) => s.state.rarity == Rarity::Magic,
                    Some(Currency::Chaos | Currency::Exalted) => s.state.rarity == Rarity::Rare,
                    Some(Currency::Fracturing) => s.state.rarity == Rarity::Rare && self.relaxation.fractured(s) == 0,
                    Some(Currency::Annulment | Currency::Scouring) => s.state.rarity != Rarity::Normal,
                    None => true,
                })
                .map(|(i, (_, price))| (self.operating() + i, *price))
                .collect(),
            Frame::Settle(..) => self.rarities(s).map(|i| (i, 0.0)).collect(),
            Frame::Fill(_, _, n, _) if n > 0 => (0..self.relaxation.contexts()).map(|i| (i, 0.0)).collect(),
            Frame::Run(i, k, _) => match &self.craft.ops[i].0.steps()[k] {
                Step::Settle => self.rarities(s).map(|i| (i, 0.0)).collect(),
                Step::Draw(_) | Step::Unveil(_) | Step::Swap { .. } => (0..self.relaxation.contexts()).map(|i| (i, 0.0)).collect(),
                Step::RecombineBonus if self.craft.ctx.assumptions.recombine_bonus > 0.0 => {
                    (0..self.relaxation.contexts()).map(|i| (i, 0.0)).collect()
                }
                Step::Craft(mods) => (0..self.relaxation.craft(self.craft.lens, s, mods).len()).map(|i| (i, 0.0)).collect(),
                _ => vec![(0, 0.0)],
            },
            _ => vec![(0, 0.0)],
        };
        let n = self.craft.lens.classes();
        for c in 0..2 * n {
            if s.state.counts[c] == 0 {
                continue;
            }
            actions.push((self.erasing() + c, 0.0));
            if c >= n {
                actions.push((self.unfixing() + c - n, 0.0));
            }
        }
        actions.push((RESTART, 0.0));
        actions
    }

    fn outcomes(&self, frame: &Frame, action: usize) -> Vec<(f64, Frame)> {
        if action == RESTART {
            return vec![(1.0, Frame::Restart)];
        }
        if frame.state().is_some() && (self.erasing()..self.unfixing()).contains(&action) {
            return vec![(1.0, frame.with(self.relaxation.erase(frame.state().unwrap(), action - self.erasing())))];
        }
        if frame.state().is_some() && (self.unfixing()..self.operating()).contains(&action) {
            return vec![(1.0, frame.with(self.relaxation.unfix(frame.state().unwrap(), action - self.unfixing())))];
        }
        let (lens, ctx) = (self.craft.lens, self.craft.ctx);
        match *frame {
            Frame::Market => self.craft.start_states(action).iter().map(|(p, s)| (*p, Frame::Item(self.relaxation.project(s)))).collect(),
            Frame::Restart => Vec::new(),
            Frame::Item(s) => vec![(1.0, self.next(action - self.operating(), 0, s))],
            Frame::Select(i, k, pool) => {
                self.relaxation.sample(lens, ctx, pool, action != 0).into_iter().map(|(p, s)| (p, self.settle(i, k + 1, s))).collect()
            }
            Frame::Settle(i, k, mut s) => {
                s.state.rarity = RARITIES[action];
                vec![(1.0, self.next(i, k, s))]
            }
            Frame::Fill(i, k, 0, s) => vec![(1.0, self.next(i, k + 1, s))],
            Frame::Fill(i, k, n, s) => {
                let Step::Fill(pick) = &self.craft.ops[i].0.steps()[k] else { unreachable!() };
                self.relaxation.draw(lens, ctx, s, pick, action).into_iter().map(|(p, s)| (p, Frame::Fill(i, k, n - 1, s))).collect()
            }
            Frame::Run(i, k, mut s) => {
                match &self.craft.ops[i].0.steps()[k] {
                    Step::Clear { sides, respect_lock } => s = self.relaxation.clear(s, *sides, *respect_lock),
                    Step::Rarity(r) => s.state.rarity = *r,
                    Step::Settle => s.state.rarity = RARITIES[action],
                    Step::Force(m) => s = self.relaxation.force(lens, s, *m),
                    Step::Craft(mods) => s = self.relaxation.craft(lens, s, mods)[action],
                    Step::Draw(pick) => {
                        return self
                            .relaxation
                            .draw(lens, ctx, s, pick, action)
                            .into_iter()
                            .map(|(p, s)| (p, self.next(i, k + 1, s)))
                            .collect();
                    }
                    Step::Fill(_) => {
                        let limit: u8 = if s.state.rarity == Rarity::Rare { 3 } else { 1 };
                        return vec![(1.0, Frame::Fill(i, k, limit.saturating_sub(self.relaxation.count(s) as u8), s))];
                    }
                    Step::Fracture => {
                        return self.relaxation.fracture(lens, ctx, s).into_iter().map(|(p, s)| (p, self.next(i, k + 1, s))).collect();
                    }
                    Step::Veil(sides) => {
                        return self.relaxation.veil(lens, ctx, s, *sides).into_iter().map(|(p, s)| (p, self.next(i, k + 1, s))).collect();
                    }
                    step @ Step::Unveil(_) => {
                        return self
                            .relaxation
                            .run(lens, ctx, s, step, action)
                            .into_iter()
                            .map(|(p, s)| (p, self.next(i, k + 1, s)))
                            .collect();
                    }
                    Step::Recombine(_) => {
                        let mut out = HashMap::<Frame, f64>::new();
                        for &(p, donor) in &self.donors[&(i, k)] {
                            for heir in [s, donor] {
                                *out.entry(Frame::Select(i, k, self.relaxation.pool(s, donor, heir))).or_default() += p / 2.0;
                            }
                        }
                        return out.into_iter().map(|(s, p)| (p, s)).collect();
                    }
                    Step::RecombineBonus if ctx.assumptions.recombine_bonus > 0.0 => {
                        s.state.rarity = Rarity::Rare;
                        return self
                            .relaxation
                            .draw(lens, ctx, s, &Pick::ANY, action)
                            .into_iter()
                            .map(|(p, s)| (p, self.settle(i, k + 1, s)))
                            .collect();
                    }
                    Step::RollCount | Step::Remove { .. } | Step::RemoveCrafted | Step::RecombineBonus => {}
                    Step::Swap { add, .. } => {
                        return self
                            .relaxation
                            .draw(lens, ctx, s, add, action)
                            .into_iter()
                            .map(|(p, s)| (p, self.next(i, k + 1, s)))
                            .collect();
                    }
                    step @ (Step::Influence(_) | Step::Eldritch(_)) => {
                        return self.relaxation.run(lens, ctx, s, step, 0).into_iter().map(|(p, s)| (p, self.next(i, k + 1, s))).collect();
                    }
                }
                vec![(1.0, self.next(i, k + 1, s))]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::Arc;

    use ponder_rules::assumptions::Assumptions;
    use ponder_rules::data::Repoe;
    use ponder_rules::game::{Game, Influences, Kind, ModId, Side};
    use ponder_rules::goal::Source;
    use ponder_rules::guide::Guide;
    use ponder_rules::item::{Affix, Item};
    use ponder_rules::lens::Lens;
    use ponder_rules::ops::recombinator::Recombinator;
    use ponder_rules::ops::{Cost, Ctx, Operation, Sides, Supply, View, parse, run};
    use ponder_rules::rng::Rng;
    use ponder_rules::target::Target;

    use crate::market::Market;
    use crate::supply::Stock;

    struct Prepare {
        steps: Vec<Step>,
        fire: ModId,
    }

    impl Operation for Prepare {
        fn name(&self) -> String {
            "prepare".into()
        }
        fn applies(&self, _: &Ctx, item: &dyn View) -> bool {
            !item.has(self.fire) && (item.rarity() == Rarity::Normal || item.room(Sides::Only(Side::Prefix)))
        }
        fn steps(&self) -> &[Step] {
            &self.steps
        }
        fn cost(&self) -> Cost {
            Cost::of("chaos", 1.0, 0.0)
        }
    }

    #[test]
    fn continuation_bounds_are_below_the_exact_full_state_values() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let game = Game::new(Repoe::load(&root.join("fixtures/repoe")).unwrap());
        let guide: Guide = toml::from_str(&std::fs::read_to_string(root.join("fixtures/guides/cheap-ele-bow.toml")).unwrap()).unwrap();
        let mut goal = Target::compile(&game, &guide.target("goal").unwrap()).unwrap();
        goal.needs.retain(|n| n.ok.iter().all(|&m| game.def(m).side == Side::Prefix));
        assert_eq!(goal.needs.len(), 3);
        let kind = Kind { base: game.base("Spine Bow").unwrap(), ilvl: 82, influences: Influences::default() };
        let assumptions = Assumptions::default();
        let ctx = Ctx { game: &game, assumptions: &assumptions };
        let market =
            Market { prices: HashMap::from([("chaos".into(), 1.0), ("weapon-recombinator".into(), 10.0)]), base: 5.0, chaos_per_hour: 0.0 };
        let native: Vec<_> = goal
            .needs
            .iter()
            .map(|need| {
                need.ok
                    .iter()
                    .copied()
                    .find(|&m| {
                        game.def(m).source == Source::Natural
                            && game.def(m).level <= 82
                            && game.pool(kind).mods.iter().any(|&(id, _)| id == m)
                    })
                    .unwrap()
            })
            .collect();
        let fire = native[0];
        let mut donor = Item::new(kind);
        donor.rarity = Rarity::Rare;
        donor.affixes = native[1..].iter().map(|&id| Affix { id, fractured: false, crafted: false }).collect();
        let donor = Arc::new(Stock::new("donor", 150.0, donor));
        let prepare = Prepare {
            steps: vec![
                Step::Rarity(Rarity::Rare),
                Step::Force(fire),
                Step::Draw(Pick { sides: Sides::Only(Side::Suffix), ..Pick::ANY }),
                Step::Settle,
            ],
            fire,
        };
        let recombine = Recombinator::new("Bow", "test", donor.clone());
        let ops: Vec<&dyn Operation> = vec![&prepare, &recombine];
        let plain = Item::new(kind);
        let lens = Lens::new(&game, kind, vec![goal], &ops, &[&plain, &donor.items()[0]]);
        let starts: Vec<Arc<dyn Supply>> = vec![Arc::new(Stock::new("base", market.base, plain))];
        let craft = Craft::new(&ctx, &lens, &ops, &market, starts, 0);
        let exact = ponder_solve::solve_everywhere(&craft, &[(1.0, Node::Market)], 1000).unwrap();
        let lower = SideBound::new(&craft, 1000).unwrap();
        let mut hubs = 0;
        for (node, value) in exact.states.iter().zip(exact.values) {
            let bound = lower.value(node);
            assert!(bound <= value + 1e-8, "{node:?}: {bound} > {value}");
            hubs += usize::from(matches!(node, Node::Hub(..)) && bound > 0.0);
        }
        assert!(hubs > 0);
    }

    #[test]
    fn side_bounds_satisfy_the_full_operation_inequalities() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let game = Game::new(Repoe::load(&root.join("fixtures/repoe")).unwrap());
        let cases: &[(&str, Side, &[&str])] = &[
            (
                "cheap-ele-bow",
                Side::Prefix,
                &[
                    "essence:shrieking-anger",
                    "essence:shrieking-wrath",
                    "essence:deafening-hatred",
                    "bench:prefixes-cannot-be-changed",
                    "veiled-chaos",
                    "bench:remove-crafted",
                    "bench:speed-or-critical",
                    "unveil:speed-or-critical",
                    "harvest:reforge-fire",
                    "harvest:reforge-cold",
                    "harvest:reforge-lightning",
                ],
            ),
            (
                "ama-327-fire-dot-spell-crit-sceptre",
                Side::Suffix,
                &[
                    "essence:shrieking-scorn",
                    "bench:suffixes-cannot-be-changed",
                    "veiled-chaos",
                    "bench:physical-damage",
                    "unveil:fire-damage",
                    "bench:increased-damage-over-time",
                    "bench:remove-crafted",
                ],
            ),
            (
                "cheap-high-es-regalia",
                Side::Prefix,
                &[
                    "essence:shrieking-woe",
                    "essence:shrieking-spite",
                    "bench:prefixes-cannot-be-changed",
                    "veiled-chaos",
                    "bench:remove-crafted",
                ],
            ),
        ];
        for &(file, side, names) in cases {
            let guide: Guide =
                toml::from_str(&std::fs::read_to_string(root.join(format!("fixtures/guides/{file}.toml"))).unwrap()).unwrap();
            let target = Target::compile(&game, &guide.target("goal").unwrap()).unwrap();
            let kind =
                Kind { base: game.base(&guide.base_name()).unwrap(), ilvl: guide.ilvl.unwrap_or(86), influences: Influences::default() };
            let class = &game.bases[kind.base.0 as usize].class;
            let assumptions = Assumptions::default();
            let ctx = Ctx { game: &game, assumptions: &assumptions };
            let market = Market::load(&root.join("fixtures/prices.json.gz")).unwrap();
            let whole = target.witness(&game, kind).unwrap();
            let mut donor = whole.clone();
            donor.affixes.retain(|a| game.def(a.id).side == side);
            donor.affixes.remove(0);
            let donor = Arc::new(Stock::new("donor", 150.0, donor));
            let mut ops: Vec<Box<dyn Operation>> = Currency::ALL.into_iter().map(|c| Box::new(c) as Box<dyn Operation>).collect();
            for &name in names {
                ops.push(parse(&game, class, name, &[]).unwrap());
            }
            ops.push(Box::new(Recombinator::new(class, "test", donor.clone())));
            let refs: Vec<_> = ops.iter().map(|op| op.as_ref()).collect();
            let plain = Item::new(kind);
            let lens = Lens::new(&game, kind, vec![target], &refs, &[&plain, &whole, &donor.items()[0]]);
            let starts: Vec<Arc<dyn Supply>> = vec![Arc::new(Stock::new("base", market.base, plain))];
            let craft = Craft::new(&ctx, &lens, &refs, &market, starts, 0);
            let bound = SideBound::new(&craft, 200_000).unwrap();
            assert!(bound.market > market.base);
            let mut rng = Rng::new(719);
            for trial in 0..192 {
                let mut item = whole.clone();
                item.affixes.retain(|a| game.def(a.id).side == side);
                item.affixes = item.affixes.into_iter().enumerate().filter(|(i, _)| trial & (1 << i) != 0).map(|(_, a)| a).collect();
                item.rarity = if item.affixes.len() <= 1 && trial % 3 == 0 { Rarity::Magic } else { Rarity::Rare };
                for _ in 0..trial % 7 {
                    let add = if item.rarity == Rarity::Magic { Currency::Augmentation } else { Currency::Exalted };
                    if add.applies(&ctx, &(&game, &item)) {
                        run(&ctx, add.steps(), &mut item, &mut rng);
                    }
                }
                if !item.affixes.is_empty() && trial % 2 == 0 {
                    let index = rng.below(item.affixes.len());
                    item.affixes[index].fractured = true;
                }
                let other = Side::BOTH[1 - side as usize];
                let limit = if item.rarity == Rarity::Rare { 3 } else { 1 };
                if trial % 5 == 0 && item.count(&game, other) < limit {
                    item.affixes.push(Affix { id: game.metamods[side as usize], crafted: true, fractured: trial % 10 == 0 });
                }
                for (divisor, veil_side) in [(7, other), (11, side)] {
                    if trial % divisor == 0 && item.count(&game, veil_side) < limit {
                        item.affixes.push(Affix { id: game.veils[veil_side as usize], crafted: false, fractured: false });
                    }
                }
                let state = lens.project(&item);
                let lower = bound.value(&Node::Item(state));
                assert!(lower <= bound.market + 1e-8);
                for (i, (op, price)) in craft.ops.iter().enumerate() {
                    if !op.applies(&ctx, &(&game, &item)) {
                        continue;
                    }
                    let expected = price + craft.outcomes_of(&state, i).iter().map(|(p, t)| p * bound.value(&Node::Item(*t))).sum::<f64>();
                    assert!(
                        lower <= expected + 1e-8 * lower.max(1.0),
                        "{file}, trial {trial}, {}: {lower} > {expected}, {item:?}",
                        op.name()
                    );
                }
            }
        }
    }
}
