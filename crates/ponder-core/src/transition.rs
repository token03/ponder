use std::ops::Deref;
use std::sync::Arc;

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
