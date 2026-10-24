//! The solver against an exact model of small items, where every mod on the item is known.
//!
//! The solver plans on lumped states (how many other mods per slot, not which), so the chances
//! it sees are point estimates within bounds. Here random small pools are solved exactly, item by
//! item, and the solver's certificate is checked: the cheapest way to craft (one that reads every
//! mod) costs at least `bounds.lower`, and the solver's plan played on real items costs at most
//! `bounds.upper` and has an action for every item it meets.

use std::collections::HashMap;

use ponder_core::*;

const LIMIT: usize = 3; // affixes per side on gear

/// A real item: rarity and which pool mods it has (bit i: pool[i]).
type Item = (u8, u32);

struct Exact<'a> {
    pool: &'a [Mod],
}

impl Exact<'_> {
    fn on_side(&self, mods: u32, side: Side) -> usize {
        (0..self.pool.len()).filter(|&i| mods & 1 << i != 0 && self.pool[i].side == side).count()
    }

    /// One mod added at `rarity`: each eligible mod in proportion to its weight.
    fn draw(&self, rarity: u8, mods: u32) -> Vec<(f64, u32)> {
        let limit = if rarity == MAGIC { 1 } else { LIMIT };
        let taken: Vec<&String> = (0..self.pool.len()).filter(|&i| mods & 1 << i != 0).flat_map(|i| &self.pool[i].groups).collect();
        let eligible: Vec<usize> = (0..self.pool.len())
            .filter(|&i| {
                let m = &self.pool[i];
                mods & 1 << i == 0 && m.weight > 0.0 && self.on_side(mods, m.side) < limit && !m.groups.iter().any(|g| taken.contains(&g))
            })
            .collect();
        let total: f64 = eligible.iter().map(|&i| self.pool[i].weight).sum();
        eligible.iter().map(|&i| (self.pool[i].weight / total, mods | 1 << i)).collect()
    }

    /// A fresh roll: the mod count first, then draws until it is reached or nothing can roll.
    fn roll(&self, rarity: u8) -> Vec<(f64, u32)> {
        let counts: Vec<(usize, f64)> =
            if rarity == MAGIC { vec![(1, 1.0 - MAGIC_TWO_MOD_CHANCE), (2, MAGIC_TWO_MOD_CHANCE)] } else { RARE_MOD_COUNTS_GEAR.to_vec() };
        let mut out: HashMap<u32, f64> = HashMap::new();
        for (n, pn) in counts {
            let mut dist = vec![(pn, 0u32)];
            for _ in 0..n {
                let mut next = Vec::new();
                for (p, m) in dist {
                    let d = self.draw(rarity, m);
                    if d.is_empty() {
                        next.push((p, m));
                    }
                    next.extend(d.into_iter().map(|(q, x)| (p * q, x)));
                }
                dist = next;
            }
            for (p, m) in dist {
                *out.entry(m).or_default() += p;
            }
        }
        out.into_iter().map(|(m, p)| (p, m)).collect()
    }

    /// What `action` does to a real item, the way the simulator plays it; empty if it can't be used.
    fn apply(&self, a: &Action, (rarity, mods): Item) -> Vec<(f64, Item)> {
        if !a.from_rarity.contains(&rarity) {
            return Vec::new();
        }
        let total = mods.count_ones() as usize;
        let at = |r: u8, v: Vec<(f64, u32)>| v.into_iter().map(|(p, m)| (p, (r, m))).collect();
        match a.kind {
            ActionKind::Transmute | ActionKind::Alt => at(MAGIC, self.roll(MAGIC)),
            ActionKind::RerollRare | ActionKind::Reforge => at(RARE, self.roll(RARE)),
            ActionKind::Aug if total < 2 => at(MAGIC, self.draw(MAGIC, mods)),
            ActionKind::Regal => at(RARE, self.draw(RARE, mods)),
            ActionKind::Exalt if total < 2 * LIMIT => at(RARE, self.draw(RARE, mods)),
            ActionKind::Annul if total > 0 => {
                (0..self.pool.len()).filter(|&i| mods & 1 << i != 0).map(|i| (1.0 / total as f64, (rarity, mods & !(1 << i)))).collect()
            }
            _ => Vec::new(),
        }
    }
}

