use super::{Cost, Profiles, act, loaded, slug};
use ponder_core::{Action, ActionKind, FOSSIL_SECONDS, Mod, NORMAL, Prices, RARE, Settings, Side};
use ponder_game::data;
use rayon::prelude::*;
use std::collections::{BTreeSet, HashMap};

pub const RESONATOR: &str = "primitive-chaotic-resonator";
/// Chaotic resonators by socket count (index 0 is unused): a reroll with 1-4 fossils.
pub const RESONATORS: [&str; 5] = ["", RESONATOR, "potent-chaotic-resonator", "powerful-chaotic-resonator", "prime-chaotic-resonator"];
pub const FOSSIL_COMBOS_KEPT: usize = 10; // combo profiles handed to the solver

/// Single fossils (with a Primitive Resonator) that change this pool's weights.
pub fn fossils(pool: &[Mod]) -> (Vec<Action>, Profiles) {
    let (mut acts, mut profiles) = (Vec::new(), Profiles::new());
    for fossil in loaded(data::fossils()).values() {
        let Some(mult) = usable_fossil(fossil) else {
            continue;
        };
        let weights: Vec<f64> =
            pool.iter().map(|m| m.tags.iter().filter_map(|t| mult.get(t.as_str())).fold(m.weight, |w, x| w * x)).collect();
        if pool.iter().zip(&weights).all(|(m, w)| (w - m.weight).abs() < 1e-9) {
            continue; // changes nothing here
        }
        let name = fossil.name.replace(" Fossil", "").to_lowercase();
        let profile = format!("fossil:{name}");
        profiles.insert(profile.clone(), pool.iter().zip(weights).map(|(m, w)| (m.id.clone(), w)).collect());
        acts.push(
            act(&format!("{name} fossil"), ActionKind::RerollRare, &[(&slug(&fossil.name), 1.0), (RESONATOR, 1.0)], &[NORMAL, RARE])
                .with_profile(&profile),
        );
    }
    (acts, profiles)
}

/// The weight multiplier per tag of a plain fossil (no added or forced mods, no tricks), if it has any.
fn usable_fossil(f: &data::Fossil) -> Option<HashMap<&str, f64>> {
    if f.name.is_empty() || !f.added_mods.is_empty() || !f.forced_mods.is_empty() || f.mirrors || f.rolls_lucky || f.changes_quality {
        return None;
    }
    let mult: HashMap<&str, f64> =
        f.positive_mod_weights.iter().chain(&f.negative_mod_weights).map(|w| (w.tag.as_str(), w.weight / 100.0)).collect();
    (!mult.is_empty()).then_some(mult)
}

/// A weight-changing fossil: its name, price id, and per-mod weight multiplier.
#[derive(Clone, Debug)]
pub struct FossilRow {
    pub name: String,
    pub pid: String,
    pub mult: Vec<f64>,
}

/// (name, price id, per-mod weight multiplier) for every weight-changing fossil.
///
/// A mod's multiplier is the product over its tags of the fossil's modifiers
/// for them; fossils in one resonator multiply together.
pub fn fossil_rows(pool: &[Mod]) -> Vec<FossilRow> {
    let mut rows = Vec::new();
    for fossil in loaded(data::fossils()).values() {
        let Some(mult) = usable_fossil(fossil) else {
            continue;
        };
        let row: Vec<f64> = pool.iter().map(|m| m.tags.iter().filter_map(|t| mult.get(t.as_str())).fold(1.0, |p, x| p * x)).collect();
        if row.iter().all(|x| (x - 1.0).abs() <= 1e-8 + 1e-5) {
            continue; // np.allclose(row, 1.0): changes nothing here
        }
        rows.push(FossilRow { name: fossil.name.replace(" Fossil", "").to_lowercase(), pid: slug(&fossil.name), mult: row });
    }
    rows
}

#[derive(Clone, Copy, Debug)]
pub struct FossilOpts {
    /// Mods a reroll lands per side (half of what a rare rolls).
    pub mods_per_side: f64,
    pub max_fossils: usize,
    pub keep: usize,
}

impl Default for FossilOpts {
    fn default() -> FossilOpts {
        FossilOpts { mods_per_side: 2.3, max_fossils: 4, keep: FOSSIL_COMBOS_KEPT }
    }
}

/// The fossil combinations (1-4 in a chaotic resonator) worth giving the solver.
///
/// For each wanted mod (a set of acceptable ids), the chance a reroll hits it is about
/// `1 - (1 - its share of its side's weight) ** mods_per_side`, and a set of
/// wants is hit with the product. For every subset of the wants, the
/// combination with the lowest price per hit is kept (a reroll that lands
/// two prefixes is worth having even when exalts finish the item), largest
/// subsets first, up to `keep`. The solver then values them exactly.
pub fn fossil_combos(
    pool: &[Mod],
    wants: &[&BTreeSet<String>],
    prices: &Prices,
    settings: &Settings,
    opts: FossilOpts,
) -> (Vec<Action>, Profiles) {
    let rows: Vec<FossilRow> = fossil_rows(pool).into_iter().filter(|r| prices.contains_key(&r.pid)).collect();
    fossil_combos_with(&rows, pool, wants, prices, settings, opts)
}

