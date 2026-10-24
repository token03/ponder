//! The crafting actions available on an item, with costs in
//! poe.ninja price ids.
//!
//! Game data (essences, fossils, the bench) comes from the data directory;
//! a missing file panics with the usual "run `ponder data fetch`" message.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use ponder_core::{Action, FOSSIL_SECONDS, MAGIC, Mod, NORMAL, Prices, RARE, Settings, Side};
use ponder_game::data::{self, DataError};
use rayon::prelude::*;

/// `[(price id, quantity)]`
pub type Cost = Vec<(String, f64)>;
/// weight profile name -> mod id -> weight
pub type Profiles = BTreeMap<String, BTreeMap<String, f64>>;

pub(crate) fn loaded<T>(r: Result<Arc<T>, DataError>) -> Arc<T> {
    r.unwrap_or_else(|e| panic!("{e}"))
}

fn act(name: &str, kind: &str, cost: &[(&str, f64)], from: &[u8]) -> Action {
    Action::new(name, kind, cost, from)
}

fn owned(cost: &[(&str, f64)]) -> Cost {
    cost.iter().map(|(p, q)| (p.to_string(), *q)).collect()
}

/// The plain currency actions every item has.
pub fn basic() -> Vec<Action> {
    vec![
        act("transmute", "transmute", &[("transmute", 1.0)], &[NORMAL]),
        act("alt", "alt", &[("alt", 1.0)], &[MAGIC]),
        act("aug", "aug", &[("aug", 1.0)], &[MAGIC]),
        act("regal", "regal", &[("regal", 1.0)], &[MAGIC]),
        act("exalt", "exalt", &[("exalted", 1.0)], &[RARE]),
        act("annul", "annul", &[("annul", 1.0)], &[RARE]),
        act("alch", "reroll_rare", &[("alch", 1.0)], &[NORMAL]),
        act("chaos", "reroll_rare", &[("chaos", 1.0)], &[RARE]),
        act(
            "scour+alch",
            "reroll_rare",
            &[("scour", 1.0), ("alch", 1.0)],
            &[RARE],
        ),
        act(
            "scour+transmute",
            "reroll_magic",
            &[("scour", 1.0), ("transmute", 1.0)],
            &[RARE],
        ),
    ]
}

/// Harvest "Reforge a Rare item with new random modifiers, including a <tag> modifier":
/// (harvest tag, mod implicit tag, lifeforce price id, amount).
/// Fire/cold/lightning costs are from Maxroll and PoEDB; the rest are unverified
/// guesses, set to the dearest colour so they don't look better than they are.
pub const HARVEST: [(&str, &str, &str, f64); 11] = [
    ("fire", "fire", "wild-lifeforce", 50.0),
    ("cold", "cold", "vivid-lifeforce", 50.0),
    ("lightning", "lightning", "primal-lifeforce", 50.0),
    ("physical", "physical", "vivid-lifeforce", 50.0),
    ("chaos", "chaos", "vivid-lifeforce", 50.0),
    ("life", "life", "vivid-lifeforce", 75.0),
    ("defence", "defences", "vivid-lifeforce", 50.0),
    ("attack", "attack", "vivid-lifeforce", 50.0),
    ("caster", "caster", "vivid-lifeforce", 50.0),
    ("speed", "speed", "vivid-lifeforce", 50.0),
    ("critical", "critical", "vivid-lifeforce", 50.0),
];
pub const RESONATOR: &str = "primitive-chaotic-resonator";
/// Chaotic resonators by socket count (index 0 is unused): a reroll with 1-4 fossils.
pub const RESONATORS: [&str; 5] = [
    "",
    RESONATOR,
    "potent-chaotic-resonator",
    "powerful-chaotic-resonator",
    "prime-chaotic-resonator",
];
pub const FOSSIL_COMBOS_KEPT: usize = 10; // combo profiles handed to the solver

