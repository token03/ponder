use super::{Cost, Profiles, act, loaded, slug};
use ponder_core::{Action, ActionKind, FOSSIL_SECONDS, Mod, NORMAL, Prices, RARE, Settings, Side};
use ponder_game::data;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use crate::memo::Memo;

pub const RESONATOR: &str = "primitive-chaotic-resonator";
/// Chaotic resonators by socket count (index 0 is unused): a reroll with 1-4 fossils.
pub const RESONATORS: [&str; 5] = ["", RESONATOR, "potent-chaotic-resonator", "powerful-chaotic-resonator", "prime-chaotic-resonator"];
pub const FOSSIL_COMBOS_KEPT: usize = 10; // combo profiles handed to the solver

/// Single fossils (with a Primitive Resonator) that change this pool's weights.
pub fn fossils(pool: &[Mod]) -> (Vec<Action>, Profiles) {
    let mut actions = Vec::new();
    let mut profiles = Profiles::new();
    for row in fossil_rows(pool) {
        let profile = format!("fossil:{}", row.name);
        profiles.insert(
            profile.clone(),
            pool.iter()
                .zip(&row.effects)
                .map(|(modifier, effect)| (modifier.id.clone(), modifier.weight * effect.multiplier()))
                .collect::<BTreeMap<_, _>>()
                .into(),
        );
        actions.push(
            act(&format!("{} fossil", row.name), ActionKind::RerollRare, &[(&row.pid, 1.0), (RESONATOR, 1.0)], &[NORMAL, RARE])
                .with_profile(&profile),
        );
    }
    (actions, profiles)
}

/// The weight multiplier per tag of a plain fossil (no added or forced mods, no tricks), if it has any.
fn usable_fossil(f: &data::Fossil) -> Option<HashMap<&str, FossilEffect>> {
    if f.name.is_empty() || !f.added_mods.is_empty() || !f.forced_mods.is_empty() || f.mirrors || f.rolls_lucky || f.changes_quality {
        return None;
    }
    let mut effects: HashMap<&str, FossilEffect> = HashMap::new();
    for weight in f.positive_mod_weights.iter().chain(&f.negative_mod_weights) {
        let effect = effects.entry(weight.tag.as_str()).or_default();
        *effect = effect.combine(FossilEffect::from(weight.weight / 100.0));
    }
    (!effects.is_empty()).then_some(effects)
}

#[derive(Clone, Copy, Debug)]
pub struct FossilEffect {
    more: f64,
    less: f64,
}

impl Default for FossilEffect {
    fn default() -> Self {
        Self { more: 0.0, less: 1.0 }
    }
}

impl From<f64> for FossilEffect {
    fn from(factor: f64) -> Self {
        if factor > 1.0 { Self { more: factor, less: 1.0 } } else { Self { more: 0.0, less: factor } }
    }
}

impl FossilEffect {
    fn combine(self, other: Self) -> Self {
        Self { more: self.more + other.more, less: self.less * other.less }
    }

    fn multiplier(self) -> f64 {
        self.more.max(1.0) * self.less
    }
}

pub fn fossil_multiplier(effects: impl IntoIterator<Item = FossilEffect>) -> f64 {
    effects.into_iter().fold(FossilEffect::default(), FossilEffect::combine).multiplier()
}

/// A weight-changing fossil: its name, price id, and per-mod weight multiplier.
#[derive(Clone, Debug)]
pub struct FossilRow {
    pub name: String,
    pub pid: String,
    pub effects: Vec<FossilEffect>,
}

