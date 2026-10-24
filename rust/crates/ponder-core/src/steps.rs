//! Reading a plan: state descriptions and the policy as numbered steps.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::problem::Problem;
use crate::solve::Plan;
use crate::types::*;

const RARITY: [&str; 3] = ["normal", "magic", "rare"];

/// E.g. `rare: Prismatic Heart + Life (blocked) + 1 other suffix`.
///
/// "blocked" means a mod sharing the target's group (a worse tier, or an
/// exclusive notable) is on the item, so the target can't roll until it goes.
pub fn describe(problem: &Problem, s: &State) -> String {
    let (rarity, parts) = describe_parts(problem, s);
    let text: Vec<String> = parts.into_iter().map(|p| p.0).collect();
    format!("{rarity}: {}", if text.is_empty() { "nothing useful".to_string() } else { text.join(" + ") })
}

/// `describe` in pieces for colouring: (rarity, [(text, "good" | "blocked" | "other" | "filler")]).
pub fn describe_parts(problem: &Problem, s: &State) -> (&'static str, Vec<(String, &'static str)>) {
    let mut parts = Vec::new();
    for (i, t) in problem.targets.iter().enumerate() {
        match s.status[i] {
            GOOD => parts.push((t.name.clone(), "good")),
            BLOCKED => parts.push((format!("{} (blocked)", t.name), "blocked")),
            _ => {}
        }
    }
    for (slot, label) in JUNK_LABELS.iter().enumerate() {
        if s.junk[slot] != 0 {
            parts.push((format!("{} {label}", s.junk[slot]), if slot >= 4 { "filler" } else { "other" }));
        }
    }
    (RARITY[s.rarity as usize], parts)
}

/// Like `recipe`, but with the states themselves.
pub fn recipe_states(plan: &Plan, min_share: f64) -> Vec<(State, String, f64)> {
    let total = plan.visits.values().sum::<f64>();
    let total = if total != 0.0 { total } else { 1.0 };
    let mut rows: Vec<_> = ordered_visits(plan)
        .into_iter()
        .filter(|(_, n)| n / total >= min_share)
        .map(|(s, n)| (s, plan.policy[&s].clone(), n / total))
        .collect();
    rows.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    rows
}

/// The policy for the states you actually meet: (state, action, share of all actions).
pub fn recipe(problem: &Problem, plan: &Plan, min_share: f64) -> Vec<(String, String, f64)> {
    recipe_states(plan, min_share).into_iter().map(|(s, a, share)| (describe(problem, &s), a, share)).collect()
}

fn ordered_visits(plan: &Plan) -> Vec<(State, f64)> {
    if plan.order.is_empty() {
        return plan.visits.iter().map(|(s, v)| (*s, *v)).collect();
    }
    plan.order.iter().filter_map(|s| plan.visits.get(s).map(|v| (*s, *v))).collect()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exit {
    pub chance: f64,       // per time you reach the step
    pub to: Option<usize>, // step number; None when the item is done or the branch is too rare to list
    pub what: String,      // e.g. "+Life", "lost Life", "done"
    /// "done", "win", "back" (lost a target), "side" (same targets, something else changed),
    /// "sell" (sold, start over), "rare" (unlisted)
    pub kind: String,
    #[serde(default)]
    pub via: String, // the action that mostly leads here
}

/// One stretch of a recipe: every situation with the same rarity and the same targets on the item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Step {
    pub number: usize,
    pub rarity: u8,
    pub have: Vec<String>,                   // target names on the item
    pub actions: Vec<(String, String, f64)>, // (action, what else is on the item when you use it, uses per reach), most used first
    pub reached: f64,                        // expected times you get here per finished item
    pub uses: f64,                           // expected actions here per finished item
    pub exits: Vec<Exit>,
}

impl Step {
    pub fn per_reach(&self) -> f64 {
        if self.reached != 0.0 { self.uses / self.reached } else { 0.0 }
    }
}