/// Crafting bench costs are game metadata ids; these are their poe.ninja price ids.
pub const BENCH_CURRENCY: [(&str, &str); 16] = [
    ("CurrencyUpgradeToMagic", "transmute"),
    ("CurrencyRerollMagic", "alt"),
    ("CurrencyAddModToMagic", "aug"),
    ("CurrencyUpgradeMagicToRare", "regal"),
    ("CurrencyUpgradeToRare", "alch"),
    ("CurrencyRerollRare", "chaos"),
    ("CurrencyAddModToRare", "exalted"),
    ("CurrencyUpgradeRandomly", "chance"),
    ("CurrencyModValues", "divine"),
    ("CurrencyCorrupt", "vaal"),
    ("CurrencyConvertToNormal", "scour"),
    ("CurrencyInstillingOrb", "instilling-orb"),
    ("CurrencyRerollSocketNumbers", "jewellers"),
    ("CurrencyRerollSocketLinks", "fusing"),
    ("CurrencyRerollSocketColours", "chrome"),
    ("CurrencyArmourQuality", "scrap"),
];
pub const LOCK_PREFIXES: &str = "StrMasterItemGenerationCannotChangePrefixes"; // a suffix
pub const LOCK_SUFFIXES: &str = "DexMasterItemGenerationCannotChangeSuffixes"; // a prefix
pub const MULTIMOD: &str = "StrIntMasterItemGenerationCanHaveMultipleCraftedMods"; // a suffix

/// Harvest reforges whose tag at least one mod in the pool carries.
pub fn harvest(pool: &[Mod]) -> (Vec<Action>, Profiles) {
    let (mut acts, mut profiles) = (Vec::new(), Profiles::new());
    for (name, tag, lifeforce, amount) in HARVEST {
        let mut tagged = BTreeMap::new();
        for m in pool.iter().filter(|m| m.tags.iter().any(|t| t == tag)) {
            tagged.insert(m.id.clone(), m.weight);
        }
        if tagged.is_empty() {
            continue;
        }
        let profile = format!("harvest:{name}");
        profiles.insert(profile.clone(), tagged);
        acts.push(
            act(
                &format!("harvest {name}"),
                "reroll_rare",
                &[(lifeforce, amount)],
                &[RARE],
            )
            .with_first_profile(&profile),
        );
    }
    (acts, profiles)
}

/// Shrieking and Deafening essences that work on this item class.
///
/// On a normal item an essence makes it rare; on a rare it needs a scour first.
/// Returns the actions and the forced mod id of each (by "<name> essence", in order).
pub fn essences(
    item_class: &str,
    item_level: i64,
    min_level: i64,
) -> (Vec<Action>, Vec<(String, String)>) {
    let (mut acts, mut forced): (Vec<Action>, Vec<(String, String)>) = (Vec::new(), Vec::new());
    // The raw file: the typed loader rejects the null mods of corruption-only essences.
    let all = loaded(data::load("essences"));
    for essence in all.as_object().into_iter().flat_map(|o| o.values()) {
        let Some(mod_id) = essence["mods"][item_class]
            .as_str()
            .filter(|m| !m.is_empty())
        else {
            continue;
        };
        let restricted = essence["item_level_restriction"]
            .as_i64()
            .is_some_and(|r| r != 0 && item_level > r);
        let name = essence["name"].as_str().unwrap_or("");
        if essence["level"].as_i64().unwrap_or(0) < min_level
            || restricted
            || essence["type"]["is_corruption_only"]
                .as_bool()
                .unwrap_or(false)
        {
            continue;
        }
        let pid = slug(name);
        let name = name.replace("Essence of ", "").to_lowercase();
        let label = format!("{name} essence");
        acts.push(act(&label, "reroll_rare", &[(&pid, 1.0)], &[NORMAL]).with_forced(mod_id));
        acts.push(
            act(
                &format!("scour+{name} essence"),
                "reroll_rare",
                &[("scour", 1.0), (&pid, 1.0)],
                &[RARE],
            )
            .with_forced(mod_id),
        );
        match forced.iter_mut().find(|(k, _)| *k == label) {
            Some(e) => e.1 = mod_id.to_string(), // a dict: the later essence of that name wins its place
            None => forced.push((label, mod_id.to_string())),
        }
    }
    (acts, forced)
}

