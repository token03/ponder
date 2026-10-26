use std::ops::Deref;
use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::types::{Outcome, State};

pub type ProbabilityRange = (f64, f64);

#[derive(Clone, Debug, PartialEq)]
pub enum Distribution {
    Exact(Vec<Outcome>),
    Bounded { outcomes: Vec<Outcome>, bounds: Vec<ProbabilityRange> },
}

impl Distribution {
    pub(crate) fn new(outcomes: Vec<Outcome>, bounds: Option<Vec<ProbabilityRange>>) -> Distribution {
        if let Some(b) = &bounds {
            assert_eq!(outcomes.len(), b.len());
        }
        match bounds {
            Some(bounds) if outcomes.iter().zip(&bounds).any(|(&(p, _), &(lo, hi))| p != lo || p != hi) => {
                Distribution::Bounded { outcomes, bounds }
            }
            _ => Distribution::Exact(outcomes),
        }
    }

    pub fn is_exact(&self) -> bool {
        matches!(self, Distribution::Exact(_))
    }

    pub fn bounds(&self) -> Option<&[ProbabilityRange]> {
        match self {
            Distribution::Exact(_) => None,
            Distribution::Bounded { bounds, .. } => Some(bounds),
        }
    }
}

impl Deref for Distribution {
    type Target = [Outcome];

    fn deref(&self) -> &[Outcome] {
        match self {
            Distribution::Exact(outcomes) | Distribution::Bounded { outcomes, .. } => outcomes,
        }
    }
}

#[derive(Default)]
pub struct TransitionBuffer {
    outcomes: Vec<Outcome>,
    bounds: Vec<ProbabilityRange>,
}

impl TransitionBuffer {
    pub fn clear(&mut self) {
        self.outcomes.clear();
        self.bounds.clear();
    }

    pub fn push(&mut self, probability: f64, range: ProbabilityRange, next: State) {
        self.outcomes.push((probability, next));
        self.bounds.push(range);
    }

    pub fn outcomes(&self) -> &[Outcome] {
        &self.outcomes
    }

    pub fn bounds(&self) -> &[ProbabilityRange] {
        &self.bounds
    }

    pub(crate) fn parts_mut(&mut self) -> (&mut Vec<Outcome>, &mut Vec<ProbabilityRange>) {
        (&mut self.outcomes, &mut self.bounds)
    }
}

pub enum TransitionResult {
    Empty,
    Buffered,
    Shared(Arc<Distribution>),
}

/// An insertion-ordered distribution over states (Python dicts keep order; so do the sums),
/// with bounds on each probability.
pub(crate) struct Dist {
    pub(crate) items: Vec<(State, f64, f64, f64)>,
    index: FxHashMap<State, usize>,
}

impl Dist {
    pub(crate) fn new() -> Dist {
        Dist { items: Vec::new(), index: FxHashMap::default() }
    }

    pub(crate) fn one(s: State) -> Dist {
        let mut d = Dist::new();
        d.add(s, 1.0, 1.0, 1.0);
        d
    }

    pub(crate) fn add(&mut self, s: State, p: f64, lo: f64, hi: f64) {
        match self.index.get(&s) {
            Some(&i) => {
                let x = &mut self.items[i];
                x.1 += p;
                x.2 += lo;
                x.3 += hi;
            }
            None => {
                self.index.insert(s, self.items.len());
                self.items.push((s, p, lo, hi));
            }
        }
    }

    /// The outcomes with any chance of happening, and their bounds unless all are exact.
    pub(crate) fn finish(self) -> (Vec<Outcome>, Option<Vec<ProbabilityRange>>) {
        let kept: Vec<_> = self.items.into_iter().filter(|x| x.3 > 0.0).collect();
        let exact = kept.iter().all(|x| x.2 == x.1 && x.3 == x.1);
        let out = kept.iter().map(|x| (x.1, x.0)).collect();
        (out, (!exact).then(|| kept.iter().map(|x| (x.2.min(x.1), x.3.min(1.0).max(x.1))).collect()))
    }
}

/// Probabilities from weights known within bounds: `(next, weight, least, most)`.
///
/// Each probability is bounded by its own weight at its least (most) against the others at
/// their most (least). A choice whose weight is 0 but may not be stays in with probability 0.
pub(crate) fn normalise(cands: &[(State, f64, f64, f64)], out: &mut Vec<Outcome>, bounds: &mut Vec<ProbabilityRange>) {
    out.clear();
    bounds.clear();
    let (mut sum, lo_sum, hi_sum) = cands.iter().fold((0.0, 0.0, 0.0), |a, c| (a.0 + c.1, a.1 + c.2, a.2 + c.3));
    if hi_sum <= 0.0 {
        return;
    }
    let use_hi = sum <= 0.0; // the point estimate runs dry where the item may not: fall back on the most
    if use_hi {
        sum = hi_sum;
    }
    if cands.iter().all(|c| c.1 == c.2 && c.1 == c.3) {
        for &(n, w, ..) in cands {
            out.push((w / sum, n));
            bounds.push((w / sum, w / sum));
        }
        return;
    }
    for &(n, w, lo, hi) in cands {
        let p = if use_hi { hi } else { w } / sum;
        let low = if lo > 0.0 { lo / (lo + hi_sum - hi).max(lo) } else { 0.0 };
        let high = if hi > 0.0 { (hi / (hi + lo_sum - lo).max(hi)).min(1.0) } else { 0.0 };
        out.push((p, n));
        bounds.push((low.min(p), high.max(p)));
    }
}
