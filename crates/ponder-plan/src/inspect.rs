use std::fmt::Write;

use ponder_rules::game::Side;
use ponder_rules::ops::View;
use ponder_rules::rng::Rng;
use ponder_solve::inspect::{Inspection, Options, inspect};
use ponder_solve::{Model, Solution};

use crate::craft::{Craft, Node};

pub struct Report {
    pub markdown: String,
    pub states_tsv: String,
}

fn escape(value: &str) -> String {
    value.replace('|', "\\|").replace(['\t', '\n', '\r'], " ")
}

fn action_name(craft: &Craft, node: &Node, action: usize) -> String {
    let name = craft.action_name(action);
    if matches!(node, Node::Hub(..)) { format!("continue:{name}") } else { name }
}

fn description(craft: &Craft, node: &Node) -> String {
    match node {
        Node::Item(s) | Node::Hub(_, s) => {
            let view = craft.lens.view(s);
            let n = craft.lens.classes();
            let classes: Vec<_> = (0..n)
                .filter(|&c| s.counts[c] + s.counts[c + n] > 0)
                .map(|c| format!("{c}:{}+{}f", s.counts[c], s.counts[c + n]))
                .collect();
            format!(
                "{} {:?}, {}p/{}s, {} fractured, locks {:?}, classes [{}]",
                if matches!(node, Node::Hub(..)) { "continuation" } else { "item" },
                s.rarity,
                view.count(Side::Prefix),
                view.count(Side::Suffix),
                view.fractured(),
                view.locked(),
                classes.join(", ")
            )
        }
        other => format!("{other:?}"),
    }
}

pub fn report(craft: &Craft, solution: &Solution<Node>, options: Options, top: usize, state: Option<usize>) -> Result<Report, String> {
    let mut rng = Rng::new(1_000_000);
    let analysis = inspect(craft, solution, &[(1.0, Node::Market)], options, || rng.unit())?;
    render(craft, solution, &analysis, top, state)
}