/// Single fossils (with a Primitive Resonator) that change this pool's weights.
pub fn fossils(pool: &[Mod]) -> (Vec<Action>, Profiles) {
    let (mut acts, mut profiles) = (Vec::new(), Profiles::new());
    for fossil in loaded(data::fossils()).values() {
        let Some(mult) = usable_fossil(fossil) else {
            continue;
        };
        let weights: Vec<f64> = pool
            .iter()
            .map(|m| {
                m.tags
                    .iter()
                    .filter_map(|t| mult.get(t.as_str()))
                    .fold(m.weight, |w, x| w * x)
            })
            .collect();
        if pool
            .iter()
            .zip(&weights)
            .all(|(m, w)| (w - m.weight).abs() < 1e-9)
        {
            continue; // changes nothing here
        }
        let name = fossil.name.replace(" Fossil", "").to_lowercase();
        let profile = format!("fossil:{name}");
        profiles.insert(
            profile.clone(),
            pool.iter()
                .zip(weights)
                .map(|(m, w)| (m.id.clone(), w))
                .collect(),
        );
        acts.push(
            act(
                &format!("{name} fossil"),
                "reroll_rare",
                &[(&slug(&fossil.name), 1.0), (RESONATOR, 1.0)],
                &[NORMAL, RARE],
            )
            .with_profile(&profile),
        );
    }
    (acts, profiles)
}

/// The weight multiplier per tag of a plain fossil (no added or forced mods, no tricks), if it has any.
fn usable_fossil(f: &data::Fossil) -> Option<HashMap<&str, f64>> {
    if f.name.is_empty()
        || !f.added_mods.is_empty()
        || !f.forced_mods.is_empty()
        || f.mirrors
        || f.rolls_lucky
        || f.changes_quality
    {
        return None;
    }
    let mult: HashMap<&str, f64> = f
        .positive_mod_weights
        .iter()
        .chain(&f.negative_mod_weights)
        .map(|w| (w.tag.as_str(), w.weight / 100.0))
        .collect();
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
        let row: Vec<f64> = pool
            .iter()
            .map(|m| {
                m.tags
                    .iter()
                    .filter_map(|t| mult.get(t.as_str()))
                    .fold(1.0, |p, x| p * x)
            })
            .collect();
        if row.iter().all(|x| (x - 1.0).abs() <= 1e-8 + 1e-5) {
            continue; // np.allclose(row, 1.0): changes nothing here
        }
        rows.push(FossilRow {
            name: fossil.name.replace(" Fossil", "").to_lowercase(),
            pid: slug(&fossil.name),
            mult: row,
        });
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
        FossilOpts {
            mods_per_side: 2.3,
            max_fossils: 4,
            keep: FOSSIL_COMBOS_KEPT,
        }
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
    let rows: Vec<FossilRow> = fossil_rows(pool)
        .into_iter()
        .filter(|r| prices.contains_key(&r.pid))
        .collect();
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
            let mut v: Vec<usize> = w
                .iter()
                .filter_map(|m| index.get(m.as_str()).copied())
                .collect();
            v.sort_unstable();
            v
        })
        .collect();
    let want_side: Vec<usize> = want_at
        .iter()
        .map(|v| pool[v.first().copied().unwrap_or(0)].side.index())
        .collect();
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
        let mut c: Vec<usize> = (0..k).collect();
        loop {
            let mut idx = [0; 4];
            idx[..k].copy_from_slice(&c);
            all.push(idx);
            if !next_combination(&mut c, rows.len()) {
                break;
            }
        }
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
            if !next_combination(&mut subset, nw) {
                break;
            }
        }
    }

    let (mut acts, mut profiles) = (Vec::new(), Profiles::new());
    for combo in picked {
        let name = combo
            .iter()
            .map(|&i| rows[i].name.as_str())
            .collect::<Vec<_>>()
            .join("+");
        let weights = (0..pool.len()).map(|j| {
            let mut mult = rows[combo[0]].mult[j];
            for &r in &combo[1..] {
                mult *= rows[r].mult[j];
            }
            base[j] * mult
        });
        let profile = format!("fossil:{name}");
        profiles.insert(
            profile.clone(),
            pool.iter()
                .zip(weights)
                .map(|(m, w)| (m.id.clone(), w))
                .collect(),
        );
        let mut cost: Cost = combo.iter().map(|&i| (rows[i].pid.clone(), 1.0)).collect();
        cost.push((RESONATORS[combo.len()].to_string(), 1.0));
        let mut a = act(
            &format!("{name} fossil{}", if combo.len() > 1 { "s" } else { "" }),
            "reroll_rare",
            &[],
            &[NORMAL, RARE],
        )
        .with_profile(&profile);
        a.cost = cost;
        acts.push(a);
    }
    (acts, profiles)
}

/// The next k-combination of 0..n in lexicographic order (itertools.combinations); false after the last.
fn next_combination(c: &mut [usize], n: usize) -> bool {
    let k = c.len();
    let Some(i) = (0..k).rev().find(|&i| c[i] != i + n - k) else {
        return false;
    };
    c[i] += 1;
    for j in i + 1..k {
        c[j] = c[j - 1] + 1;
    }
    true
}