/// (name, price id, per-mod weight multiplier) for every weight-changing fossil.
///
pub fn fossil_rows(pool: &[Mod]) -> Vec<FossilRow> {
    let mut rows = Vec::new();
    for fossil in loaded(data::fossils()).values() {
        let Some(mult) = usable_fossil(fossil) else {
            continue;
        };
        let row: Vec<_> = pool
            .iter()
            .map(|m| m.tags.iter().filter_map(|t| mult.get(t.as_str())).copied().fold(FossilEffect::default(), FossilEffect::combine))
            .collect();
        if row.iter().all(|effect| effect.multiplier() == 1.0) {
            continue; // np.allclose(row, 1.0): changes nothing here
        }
        rows.push(FossilRow { name: fossil.name.replace(" Fossil", "").to_lowercase(), pid: slug(&fossil.name), effects: row });
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

struct Combo {
    idx: [usize; 4],
    k: usize,
    weights: Vec<f64>,
    totals: [f64; 2],
    profile: OnceLock<ponder_core::WeightProfile>,
}

fn combination_weights(rows: &[FossilRow], pool: &[Mod], base: &[f64], is_suffix: &[bool], max_fossils: usize) -> Arc<Vec<Combo>> {
    static CACHE: Memo<blake3::Hash, Arc<Vec<Combo>>> = Memo::new();
    let mut key = blake3::Hasher::new();
    for size in [base.len(), rows.len(), max_fossils] {
        key.update(&(size as u64).to_le_bytes());
    }
    for &weight in base {
        key.update(&weight.to_bits().to_le_bytes());
    }
    for m in pool {
        key.update(&(m.id.len() as u64).to_le_bytes());
        key.update(m.id.as_bytes());
    }
    for &suffix in is_suffix {
        key.update(&[u8::from(suffix)]);
    }
    for row in rows {
        for effect in &row.effects {
            key.update(&effect.more.to_bits().to_le_bytes());
            key.update(&effect.less.to_bits().to_le_bytes());
        }
    }
    CACHE.get_or_init(key.finalize(), || {
        let mut combos = Vec::new();
        for k in 1..=max_fossils.min(rows.len()).min(4) {
            crate::combinations::for_each(rows.len(), k, |indices| {
                let mut idx = [0; 4];
                idx[..k].copy_from_slice(indices);
                let weights: Vec<f64> = base
                    .iter()
                    .enumerate()
                    .map(|(j, weight)| weight * fossil_multiplier(idx[..k].iter().map(|&r| rows[r].effects[j])))
                    .collect();
                let mut totals = [0.0; 2];
                for (weight, &suffix) in weights.iter().zip(is_suffix) {
                    totals[suffix as usize] += weight;
                }
                combos.push(Combo { idx, k, weights, totals, profile: OnceLock::new() });
            });
        }
        Arc::new(combos)
    })
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
    let combos = combination_weights(rows, pool, &base, &is_suffix, opts.max_fossils);
    let costs: Vec<_> = combos
        .iter()
        .map(|combo| {
            let Some(&resonator) = prices.get(RESONATORS[combo.k]) else { return f64::INFINITY };
            let mut fossils = prices[&rows[combo.idx[0]].pid];
            for &r in &combo.idx[1..combo.k] {
                fossils += prices[&rows[r].pid];
            }
            fossils + resonator + (2.0 + FOSSIL_SECONDS * combo.k as f64) * settings.chaos_per_hour / 3600.0
        })
        .collect();
    let mut chances: Vec<f64> = Vec::new(); // combos x wants
    for combo in combos.iter() {
        chances.extend((0..nw).map(|i| {
            let total = combo.totals[want_side[i]];
            let hit: f64 = want_at[i].iter().map(|&j| combo.weights[j]).sum();
            let share = if total > 0.0 { hit / total } else { 0.0 };
            1.0 - (1.0 - share.clamp(0.0, 1.0)).powf(opts.mods_per_side)
        }));
    }

    let mut picked: Vec<usize> = Vec::new();
    'sizes: for size in (1..=nw).rev() {
        let mut subset: Vec<usize> = (0..size).collect();
        loop {
            let mut best: Option<usize> = None;
            let mut best_score = f64::INFINITY;
            for (n, &cost) in costs.iter().enumerate() {
                let p = &chances[n * nw..(n + 1) * nw];
                let mut prod = p[subset[0]];
                for &s in &subset[1..] {
                    prod *= p[s];
                }
                let per_hit = cost / prod.max(1e-12);
                if per_hit < best_score {
                    best = Some(n);
                    best_score = per_hit;
                }
            }
            if let Some(n) = best.filter(|_| best_score < 1e11)
                && !picked.contains(&n)
            {
                picked.push(n);
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
    for n in picked {
        let selected = &combos[n];
        let combo = &selected.idx[..selected.k];
        let name = combo.iter().map(|&i| rows[i].name.as_str()).collect::<Vec<_>>().join("+");
        let profile = format!("fossil:{name}");
        let weights = selected
            .profile
            .get_or_init(|| pool.iter().zip(&selected.weights).map(|(m, &w)| (m.id.clone(), w)).collect::<BTreeMap<_, _>>().into());
        profiles.insert(profile.clone(), weights.clone());
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