fn render(craft: &Craft, solution: &Solution<Node>, analysis: &Inspection, top: usize, selected: Option<usize>) -> Result<Report, String> {
    let mut out = String::new();
    writeln!(out, "# Policy inspection\n").unwrap();
    writeln!(
        out,
        "Exact expected cost: {:.12}c. {} explored states; {} retained policy states; {} selected transitions.\n",
        solution.cost,
        solution.explored,
        analysis.states.len(),
        analysis.states.iter().map(|s| s.outcomes.len()).sum::<usize>()
    )
    .unwrap();
    writeln!(
        out,
        "{} selected-policy retry components, largest {} states. Restart edges are excluded from component sizes.\n",
        analysis.loops.len(),
        analysis.loops.first().map_or(0, Vec::len)
    )
    .unwrap();
    writeln!(out, "Seeded abstract-policy simulation: {}/{} finished; completed-run mean {:.6}c, standard deviation {:.6}c, mean {:.3} transitions. Visits include unfinished runs. Unseen in these samples does not mean unreachable.\n", analysis.finished, analysis.runs, analysis.mean_cost, analysis.cost_stddev, analysis.mean_steps).unwrap();
    writeln!(
        out,
        "Maximum selected-action Bellman residual: {:.3e}.\n",
        analysis.states.iter().map(|s| s.residual.abs()).fold(0.0, f64::max)
    )
    .unwrap();
    let mut actions = std::collections::BTreeMap::<String, (usize, usize, f64)>::new();
    for (i, s) in analysis.states.iter().enumerate() {
        if let Some(a) = s.action {
            let entry = actions.entry(action_name(craft, &solution.states[i], a)).or_default();
            entry.0 += 1;
            entry.1 += s.visits;
            entry.2 += s.visits as f64 * s.cost;
        }
    }
    let mut actions: Vec<_> = actions.into_iter().collect();
    actions.sort_by_key(|(_, a)| std::cmp::Reverse(a.1));
    writeln!(out, "## Selected actions\n\n| Action | Policy states | Uses/run | Spend/run (c) |\n|---|---:|---:|---:|").unwrap();
    for (a, (states, visits, spend)) in actions {
        writeln!(out, "| {} | {states} | {:.6} | {:.6} |", escape(&a), visits as f64 / analysis.runs as f64, spend / analysis.runs as f64)
            .unwrap();
    }
    writeln!(out, "\n## Largest retry components\n\n| States | Uses/run | Actions |\n|---:|---:|---|").unwrap();
    for component in analysis.loops.iter().take(top) {
        let mut names = std::collections::BTreeSet::new();
        let mut visits = 0;
        for &s in component {
            visits += analysis.states[s].visits;
            if let Some(a) = analysis.states[s].action {
                names.insert(craft.action_name(a));
            }
        }
        writeln!(
            out,
            "| {} | {:.6} | {} |",
            component.len(),
            visits as f64 / analysis.runs as f64,
            escape(&names.into_iter().collect::<Vec<_>>().join(", "))
        )
        .unwrap();
    }
    let mut ids: Vec<_> = (0..analysis.states.len()).collect();
    ids.sort_by_key(|&s| std::cmp::Reverse(analysis.states[s].visits));
    writeln!(
        out,
        "\n## Most visited states\n\n| ID | Reached/run | Visits/run | Value (c) | Action | State |\n|---:|---:|---:|---:|---|---|"
    )
    .unwrap();
    for &i in ids.iter().take(top) {
        let s = &analysis.states[i];
        let action = s.action.map_or_else(|| "terminal".into(), |a| action_name(craft, &solution.states[i], a));
        writeln!(
            out,
            "| {i} | {:.6} | {:.6} | {:.6} | {} | {} |",
            s.runs_reaching as f64 / analysis.runs as f64,
            s.visits as f64 / analysis.runs as f64,
            solution.values[i],
            escape(&action),
            escape(&description(craft, &solution.states[i]))
        )
        .unwrap();
    }
    if let Some(i) = selected {
        let s = analysis.states.get(i).ok_or_else(|| format!("state {i} is out of range ({} states)", analysis.states.len()))?;
        writeln!(
            out,
            "\n## State {i}\n\n{}\n\nSelected outcomes:\n\n| Probability | Next ID | Value (c) |\n|---:|---:|---:|",
            description(craft, &solution.states[i])
        )
        .unwrap();
        let mut outcomes = s.outcomes.to_vec();
        outcomes.sort_by(|a, b| b.0.total_cmp(&a.0));
        for (p, next) in outcomes {
            writeln!(out, "| {p:.12} | {next} | {:.12} |", solution.values[next]).unwrap();
        }
        writeln!(out, "\nAvailable actions, using retained successor values. Missing successors leave Q unevaluated.\n\n| Action | Immediate cost (c) | Unknown probability | Q (c) |\n|---|---:|---:|---:|").unwrap();
        for (action, cost) in craft.actions(&solution.states[i]) {
            let mut unknown = 0.0;
            let mut q = cost;
            craft.for_each_outcome(&solution.states[i], action, &mut |p, t| {
                if p <= 0.0 {
                    return;
                }
                if craft.is_goal(&t) {
                    return;
                }
                if craft.is_restart(&t) {
                    q += p * solution.cost;
                } else if let Some(&t) = solution.index.get(&t) {
                    q += p * solution.values[t];
                } else {
                    unknown += p;
                }
            });
            let value = if unknown > 0.0 { "unevaluated".into() } else { format!("{q:.12}") };
            writeln!(out, "| {} | {cost:.12} | {unknown:.12} | {value} |", escape(&action_name(craft, &solution.states[i], action)))
                .unwrap();
        }
    }
    writeln!(out, "\n## Class legend\n").unwrap();
    for c in 0..craft.lens.classes() {
        writeln!(out, "- {c}: {}", escape(&craft.lens.describe(c))).unwrap();
    }
    let mut states_tsv = "state\treachable\taction\tvalue_c\tvisits_per_run\truns_reaching\tresidual\tdescription\n".to_string();
    for (i, s) in analysis.states.iter().enumerate() {
        let action = s.action.map_or_else(|| "terminal".into(), |a| action_name(craft, &solution.states[i], a));
        writeln!(
            states_tsv,
            "{i}\t{}\t{}\t{:.17}\t{:.9}\t{}\t{:.9e}\t{}",
            s.reachable,
            escape(&action),
            solution.values[i],
            s.visits as f64 / analysis.runs as f64,
            s.runs_reaching,
            s.residual,
            escape(&description(craft, &solution.states[i]))
        )
        .unwrap();
    }
    Ok(Report { markdown: out, states_tsv })
}