/// [`fossil_combos`] over the given fossil rows (tests supply their own).
pub fn fossil_combos_with(
    rows: &[FossilRow],
    pool: &[Mod],
    wants: &[&BTreeSet<String>],
    prices: &Prices,
    settings: &Settings,
    opts: FossilOpts,
) -> (Vec<Action>, Profiles) {
    let wants: Vec<&BTreeSet<String>> = wants.iter().copied().filter(|w| !w.is_empty()).collect();
    if rows.is_empty() || wants.is_empty() {
        return (Vec::new(), Profiles::new());
    }
    let nw = wants.len();
    let base: Vec<f64> = pool.iter().map(|m| m.weight).collect();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for (i, m) in pool.iter().enumerate() {
        index.insert(&m.id, i);
    }
    // Pool positions of each want (ascending), and the side its first member sits on.
    let want_at: Vec<Vec<usize>> = wants
        .iter()
        .map(|w| {
            let mut v: Vec<usize> = w.iter().filter_map(|m| index.get(m.as_str()).copied()).collect();
            v.sort_unstable();
            v
        })
        .collect();
    let want_side: Vec<usize> = want_at.iter().map(|v| pool[v.first().copied().unwrap_or(0)].side.index()).collect();
    let is_suffix: Vec<bool> = pool.iter().map(|m| m.side == Side::Suffix).collect();
    let fossil_price: Vec<f64> = rows.iter().map(|r| prices[&r.pid]).collect();

    struct Combo {
        idx: [usize; 4],
        k: usize,
        cost: f64,
    }
    let mut combos: Vec<Combo> = Vec::new();
    let mut chances: Vec<f64> = Vec::new(); // combos x wants
    for k in 1..=opts.max_fossils.min(rows.len()).min(4) {
        if !prices.contains_key(RESONATORS[k]) {
            continue;
        }
        let mut all: Vec<[usize; 4]> = Vec::new();
        crate::combinations::for_each(rows.len(), k, |c| {
            let mut idx = [0; 4];
            idx[..k].copy_from_slice(c);
            all.push(idx);
        });
        let clicks = (2.0 + FOSSIL_SECONDS * k as f64) * settings.chaos_per_hour / 3600.0;
        let resonator = prices[RESONATORS[k]];
        let scored: Vec<(f64, Vec<f64>)> = all
            .par_iter()
            .map_init(
                || vec![0.0f64; pool.len()],
                |weights, idx| {
                    for (j, w) in weights.iter_mut().enumerate() {
                        let mut mult = rows[idx[0]].mult[j];
                        for &r in &idx[1..k] {
                            mult *= rows[r].mult[j];
                        }
                        *w = base[j] * mult;
                    }
                    let mut totals = [0.0f64; 2];
                    for (w, &suffix) in weights.iter().zip(&is_suffix) {
                        totals[suffix as usize] += w;
                    }
                    let p: Vec<f64> = (0..nw)
                        .map(|i| {
                            let total = totals[want_side[i]];
                            let hit: f64 = want_at[i].iter().map(|&j| weights[j]).sum();
                            let share = if total > 0.0 { hit / total } else { 0.0 };
                            1.0 - (1.0 - share.clamp(0.0, 1.0)).powf(opts.mods_per_side)
                        })
                        .collect();
                    let mut fossils = fossil_price[idx[0]];
                    for &r in &idx[1..k] {
                        fossils += fossil_price[r];
                    }
                    (fossils + resonator + clicks, p)
                },
            )
            .collect();
        for (idx, (cost, p)) in all.into_iter().zip(scored) {
            combos.push(Combo { idx, k, cost });
            chances.extend(p);
        }
    }

    let mut picked: Vec<Vec<usize>> = Vec::new();
    'sizes: for size in (1..=nw).rev() {
        let mut subset: Vec<usize> = (0..size).collect();
        loop {
            let mut best: Option<usize> = None;
            let mut best_score = f64::INFINITY;
            for (n, combo) in combos.iter().enumerate() {
                let p = &chances[n * nw..(n + 1) * nw];
                let mut prod = p[subset[0]];
                for &s in &subset[1..] {
                    prod *= p[s];
                }
                let per_hit = combo.cost / prod.max(1e-12);
                if per_hit < best_score {
                    best = Some(n);
                    best_score = per_hit;
                }
            }
            if let Some(n) = best.filter(|_| best_score < 1e11) {
                let c = combos[n].idx[..combos[n].k].to_vec();
                if !picked.contains(&c) {
                    picked.push(c);
                }
            }
            if picked.len() >= opts.keep {
                break 'sizes;
            }
            if !crate::combinations::next(&mut subset, nw) {
                break;
            }
        }
    }

    let (mut acts, mut profiles) = (Vec::new(), Profiles::new());
    for combo in picked {
        let name = combo.iter().map(|&i| rows[i].name.as_str()).collect::<Vec<_>>().join("+");
        let weights = (0..pool.len()).map(|j| {
            let mut mult = rows[combo[0]].mult[j];
            for &r in &combo[1..] {
                mult *= rows[r].mult[j];
            }
            base[j] * mult
        });
        let profile = format!("fossil:{name}");
        profiles.insert(profile.clone(), pool.iter().zip(weights).map(|(m, w)| (m.id.clone(), w)).collect());
        let mut cost: Cost = combo.iter().map(|&i| (rows[i].pid.clone(), 1.0)).collect();
        cost.push((RESONATORS[combo.len()].to_string(), 1.0));
        let mut a = act(&format!("{name} fossil{}", if combo.len() > 1 { "s" } else { "" }), ActionKind::RerollRare, &[], &[NORMAL, RARE])
            .with_profile(&profile)
            .with_seconds(2.0 + FOSSIL_SECONDS * combo.len() as f64);
        a.cost = cost;
        acts.push(a);
    }
    (acts, profiles)
}
