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