type BenchIndex = HashMap<String, HashMap<String, Cost>>;

/// mod id -> item class -> cost, from the first bench option (in file order) that adds the mod to the class.
fn bench_index() -> &'static BenchIndex {
    static INDEX: OnceLock<BenchIndex> = OnceLock::new();
    INDEX.get_or_init(|| {
        let mut index: BenchIndex = HashMap::new();
        for option in loaded(data::crafting_bench_options()).iter() {
            let Some(mod_id) = option.add_explicit_mod() else {
                continue;
            };
            let cost: Cost = option
                .cost
                .iter()
                .map(|(k, n)| {
                    let id = k.rsplit('/').next().unwrap_or(k);
                    (
                        BENCH_CURRENCY
                            .iter()
                            .find(|(c, _)| *c == id)
                            .map_or(id, |(_, p)| p)
                            .to_string(),
                        *n,
                    )
                })
                .collect();
            let by_class = index.entry(mod_id.to_string()).or_default();
            for class in &option.item_classes {
                by_class
                    .entry(class.clone())
                    .or_insert_with(|| cost.clone());
            }
        }
        index
    })
}

/// What the crafting bench charges to add `mod_id`, or None if it can't.
pub fn bench_cost(item_class: &str, mod_id: &str) -> Option<Cost> {
    bench_index().get(mod_id)?.get(item_class).cloned()
}

/// Lock one side with a bench metamod, then scour away the other side, or reroll it.
///
/// `rerolls` are reforges (e.g. Harvest) worth trying on a locked item; a
/// chaos orb is always tried. Each becomes "lock <side>+<reroll>".
pub fn metamods(item_class: &str, rerolls: &[Action]) -> Vec<Action> {
    let mut acts = Vec::new();
    for (mod_id, side) in [(LOCK_PREFIXES, "prefixes"), (LOCK_SUFFIXES, "suffixes")] {
        let Some(cost) = bench_cost(item_class, mod_id) else {
            continue;
        };
        let with =
            |extra: &[(&str, f64)]| -> Cost { cost.iter().cloned().chain(owned(extra)).collect() };
        let mut keep = act(
            &format!("lock {side}+scour"),
            &format!("keep_{side}"),
            &[],
            &[RARE],
        );
        keep.cost = with(&[("scour", 1.0)]);
        acts.push(keep);
        let mut chaos = act(
            &format!("lock {side}+chaos"),
            &format!("lockreroll_{side}"),
            &[],
            &[RARE],
        );
        chaos.cost = with(&[("chaos", 1.0)]);
        acts.push(chaos);
        for r in rerolls {
            let mut a = act(
                &format!("lock {side}+{}", r.name),
                &format!("lockreroll_{side}"),
                &[],
                &[RARE],
            );
            a.cost = cost.iter().cloned().chain(r.cost.iter().cloned()).collect();
            a.profile = r.profile.clone();
            a.first_profile = r.first_profile.clone();
            acts.push(a);
        }
    }
    acts
}

pub const ELDRITCH_CLASSES: [&str; 4] = ["Helmet", "Body Armour", "Gloves", "Boots"];

/// Eldritch Exalted Orb / Orb of Annulment: add or remove a mod on one side only.
///
/// They need eldritch implicits (Exarch/Eater, from embers and ichors) whose
/// dominance picks the side, and don't work on influenced items. Setting up
/// the implicits is cheap next to the orbs and isn't counted.
pub fn eldritch(item_class: &str, influences: &BTreeSet<String>) -> Vec<Action> {
    if !ELDRITCH_CLASSES.contains(&item_class) || !influences.is_empty() {
        return Vec::new();
    }
    vec![
        act(
            "eldritch exalt (prefix)",
            "exalt_prefix",
            &[("eldritch-exalted-orb", 1.0)],
            &[RARE],
        ),
        act(
            "eldritch exalt (suffix)",
            "exalt_suffix",
            &[("eldritch-exalted-orb", 1.0)],
            &[RARE],
        ),
        act(
            "eldritch annul (prefix)",
            "annul_prefix",
            &[("eldritch-orb-of-annulment", 1.0)],
            &[RARE],
        ),
        act(
            "eldritch annul (suffix)",
            "annul_suffix",
            &[("eldritch-orb-of-annulment", 1.0)],
            &[RARE],
        ),
    ]
}