/// Rarity plus which targets are on at a good tier (a bit set).
type Key = (u8, u8);

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Dest {
    Done,
    At(Key),
}

/// Add to a small insertion-ordered map (Python dicts: ties keep first-seen order).
fn bump<K: PartialEq>(v: &mut Vec<(K, f64)>, k: K, x: f64) {
    match v.iter_mut().find(|e| e.0 == k) {
        Some(e) => e.1 += x,
        None => v.push((k, x)),
    }
}

fn good_key(problem: &Problem, st: &State) -> Key {
    let mask = (0..problem.targets.len()).filter(|&i| st.status[i] == GOOD).fold(0u8, |m, i| m | 1 << i);
    (st.rarity, mask)
}

/// `{n:,.0f}`
fn grouped(x: f64) -> String {
    let s = format!("{x:.0}");
    let (sign, digits) = s.strip_prefix('-').map_or(("", s.as_str()), |d| ("-", d));
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    format!("{sign}{out}")
}

/// The policy as numbered steps ("spam X until Y: a. ... → 3, b. ... → back to 1").
///
/// States are grouped by progress: rarity plus which targets are on at a good
/// tier. Following the policy's flow between groups gives, per step, how
/// often you reach it, how many actions it takes and where each outcome
/// leads. Steps reached less than `min_reach` times per item are folded
/// into their parent's "rare" exit.
pub fn steps(problem: &Problem, plan: &Plan, min_reach: f64) -> Vec<Step> {
    let by_name: HashMap<&str, &Action> = problem.actions.iter().map(|a| (a.name.as_str(), a)).collect();
    let open_visits: Vec<(State, f64)> =
        ordered_visits(plan).into_iter().filter(|(s, v)| *v > 1e-9 && plan.policy.contains_key(s)).collect();
    let ov: HashMap<State, f64> = open_visits.iter().copied().collect();
    let mut gi: HashMap<Key, usize> = HashMap::new();
    let mut keys: Vec<Key> = Vec::new();
    let mut groups: Vec<Vec<State>> = Vec::new();
    for &(st, _) in &open_visits {
        let k = good_key(problem, &st);
        let g = *gi.entry(k).or_insert_with(|| {
            keys.push(k);
            groups.push(Vec::new());
            keys.len() - 1
        });
        groups[g].push(st);
    }
    let mut reached = vec![0.0; keys.len()];
    let origin = problem.origin();
    let first = good_key(problem, &origin);
    if let Some(&g) = gi.get(&first) {
        reached[g] += 1.0;
    }
    let mut flow: Vec<Vec<(Dest, f64)>> = vec![Vec::new(); keys.len()]; // group -> next group (or done) -> expected moves
    let mut via: HashMap<(usize, Dest), Vec<(&str, f64)>> = HashMap::new(); // (group, next) -> action -> expected moves
    for &(st, v) in &open_visits {
        let g = gi[&good_key(problem, &st)];
        let a = plan.policy[&st].as_str();
        for &(p, nxt) in problem.outcomes(&st, by_name[a]).iter() {
            let to = if problem.done(&nxt) { Dest::Done } else { Dest::At(good_key(problem, &nxt)) };
            if to == Dest::At(keys[g]) {
                continue;
            }
            bump(&mut flow[g], to, v * p);
            bump(via.entry((g, to)).or_default(), a, v * p);
            if let Dest::At(tk) = to
                && let Some(&t) = gi.get(&tk)
            {
                reached[t] += v * p;
            }
        }
    }
    let mut kept: Vec<usize> = (0..keys.len()).filter(|&g| reached[g] >= min_reach || keys[g] == first).collect();
    kept.sort_by(|&a, &b| {
        let key = |g: usize| (keys[g] != first, keys[g].1.count_ones(), keys[g].0);
        key(a).cmp(&key(b)).then(reached[b].partial_cmp(&reached[a]).unwrap())
    });
    let number: HashMap<Key, usize> = kept.iter().enumerate().map(|(i, &g)| (keys[g], i + 1)).collect();
    let names: Vec<&str> = problem.targets.iter().map(|t| t.name.as_str()).collect();
    let mut out = Vec::new();
    for &g in &kept {
        let (rarity, have) = keys[g];
        let mut acts: Vec<(&str, f64, State, f64)> = Vec::new(); // action, visits, busiest state, its visits
        for &st in &groups[g] {
            let a = plan.policy[&st].as_str();
            let i = acts.iter().position(|x| x.0 == a).unwrap_or_else(|| {
                acts.push((a, 0.0, st, 0.0));
                acts.len() - 1
            });
            acts[i].1 += ov[&st];
            if ov[&st] > acts[i].3 {
                acts[i].2 = st;
                acts[i].3 = ov[&st];
            }
        }
        let uses = acts.iter().map(|x| x.1).sum::<f64>();
        acts.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        let actions = acts.iter().map(|&(a, v, st, _)| (a.to_string(), else_of(problem, &st), v / reached[g])).collect();
        let mut exits = Vec::new();
        let mut rare = 0.0;
        let mut dests = flow[g].clone();
        dests.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        for (to, n) in dests {
            let chance = n / reached[g];
            let mut by = ("", f64::NEG_INFINITY);
            for &(a, v) in &via[&(g, to)] {
                if v > by.1 {
                    by = (a, v);
                }
            }
            let by = by.0;
            match to {
                Dest::Done => {
                    let rest: Vec<&str> = (0..names.len()).filter(|i| have >> i & 1 == 0).map(|i| names[i]).collect();
                    let mut got = Vec::new();
                    if !rest.is_empty() && problem.need == names.len() {
                        got.push(rest.join(", "));
                    }
                    if problem.filler_need > 0 {
                        got.push("filler".to_string());
                    }
                    let what = if got.is_empty() { "done".to_string() } else { format!("got {}", got.join(" + ")) };
                    exits.push(Exit { chance, to: None, what, kind: "done".into(), via: by.into() });
                }
                Dest::At(t) if by_name[by].kind == "sell" && number.contains_key(&t) => {
                    let what = format!("{by} (+{}c)", grouped(by_name[by].credit));
                    exits.push(Exit { chance, to: Some(number[&t]), what, kind: "sell".into(), via: by.into() });
                }
                Dest::At(t) if number.contains_key(&t) => {
                    let (gained, lost) = (t.1 & !have, have & !t.1);
                    let mut what: Vec<String> =
                        (0..names.len()).filter(|i| gained >> i & 1 == 1).map(|i| format!("got {}", names[i])).collect();
                    what.extend((0..names.len()).filter(|i| lost >> i & 1 == 1).map(|i| format!("lost {}", names[i])));
                    let what = if what.is_empty() { format!("now {}", RARITY[t.0 as usize]) } else { what.join(", ") };
                    let kind = if lost != 0 {
                        "back"
                    } else if gained != 0 {
                        "win"
                    } else {
                        "side"
                    };
                    exits.push(Exit { chance, to: Some(number[&t]), what, kind: kind.into(), via: by.into() });
                }
                Dest::At(_) => rare += chance,
            }
        }
        if rare > 1e-6 {
            exits.push(Exit { chance: rare, to: None, what: "something rarer".into(), kind: "rare".into(), via: String::new() });
        }
        let have_names = (0..names.len()).filter(|i| have >> i & 1 == 1).map(|i| names[i].to_string()).collect();
        out.push(Step { number: number[&keys[g]], rarity, have: have_names, actions, reached: reached[g], uses, exits });
    }
    out
}

/// What's on the item besides good targets, e.g. "2 other prefix + 1 other suffix".
fn else_of(problem: &Problem, s: &State) -> String {
    let (_, parts) = describe_parts(problem, s);
    let text: Vec<String> = parts.into_iter().filter(|p| p.1 != "good").map(|p| p.0).collect();
    if text.is_empty() { "nothing else".to_string() } else { text.join(" + ") }
}