/// Expected cost from each real item, by Gauss-Seidel value iteration: the cheapest action
/// everywhere (`policy` None), or the one `policy` names for the item's lumped state. Panics if
/// the policy has no usable action for an item it reaches.
fn exact_values(problem: &Problem, prices: &Prices, settings: &Settings, policy: Option<&StateMap<String>>) -> f64 {
    let ex = Exact { pool: &problem.pool };
    let state = |(r, m): Item| problem.state_of(r, (0..problem.pool.len()).filter(|&i| m & 1 << i != 0).map(|i| &problem.pool[i]));
    let costs: Vec<f64> = problem.actions.iter().map(|a| step_cost(a, prices, settings)).collect();
    // Every item reachable from a fresh base, with each usable action's outcomes.
    let origin: Item = (NORMAL, 0);
    let mut index: HashMap<Item, usize> = HashMap::from([(origin, 0)]);
    let mut items = vec![origin];
    // per item: (cost, outcomes as (chance, item number)) for each usable action
    type Moves = Vec<(f64, Vec<(f64, usize)>)>;
    let mut moves: Vec<Moves> = Vec::new();
    let mut k = 0;
    while k < items.len() {
        let item = items[k];
        let mut mine = Vec::new();
        if !problem.done(&state(item)) {
            let allowed: Vec<usize> = match policy {
                None => (0..problem.actions.len()).collect(),
                Some(pol) => {
                    let name =
                        pol.get(&state(item)).unwrap_or_else(|| panic!("the plan has no action for {:?} (item {item:?})", state(item)));
                    vec![problem.actions.iter().position(|a| &a.name == name).unwrap()]
                }
            };
            for ai in allowed {
                let out = ex.apply(&problem.actions[ai], item);
                if out.is_empty() {
                    assert!(policy.is_none(), "the plan's {:?} can't be used on item {item:?}", problem.actions[ai].name);
                    continue;
                }
                let mut next = Vec::new();
                for (p, n) in out {
                    let j = *index.entry(n).or_insert_with(|| {
                        items.push(n);
                        items.len() - 1
                    });
                    next.push((p, j));
                }
                mine.push((costs[ai], next));
            }
        }
        moves.push(mine);
        k += 1;
    }
    let mut v = vec![0.0; items.len()];
    for sweep in 0.. {
        let mut change: f64 = 0.0;
        for i in 0..items.len() {
            if moves[i].is_empty() {
                continue;
            }
            let best = moves[i].iter().map(|(c, next)| c + next.iter().map(|&(p, j)| p * v[j]).sum::<f64>()).fold(f64::INFINITY, f64::min);
            change = change.max((best - v[i]).abs() / best.max(1e-9));
            v[i] = best;
        }
        if change < 1e-12 {
            break;
        }
        assert!(sweep < 2_000_000, "value iteration did not settle");
    }
    v[0]
}

fn act(name: &str, kind: ActionKind, price: &str, from: &[u8]) -> Action {
    Action::new(name, kind, &[(price, 1.0)], from)
}

fn actions() -> Vec<Action> {
    vec![
        act("transmute", ActionKind::Transmute, "transmute", &[NORMAL]),
        act("alt", ActionKind::Alt, "alt", &[MAGIC]),
        act("aug", ActionKind::Aug, "aug", &[MAGIC]),
        act("regal", ActionKind::Regal, "regal", &[MAGIC]),
        act("alch", ActionKind::RerollRare, "alch", &[NORMAL]),
        act("chaos", ActionKind::Reforge, "chaos", &[RARE]),
        act("exalt", ActionKind::Exalt, "exalt", &[RARE]),
        act("annul", ActionKind::Annul, "annul", &[MAGIC, RARE]),
    ]
}