/// A price id: lowercase, apostrophes dropped, runs of anything else become one dash.
pub fn slug(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.to_lowercase().chars().filter(|&c| c != '\'') {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else {
            dash = true;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs() {
        assert_eq!(
            slug("Deafening Essence of Greed"),
            "deafening-essence-of-greed"
        );
        assert_eq!(slug("Prismatic Fossil"), "prismatic-fossil");
        assert_eq!(slug("Orb of Horizons' (x)"), "orb-of-horizons-x");
    }

    #[test]
    fn combinations_are_lexicographic() {
        let mut c = vec![0, 1];
        let mut seen = vec![c.clone()];
        while next_combination(&mut c, 4) {
            seen.push(c.clone());
        }
        assert_eq!(seen, [[0, 1], [0, 2], [0, 3], [1, 2], [1, 3], [2, 3]]);
    }

    fn pool() -> Vec<Mod> {
        let mut pool = vec![
            Mod::new("x", Side::Prefix, 100.0, &["X"], false, "x", &["fire"]),
            Mod::new("y", Side::Suffix, 100.0, &["Y"], false, "y", &["cold"]),
        ];
        for i in 0..3 {
            pool.push(Mod::new(
                &format!("p{i}"),
                Side::Prefix,
                1000.0,
                &[&format!("P{i}")],
                false,
                "junk",
                &[],
            ));
            pool.push(Mod::new(
                &format!("s{i}"),
                Side::Suffix,
                1000.0,
                &[&format!("S{i}")],
                false,
                "junk",
                &[],
            ));
        }
        pool
    }

    #[test]
    fn fossil_combos_pair_up_fossils_that_each_help_one_wanted_mod() {
        let pool = pool();
        let row = |name: &str, pid: &str, f: &dyn Fn(&Mod) -> f64| FossilRow {
            name: name.into(),
            pid: pid.into(),
            mult: pool.iter().map(f).collect(),
        };
        let rows = [
            row("hot", "hot-fossil", &|m| {
                if m.tags.iter().any(|t| t == "fire") {
                    10.0
                } else {
                    1.0
                }
            }),
            row("icy", "icy-fossil", &|m| {
                if m.tags.iter().any(|t| t == "cold") {
                    10.0
                } else {
                    1.0
                }
            }),
            row("dud", "dud-fossil", &|m| {
                if m.tags.is_empty() { 0.5 } else { 1.0 }
            }),
        ];
        let mut prices: Prices = [
            ("hot-fossil", 1.0),
            ("icy-fossil", 1.0),
            ("dud-fossil", 50.0),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), *v))
        .collect();
        prices.extend(RESONATORS[1..].iter().map(|r| (r.to_string(), 0.5)));
        let (x, y): (BTreeSet<String>, BTreeSet<String>) =
            (["x".to_string()].into(), ["y".to_string()].into());
        let (acts, profiles) = fossil_combos_with(
            &rows,
            &pool,
            &[&x, &y],
            &prices,
            &Settings::default(),
            FossilOpts::default(),
        );
        assert_eq!(acts[0].name, "hot+icy fossils"); // both wants: the pair, cheapest per hit
        assert!(acts[0].cost.contains(&(RESONATORS[2].to_string(), 1.0))); // two fossils take a potent resonator
        assert_eq!(profiles["fossil:hot+icy"]["x"], 1000.0);
        assert_eq!(profiles["fossil:hot+icy"]["p0"], 1000.0);
        let names: Vec<&str> = acts.iter().map(|a| a.name.as_str()).collect();
        assert!(names.contains(&"hot fossil") && names.contains(&"icy fossil")); // each want alone: its own fossil
    }

    #[test]
    fn eldritch_needs_a_plain_armour_slot() {
        assert!(eldritch("Gloves", &["shaper".to_string()].into()).is_empty());
        assert!(eldritch("Ring", &BTreeSet::new()).is_empty());
        assert_eq!(eldritch("Gloves", &BTreeSet::new()).len(), 4);
    }

    #[test]
    fn harvest_only_offers_tags_the_pool_has() {
        let (acts, profiles) = harvest(&pool());
        let names: Vec<&str> = acts.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["harvest fire", "harvest cold"]);
        assert_eq!(profiles["harvest:fire"].len(), 1);
        assert_eq!(acts[0].first_profile.as_deref(), Some("harvest:fire"));
    }
}
