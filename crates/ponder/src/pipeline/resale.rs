use ponder_core::{Plan, Problem, RARE, Recovery, SaleRecovery, Side};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResaleQuote {
    pub proceeds: f64,
    pub replacement_base: f64,
    pub trade_seconds: f64,
}

impl Default for ResaleQuote {
    fn default() -> Self {
        Self { proceeds: 0.0, replacement_base: 0.0, trade_seconds: 30.0 }
    }
}

impl ResaleQuote {
    pub fn validate(self) -> Result<(), String> {
        if [self.proceeds, self.replacement_base, self.trade_seconds].into_iter().all(|v| v.is_finite() && v >= 0.0) {
            Ok(())
        } else {
            Err("sale proceeds, replacement base price and trade time must be finite and nonnegative".into())
        }
    }

    pub fn apply(
        self,
        recovery: &mut Recovery,
        replacement_cost: f64,
        replacement_hours: f64,
        mut usage: std::collections::HashMap<String, f64>,
        chaos_per_hour: f64,
    ) {
        *usage.entry("sell blocked item and replace base".into()).or_default() += 1.0;
        recovery.sale = Some(SaleRecovery {
            proceeds: self.proceeds,
            cost: replacement_cost + self.replacement_base + self.trade_seconds * chaos_per_hour / 3600.0,
            hours: replacement_hours + self.trade_seconds / 3600.0,
            usage: usage.into_iter().collect(),
        });
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SaleThreshold {
    pub state: ponder_core::State,
    pub description: String,
    pub premium: f64,
}

pub fn thresholds(
    problem: &Problem,
    plan: &Plan,
    recovery: &Recovery,
    core: ponder_core::Preservation,
    replacement_cost: f64,
) -> Vec<SaleThreshold> {
    let remaining: Option<f64> = recovery.initial.iter().map(|(p, s)| Some(p * plan.to_finish(problem, s)?)).sum();
    let Some(remaining) = remaining else { return Vec::new() };
    let mut out: Vec<_> = plan
        .values
        .iter()
        .chain(&plan.restart_values)
        .filter(|(s, _)| {
            s.rarity == RARE
                && s.meta == 0
                && core.kept(s)
                && [Side::Prefix, Side::Suffix].into_iter().all(|side| problem.count(s, side) == problem.side_limit(RARE, side))
        })
        .map(|(&state, &value)| SaleThreshold {
            state,
            description: ponder_core::describe(problem, &state),
            premium: replacement_cost + remaining - value,
        })
        .collect();
    out.sort_by(|a, b| a.premium.total_cmp(&b.premium).then_with(|| a.description.cmp(&b.description)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ponder_core::*;
    use std::sync::Arc;

    #[test]
    fn break_even_uses_replacement_cost_and_excludes_lost_cores() {
        let mut spec = ProblemSpec::new(
            vec![],
            vec![Target {
                name: "core".into(),
                side: Side::Prefix,
                good: ["core".into()].into(),
                groups: ["core".into()].into(),
                notable: false,
            }],
            1,
            vec![],
            Kind::Gear,
        );
        spec.min_mods = 7;
        let p = spec.compile();
        let full = State::new(RARE, &[GOOD], &[0, 2, 0, 3]);
        let lost = State::new(RARE, &[], &[0, 3, 0, 3]);
        let seed = State::new(RARE, &[GOOD], &[]);
        let mut plan = Plan::default();
        plan.values.extend([(full, 80.0), (lost, 300.0), (seed, 30.0)]);
        let recovery = Recovery {
            initial: Arc::new(Distribution::Exact(vec![(1.0, seed)])),
            cost: 100.0,
            hours: 0.0,
            usage: Default::default(),
            sale: None,
        };
        let result = thresholds(&p, &plan, &recovery, Preservation { targets: 1, filler: [0; 2] }, 110.0);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].state, full);
        assert_eq!(result[0].premium, 60.0);
    }
}