/// Prices; `slams` makes exalts and annulments cheap, so plans lean on them (and on the bounds).
fn prices(slams: bool) -> Prices {
    let (exalt, annul) = if slams { (0.3, 0.2) } else { (6.0, 4.0) };
    [("transmute", 0.1), ("alt", 0.2), ("aug", 0.1), ("regal", 1.0), ("alch", 0.5), ("chaos", 1.0), ("exalt", exalt), ("annul", annul)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
}

fn target(m: &Mod) -> Target {
    Target { name: m.id.clone(), side: m.side, good: [m.id.clone()].into(), groups: m.groups.iter().cloned().collect(), notable: false }
}

/// A random pool of 3-4 prefixes and 3-4 suffixes with widely spread weights, one or two targets.
fn random_problem(seed: u64) -> Problem {
    let mut rng = Rng::new(seed);
    let weights = [1.0, 5.0, 20.0, 100.0, 400.0];
    let mut pool = Vec::new();
    for (side, name) in [(Side::Prefix, "p"), (Side::Suffix, "s")] {
        for i in 0..3 + (rng.random() * 2.0) as usize {
            let id = format!("{name}{i}");
            let w = weights[(rng.random() * weights.len() as f64) as usize];
            pool.push(Mod::new(&id, side, w, &[&id], false, &id, &[]));
        }
    }
    let first_suffix = pool.iter().position(|m| m.side == Side::Suffix).unwrap();
    let mut targets = vec![target(&pool[0])];
    if rng.random() < 0.5 {
        targets.push(target(&pool[first_suffix]));
    }
    let need = targets.len();
    ProblemSpec::new(pool, targets, need, actions(), Kind::Gear).compile()
}

#[test]
fn certified_bounds_hold_on_real_items() {
    let settings = Settings { chaos_per_hour: 0.0 };
    let mut loose = 0;
    for (seed, slams) in (0..24).flat_map(|s| [(s, false), (s, true)]) {
        let prices = prices(slams);
        let p = random_problem(seed);
        let plan = solve(&p, &prices, &settings).unwrap_or_else(|e| panic!("seed {seed}: {e}"));
        let b = plan.bounds.expect("solve gives bounds");
        let best = exact_values(&p, &prices, &settings, None);
        let followed = exact_values(&p, &prices, &settings, Some(&plan.policy));
        let tol = 1e-7 * best;
        eprintln!(
            "seed {seed:>2}: lower {:>9.4} <= best {best:>9.4} <= plan on real items {followed:>9.4} <= upper {:>9.4} (plan says {:.4})",
            b.lower, b.upper, plan.cost
        );
        assert!(b.lower <= best + tol, "seed {seed}: lower bound {} above the exact optimum {best}", b.lower);
        assert!(best <= followed + tol, "seed {seed}: the plan beats the exact optimum");
        assert!(followed <= b.upper + tol, "seed {seed}: the plan costs {followed} on real items, above its upper bound {}", b.upper);
        assert!(b.lower <= plan.cost + tol && plan.cost <= b.upper + tol);
        loose += (b.upper - b.lower > tol) as usize;
    }
    assert!(loose >= 10, "only {loose} toys had loose bounds: the test isn't exercising them");
}

#[test]
fn fresh_rolls_are_exact() {
    // Junk of very different weights: lumping them into one average group got the roll wrong.
    let pool: Vec<Mod> = [("goal", 1.0), ("heavy", 100.0), ("light", 1.0), ("other", 1.0), ("s1", 1.0), ("s2", 1.0), ("s3", 1.0)]
        .into_iter()
        .enumerate()
        .map(|(i, (id, w))| Mod::new(id, if i < 4 { Side::Prefix } else { Side::Suffix }, w, &[id], false, id, &[]))
        .collect();
    let p = ProblemSpec::new(pool.clone(), vec![target(&pool[0])], 1, vec![], Kind::Gear).compile();
    let solver: f64 = p
        .roll(&Action::new("roll", ActionKind::RerollRare, &[], &[NORMAL, RARE]))
        .unwrap()
        .iter()
        .filter(|(_, s)| s.status[0] == GOOD)
        .map(|x| x.0)
        .sum();
    let ex = Exact { pool: &pool };
    let exact: f64 = ex.roll(RARE).iter().filter(|(_, m)| m & 1 != 0).map(|x| x.0).sum();
    assert!((solver - exact).abs() < 1e-12, "solver {solver} vs exact {exact}");
    assert!(p.roll(&Action::new("roll", ActionKind::RerollRare, &[], &[NORMAL, RARE])).unwrap().is_exact());
    // Every item a roll can make has a state the roll reaches, the six-mod miss included.
    let full_miss = State::new(RARE, &[], &[0, 3, 0, 3]);
    assert!(p.roll(&Action::new("roll", ActionKind::RerollRare, &[], &[NORMAL, RARE])).unwrap().iter().any(|(_, s)| *s == full_miss));
}

#[test]
fn an_avoidable_dead_end_is_avoided_not_fatal() {
    // Alchemy makes a rare, which transmutes and alterations can't fix: a dead end, but
    // alterations alone still get there.
    let pool: Vec<Mod> = ["A", "p1", "p2", "p3", "s1", "s2", "s3"]
        .into_iter()
        .enumerate()
        .map(|(i, id)| Mod::new(id, if i < 4 { Side::Prefix } else { Side::Suffix }, 1.0, &[id], false, id, &[]))
        .collect();
    let all = vec![
        act("transmute", ActionKind::Transmute, "transmute", &[NORMAL]),
        act("alt", ActionKind::Alt, "alt", &[MAGIC]),
        Action::new("alch", ActionKind::RerollRare, &[("alch", 1.0)], &[NORMAL]),
    ];
    let prices: Prices = [("transmute".into(), 1.0), ("alt".into(), 1.0), ("alch".into(), 1.0)].into();
    let settings = Settings { chaos_per_hour: 0.0 };
    let safe = ProblemSpec::new(pool.clone(), vec![target(&pool[0])], 1, all[..2].to_vec(), Kind::Gear).compile();
    let risky = ProblemSpec::new(pool.clone(), vec![target(&pool[0])], 1, all, Kind::Gear).compile();
    let a = solve(&safe, &prices, &settings).unwrap();
    let b = solve(&risky, &prices, &settings).unwrap();
    assert!((a.cost - b.cost).abs() < 1e-9 * a.cost, "{} vs {}", a.cost, b.cost);
    assert!(b.policy.values().all(|n| n != "alch"));
}

#[test]
fn overlapping_groups_do_not_receive_an_exact_certificate() {
    let pool = vec![
        Mod::new("goal", Side::Prefix, 1.0, &["goal"], false, "goal", &[]),
        Mod::new("x", Side::Prefix, 100.0, &["shared", "x"], false, "x", &[]),
        Mod::new("y", Side::Prefix, 100.0, &["shared", "y"], false, "y", &[]),
        Mod::new("z", Side::Prefix, 1.0, &["z"], false, "z", &[]),
        Mod::new("s1", Side::Suffix, 1.0, &["s1"], false, "s1", &[]),
        Mod::new("s2", Side::Suffix, 1.0, &["s2"], false, "s2", &[]),
        Mod::new("s3", Side::Suffix, 1.0, &["s3"], false, "s3", &[]),
    ];
    let action = Action::new("reroll", ActionKind::RerollRare, &[("orb", 1.0)], &[NORMAL, RARE]);
    let p = ProblemSpec::new(pool.clone(), vec![target(&pool[0])], 1, vec![action.clone()], Kind::Gear).compile();
    let roll = p.roll(&action).unwrap();
    assert!(!roll.is_exact());
    let prices = [("orb".into(), 1.0)].into();
    let settings = Settings { chaos_per_hour: 0.0 };
    let plan = solve(&p, &prices, &settings).unwrap();
    let exact = exact_values(&p, &prices, &settings, None);
    let bounds = plan.bounds.unwrap();
    assert!(bounds.lower <= exact + 1e-9 && exact <= bounds.upper);
    assert!((exact - 1.477272706).abs() < 1e-7);
}
