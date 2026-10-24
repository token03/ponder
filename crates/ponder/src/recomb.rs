//! Recombinator plans: build two clean donors, recombine them into the target.
//!
//! The model, in short: the
//! output keeps a number of mods per side drawn from a table that depends only on the side's
//! pool size, mods that cannot roll on the output's base (non-native natural, "NNN") count
//! towards the pool and are then dropped, so clean donors padded with NNN mods raise the
//! chance of keeping every wanted mod. A plan is a tree of clean donors (ordinary crafts,
//! solved like any other item) merged by recombinations; its cost divides by the chance and
//! is reduced by what failures hand back.
//!
//! Where Python's `Donor` and `Step` share objects, the finished tree here holds cheap
//! snapshots ([`Leaf`], [`Node`]) so the planner can go on owning the donors' problems.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::Arc;

use indexmap::IndexMap;
use ponder_core::{Plan, Problem, Rng, Side, Target, solve};
use ponder_game::mods::ModDB;
use rayon::prelude::*;
use serde_json::{Value, json};

use crate::actions;
use crate::gear::{self, DEFAULT_ITEM_LEVEL, Pricing};

/// Chance the output keeps k mods on a side, by that side's pool size (both inputs): `(k, p)` rows.
///
/// The poewiki "Estimated modifier count distribution" (from u/Butsicles' guide; page as of
/// Aug 2026). Pools over 6 are unmeasured and use row 6. Edit here when a patch changes them.
pub const SIDE_COUNTS: [&[(usize, f64)]; 7] = [
    &[(0, 1.0)],
    &[(0, 0.41), (1, 0.59)],
    &[(1, 0.67), (2, 0.33)],
    &[(1, 0.40), (2, 0.50), (3, 0.10)],
    &[(1, 0.10), (2, 0.60), (3, 0.30)],
    &[(2, 0.43), (3, 0.57)],
    &[(2, 0.30), (3, 0.70)],
];
pub const TABLE_VERSION: &str = "poewiki / Butsicles (Aug 2026)";
/// Tierless essence mods are "exclusive", and 3.26 cut the output's mod count when inputs
/// carry mods that can't normally roll on the item: essence-forced NNN pads no longer pay.
pub const ESSENCE_PADS: bool = false;
/// A wanted mod on both inputs counts twice in the pool, but once one copy is picked its mod
/// group blocks the other: free padding on the target base.
pub const DOUBLING: bool = true;
/// NNN pads per side on one donor.
pub const MAX_SIDE_PADS: usize = 2;
/// Wanted + pads on one donor.
pub const MAX_LEAF_MODS: usize = 4;
/// Chaos per recombination, on top of the donors.
pub const RECOMB_FEE: f64 = 0.0;
/// Loading the recombinator and checking the result, charged at the clicking rate.
pub const RECOMB_SECONDS: f64 = 15.0;
pub const SIDES: [Side; 2] = [Side::Prefix, Side::Suffix];

/// Per side (prefix, suffix): (wanted mods, NNN pads) on one input.
pub type Counts = [(i64, i64); 2];

fn row(pool_size: usize) -> &'static [(usize, f64)] {
    SIDE_COUNTS[pool_size.min(6)]
}

/// Chance a side with `pool_size` pooled mods keeps at least `wanted` of them.
pub fn keep_at_least(pool_size: usize, wanted: usize) -> f64 {
    row(pool_size)
        .iter()
        .filter(|(k, _)| *k >= wanted)
        .fold(0.0, |s, (_, p)| s + p)
}

fn comb(n: usize, k: usize) -> f64 {
    if k > n {
        return 0.0;
    }
    let k = k.min(n - k);
    let mut c: u128 = 1;
    for i in 0..k {
        c = c * (n - i) as u128 / (i + 1) as u128;
    }
    c as f64
}

/// Chance a side keeps exactly one particular `subset`-sized set of its `wanted` legal mods.
///
/// With clean donors only wanted mods are legal, so the side keeps min(k, wanted) of them,
/// uniformly: a particular set of that size is one of C(wanted, size).
pub fn keep_exactly(pool_size: usize, wanted: usize, subset: usize) -> f64 {
    if wanted == 0 {
        return 1.0;
    }
    row(pool_size)
        .iter()
        .filter(|(k, _)| (*k).min(wanted) == subset)
        .fold(0.0, |s, (_, p)| s + p)
        / comb(wanted, subset)
}

fn sizes(a: &Counts, b: &Counts, side: usize) -> (usize, usize, usize) {
    let ((wa, na), (wb, nb)) = (a[side], b[side]);
    (
        (wa + wb) as usize,
        (wa + na + wb + nb) as usize,
        wa as usize,
    )
}

/// Chance a failed recombination still gives a clean target-base copy of `a`'s wanted mods, and nothing else.
///
/// That output replaces input `a` on the next attempt, so `a` isn't crafted again.
pub fn salvage(a: &Counts, b: &Counts, same_base: bool) -> f64 {
    let mut p = if same_base { 1.0 } else { 0.5 };
    for side in 0..2 {
        let (wanted, pool, mine) = sizes(a, b, side);
        if wanted > 0 {
            p *= keep_exactly(pool, wanted, mine);
        }
    }
    p
}

/// Success chance for clean donors: per side, (wanted mods, NNN padding) on each donor.
///
/// Wanted mods are all distinct and legal on the target base; padding is illegal there.
/// Success: the target base, and every wanted mod kept.
pub fn chance(a: &Counts, b: &Counts, same_base: bool) -> f64 {
    let mut p = if same_base { 1.0 } else { 0.5 };
    for side in 0..2 {
        let (wanted, pool, _) = sizes(a, b, side);
        if wanted > 0 {
            p *= keep_at_least(pool, wanted);
        }
    }
    p
}

fn describe_parts(base: &str, carries: &[String], padding: &BTreeMap<String, usize>) -> String {
    let pad: Vec<String> = padding
        .iter()
        .filter(|(_, n)| **n > 0)
        .map(|(side, n)| format!("{n} NNN {side}"))
        .collect();
    let pad = pad.join(" + ");
    format!(
        "{base}: {}{}",
        carries.join(" + "),
        if pad.is_empty() {
            String::new()
        } else {
            format!(" + {pad}")
        }
    )
}

/// One recombinator input: a clean craft of some wanted mods (plus NNN padding) on a base.
#[derive(Clone, Debug)]
pub struct Donor {
    pub base: String,
    pub base_id: String,
    /// wanted target names
    pub carries: Vec<String>,
    /// side name -> NNN mods
    pub padding: BTreeMap<String, usize>,
    pub problem: Option<Problem>,
    pub plan: Option<Plan>,
    pub error: Option<String>,
    pub on_target: bool,
}

impl Donor {
    pub fn new(base: &str, base_id: &str, carries: &[&str]) -> Donor {
        Donor {
            base: base.into(),
            base_id: base_id.into(),
            carries: carries.iter().map(|s| s.to_string()).collect(),
            padding: BTreeMap::new(),
            problem: None,
            plan: None,
            error: None,
            on_target: true,
        }
    }

    pub fn cost(&self) -> Option<f64> {
        self.plan.as_ref().map(|p| p.cost)
    }

    /// (wanted mods, NNN pads) on `side`, for `chance`.
    pub fn counts(&self, side: Side, wants: &HashMap<String, Side>) -> (i64, i64) {
        leaf_counts(&self.carries, &self.padding, side, wants)
    }

    pub fn describe(&self) -> String {
        describe_parts(&self.base, &self.carries, &self.padding)
    }

    /// What a finished tree keeps of this donor.
    pub fn leaf(&self) -> Leaf {
        Leaf {
            base: self.base.clone(),
            base_id: self.base_id.clone(),
            carries: self.carries.clone(),
            padding: self.padding.clone(),
            cost: self.cost(),
            on_target: self.on_target,
        }
    }
}

fn leaf_counts(
    carries: &[String],
    padding: &BTreeMap<String, usize>,
    side: Side,
    wants: &HashMap<String, Side>,
) -> (i64, i64) {
    (
        carries.iter().filter(|n| wants[n.as_str()] == side).count() as i64,
        padding.get(side.name()).copied().unwrap_or(0) as i64,
    )
}

/// A donor as a recombination tree sees it, with its solved cost.
#[derive(Clone, Debug)]
pub struct Leaf {
    pub base: String,
    pub base_id: String,
    pub carries: Vec<String>,
    pub padding: BTreeMap<String, usize>,
    pub cost: Option<f64>,
    pub on_target: bool,
}

impl Leaf {
    pub fn describe(&self) -> String {
        describe_parts(&self.base, &self.carries, &self.padding)
    }
}

/// What the finished item needs, one mod: a target or a filler slot.
#[derive(Clone, Debug)]
pub struct Want {
    pub name: String,
    pub side: Side,
    /// None: one filler mod (any resistance)
    pub target: Option<Target>,
}

/// What the finished item needs, one entry per mod: targets, then each filler mod.
///
/// Filler slots get distinct resistance families (fire, cold, ... by weight), so two donors
/// can't both bring fire resistance and collide in the output. Equal weights keep the order
/// of the filler ids (sorted); Python's follows its set order, which varies between runs.
pub fn wants(problem: &Problem) -> Vec<Want> {
    let mut out: Vec<Want> = problem
        .targets
        .iter()
        .map(|t| Want {
            name: t.name.clone(),
            side: t.side,
            target: Some(t.clone()),
        })
        .collect();
    let need = problem.filler_need;
    if need == 0 {
        return out;
    }
    let mods = problem.mods();
    let mut families: IndexMap<String, BTreeSet<String>> = IndexMap::new();
    for m in &problem.filler {
        if let Some(mod_) = mods.get(m) {
            let key = mod_.groups.first().cloned().unwrap_or_else(|| m.clone());
            families.entry(key).or_default().insert(m.clone());
        }
    }
    let weight = |g: &String| -> f64 { families[g].iter().map(|m| mods[m].weight).sum() };
    let mut order: Vec<&String> = families.keys().collect();
    order.sort_by(|a, b| weight(b).total_cmp(&weight(a)));
    for i in 0..need {
        let mine: Vec<&String> = if order.len() > need {
            order.iter().skip(i).step_by(need).copied().collect()
        } else {
            order.iter().skip(i).take(1).copied().collect()
        };
        let ids: BTreeSet<String> = mine
            .iter()
            .flat_map(|g| families[*g].iter().cloned())
            .collect();
        let Some(first) = ids.iter().next() else {
            out.extend((i..need).map(|j| Want {
                name: format!("res {}", j + 1),
                side: Side::Suffix,
                target: None,
            }));
            return out;
        };
        let side = mods[first].side;
        let groups: BTreeSet<String> = ids
            .iter()
            .flat_map(|m| mods[m].groups.iter().cloned())
            .collect();
        let name = format!("res {}", i + 1);
        out.push(Want {
            name: name.clone(),
            side,
            target: Some(Target {
                name,
                side,
                good: ids,
                groups,
                notable: false,
            }),
        });
    }
    out
}

type Nnn = [BTreeSet<String>; 2];

/// Other bases of the same class whose native mods can't roll on `base_id`: (base id, side -> NNN mod ids).
///
/// One base per mod pool (the best), most NNN weight first.
pub fn partners(db: &ModDB, base_id: &str, item_level: i64, top: usize) -> Vec<(String, Nnn)> {
    let base = &db.bases[base_id];
    let used = gear::spawn_tags();
    let mine: HashSet<String> = gear::pool(
        db,
        base_id,
        &db.item_tags(base_id, &BTreeSet::new()),
        item_level,
    )
    .into_iter()
    .map(|m| m.id)
    .collect();
    let sig: BTreeSet<String> = db
        .item_tags(base_id, &BTreeSet::new())
        .intersection(used)
        .cloned()
        .collect();
    let mut seen: HashSet<BTreeSet<String>> = HashSet::new();
    let mut ranked: Vec<(f64, String, Nnn)> = Vec::new();
    for (other_id, other) in db.bases.iter() {
        if other.item_class != base.item_class
            || other.domain != base.domain
            || other.release_state != "released"
        {
            continue;
        }
        let osig: BTreeSet<String> = db
            .item_tags(other_id, &BTreeSet::new())
            .intersection(used)
            .cloned()
            .collect();
        if osig == sig || seen.contains(&osig) {
            continue;
        }
        let best = best_of(db, &base.item_class, &base.domain, &osig);
        seen.insert(osig);
        let mut nnn: Nnn = Default::default();
        let mut weight = 0.0;
        for m in gear::pool(
            db,
            &best,
            &db.item_tags(&best, &BTreeSet::new()),
            item_level,
        ) {
            if !mine.contains(&m.id) {
                weight += m.weight;
                nnn[m.side.index()].insert(m.id);
            }
        }
        if nnn.iter().any(|s| !s.is_empty()) {
            ranked.push((weight, best, nnn));
        }
    }
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
    ranked
        .into_iter()
        .take(top)
        .map(|(_, b, n)| (b, n))
        .collect()
}

fn best_of(db: &ModDB, item_class: &str, domain: &str, signature: &BTreeSet<String>) -> String {
    let used = gear::spawn_tags();
    let mut best: Option<(bool, bool, i64, &str)> = None;
    for (bid, b) in db.bases.iter() {
        let same =
            b.item_class == item_class && b.domain == domain && b.release_state == "released";
        if same
            && db
                .item_tags(bid, &BTreeSet::new())
                .intersection(used)
                .cloned()
                .collect::<BTreeSet<_>>()
                == *signature
        {
            let has = |t: &str| b.tags.iter().any(|x| x == t);
            let key = (
                !has("experimental_base"),
                has("top_tier_base_item_type"),
                b.drop_level,
                bid,
            );
            if best.is_none_or(|k| key > k) {
                best = Some(key);
            }
        }
    }
    best.expect("a base with this signature").3.to_string()
}

/// Essence-forced mods that can't roll on `base_id` but are ordinary ("named") mods elsewhere.
///
/// The recombinator counts them towards the pool size and then drops them, so they pad a
/// donor on the target base itself, with no base coin flip. Essence-only "of the Essence"
/// mods are special and stay in the draw; they're left out.
pub fn essence_nnn(db: &ModDB, base_id: &str, item_level: i64) -> Nnn {
    let item_class = &db.bases[base_id].item_class;
    let mine: HashSet<String> = gear::pool(
        db,
        base_id,
        &db.item_tags(base_id, &BTreeSet::new()),
        item_level,
    )
    .into_iter()
    .map(|m| m.id)
    .collect();
    let mut out: Nnn = Default::default();
    for (_, mod_id) in actions::essences(item_class, item_level, 6).1 {
        let Some(m) = db.mods.get(&mod_id) else {
            continue;
        };
        if mine.contains(&mod_id) || !matches!(m.generation_type.as_str(), "prefix" | "suffix") {
            continue;
        }
        if m.spawn_weights.iter().any(|w| w.weight > 0) {
            // rolls naturally somewhere: a named mod
            out[usize::from(m.generation_type == "suffix")].insert(mod_id);
        }
    }
    out
}

/// Split NNN mods into `n` sets of whole families, heaviest families spread first, so `n` pads are `n` distinct targets.
fn bins(db: &ModDB, ids: &BTreeSet<String>, n: usize) -> Vec<BTreeSet<String>> {
    let mut fams: IndexMap<&str, BTreeSet<String>> = IndexMap::new();
    for m in ids {
        fams.entry(db.mods[m.as_str()].type_.as_str())
            .or_default()
            .insert(m.clone());
    }
    let mut order: Vec<&str> = fams.keys().copied().collect();
    order.sort_by(|a, b| fams[b].len().cmp(&fams[a].len()).then(a.cmp(b)));
    let mut out: Vec<BTreeSet<String>> = vec![BTreeSet::new(); n];
    for (i, fam) in order.into_iter().enumerate() {
        out[i % n].extend(fams[fam].iter().cloned());
    }
    out.into_iter().filter(|b| !b.is_empty()).collect()
}

fn percent(x: f64) -> String {
    format!("{:.0}%", x * 100.0)
}

/// Either end of a recombination: a donor or an earlier step.
#[derive(Clone, Debug)]
pub enum Node {
    Donor(Arc<Leaf>),
    Step(Arc<Step>),
}

impl Node {
    pub fn cost(&self) -> Option<f64> {
        match self {
            Node::Donor(d) => d.cost,
            Node::Step(s) => s.cost,
        }
    }

    pub fn on_target(&self) -> bool {
        match self {
            Node::Donor(d) => d.on_target,
            Node::Step(_) => true,
        }
    }

    pub fn carries(&self) -> &[String] {
        match self {
            Node::Donor(d) => &d.carries,
            Node::Step(s) => &s.carries,
        }
    }

    /// (wanted mods, NNN pads) on `side`.
    pub fn counts(&self, side: Side, wants: &HashMap<String, Side>) -> (i64, i64) {
        match self {
            Node::Donor(d) => leaf_counts(&d.carries, &d.padding, side, wants),
            Node::Step(s) => s.counts(side, wants),
        }
    }
}

/// Recombine two clean items (donors or earlier steps) into one carrying both sets of mods.
#[derive(Clone, Debug)]
pub struct Step {
    pub a: Node,
    pub b: Node,
    pub chance: f64,
    /// Chance a failure hands back a clean copy of a (or of b, when b sits on the target
    /// base): that input is reused instead of crafted again.
    pub back_a: f64,
    pub back_b: f64,
    pub carries: Vec<String>,
    /// Wanted mods on both inputs: the copy inflates the pool, and its mod group blocks it from a second slot.
    pub doubled: Vec<String>,
    /// What an hour of clicking is worth, in chaos (the recombination's own seconds are charged at it).
    pub chaos_per_hour: f64,
    /// Expected chaos; None when an input has no cost or the chance is zero.
    pub cost: Option<f64>,
}

impl Step {
    pub fn new(
        a: Node,
        b: Node,
        chance: f64,
        back_a: f64,
        back_b: f64,
        chaos_per_hour: f64,
    ) -> Step {
        let mut carries = a.carries().to_vec();
        carries.extend(
            b.carries()
                .iter()
                .filter(|n| !a.carries().contains(n))
                .cloned(),
        );
        let doubled = b
            .carries()
            .iter()
            .filter(|n| a.carries().contains(n))
            .cloned()
            .collect();
        let cost = match (a.cost(), b.cost()) {
            (Some(ca), Some(cb)) if chance > 0.0 => Some(
                (ca * (1.0 - back_a)
                    + cb * (1.0 - back_b)
                    + RECOMB_FEE
                    + RECOMB_SECONDS * chaos_per_hour / 3600.0)
                    / chance,
            ),
            _ => None,
        };
        Step {
            a,
            b,
            chance,
            back_a,
            back_b,
            carries,
            doubled,
            chaos_per_hour,
            cost,
        }
    }

    /// NNN pads can't land on the target base: the output is clean.
    pub fn padding(&self) -> BTreeMap<String, usize> {
        BTreeMap::new()
    }

    pub fn on_target(&self) -> bool {
        true
    }

    pub fn counts(&self, side: Side, wants: &HashMap<String, Side>) -> (i64, i64) {
        (
            self.carries
                .iter()
                .filter(|n| wants[n.as_str()] == side)
                .count() as i64,
            0,
        )
    }

    pub fn donors(&self) -> Vec<&Leaf> {
        let mut out = Vec::new();
        for x in [&self.a, &self.b] {
            match x {
                Node::Donor(d) => out.push(&**d),
                Node::Step(s) => out.extend(s.donors()),
            }
        }
        out
    }

    /// The plan in order, as (what, detail, expected chaos): donors first, then each recombination.
    pub fn steps(&self) -> Vec<(String, String, Option<f64>)> {
        let mut out = Vec::new();
        self.walk(&mut out);
        out
    }

    fn walk(&self, out: &mut Vec<(String, String, Option<f64>)>) -> usize {
        let mut refs = Vec::new();
        for x in [&self.a, &self.b] {
            match x {
                Node::Step(s) => refs.push(s.walk(out)),
                Node::Donor(d) => {
                    out.push(("craft".into(), d.describe(), d.cost));
                    refs.push(out.len());
                }
            }
        }
        let mut notes: Vec<String> = refs
            .iter()
            .zip([self.back_a, self.back_b])
            .filter(|(_, q)| *q >= 0.005)
            .map(|(r, q)| format!("#{r} back {}", percent(q)))
            .collect();
        notes.extend(self.doubled.iter().map(|n| format!("{n} doubled")));
        let mut parts = vec![percent(self.chance)];
        parts.extend(notes);
        out.push((
            "recombine".into(),
            format!(
                "#{} + #{} \u{2192} {}  ({})",
                refs[0],
                refs[1],
                self.carries.join(" + "),
                parts.join("; ")
            ),
            self.cost,
        ));
        out.len()
    }

    pub fn to_dict(&self) -> Value {
        json!({
            "cost": self.cost, "chance": self.chance, "salvage": [self.back_a, self.back_b], "table": TABLE_VERSION,
            "steps": self.steps().into_iter().map(|(d, w, c)| json!({"do": d, "what": w, "chaos": c})).collect::<Vec<_>>(),
        })
    }
}

/// Every `r`-subset of `items`, in lexicographic index order (`itertools.combinations`).
fn combinations<T: Clone>(items: &[T], r: usize) -> Vec<Vec<T>> {
    let n = items.len();
    let mut out = Vec::new();
    if r > n {
        return out;
    }
    let mut idx: Vec<usize> = (0..r).collect();
    loop {
        out.push(idx.iter().map(|&i| items[i].clone()).collect());
        let Some(i) = (0..r).rev().find(|&i| idx[i] != i + n - r) else {
            return out;
        };
        idx[i] += 1;
        for j in i + 1..r {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

type LeafKey = (String, Vec<String>, Vec<(usize, Vec<Vec<String>>)>);

/// Recombination trees for one item: clean leaf donors, then the cheapest way to merge them.
///
/// Leaves carry at most `leaf_size` wanted mods (cheap, often magic: alt spam or one essence),
/// on the target base (optionally with an essence NNN pad) or on a partner base padded with its
/// native NNN mods. Bigger sets are built only by recombining smaller ones, cheapest first (a DP
/// over subsets), so a 4-mod item can come from two 2-mod recombinations. Solve `leaves_mut()`
/// (they are ordinary Problems), then call `best()`.
pub struct Planner {
    pub base_id: String,
    pub item_level: i64,
    pub want: Vec<Want>,
    pub side: HashMap<String, Side>,
    pub ok: bool,
    chaos_per_hour: f64,
    donors: Vec<Donor>,
    /// carried names -> donors (indices into `donors`), in creation order
    leaf_keys: IndexMap<BTreeSet<String>, Vec<usize>>,
    keys: HashMap<LeafKey, usize>,
    pools: HashMap<String, BTreeSet<String>>,
    filler: BTreeSet<String>,
}

impl Planner {
    pub fn new(
        db: &ModDB,
        problem: &Problem,
        base_id: &str,
        item_level: i64,
        pricing: Pricing,
    ) -> Planner {
        Planner::with_limits(db, problem, base_id, item_level, pricing, 2, 6)
    }

    pub fn with_limits(
        db: &ModDB,
        problem: &Problem,
        base_id: &str,
        item_level: i64,
        pricing: Pricing,
        leaf_size: usize,
        max_wants: usize,
    ) -> Planner {
        let want = wants(problem);
        let side = want.iter().map(|w| (w.name.clone(), w.side)).collect();
        let ok = problem.fixed.is_empty() && (2..=max_wants).contains(&want.len());
        let mut p = Planner {
            base_id: base_id.to_string(),
            item_level,
            want,
            side,
            ok,
            chaos_per_hour: pricing.settings.chaos_per_hour,
            donors: Vec::new(),
            leaf_keys: IndexMap::new(),
            keys: HashMap::new(),
            pools: HashMap::new(),
            filler: problem.filler.clone(),
        };
        if ok {
            p.build(db, leaf_size, pricing);
        }
        p
    }

    /// A planner over ready-made leaves (carried names -> donors), for tests: nothing is built.
    pub fn from_leaves(
        want: Vec<Want>,
        leaves: Vec<(Vec<&str>, Vec<Donor>)>,
        chaos_per_hour: f64,
    ) -> Planner {
        let side = want.iter().map(|w| (w.name.clone(), w.side)).collect();
        let mut p = Planner {
            base_id: String::new(),
            item_level: DEFAULT_ITEM_LEVEL,
            want,
            side,
            ok: true,
            chaos_per_hour,
            donors: Vec::new(),
            leaf_keys: IndexMap::new(),
            keys: HashMap::new(),
            pools: HashMap::new(),
            filler: BTreeSet::new(),
        };
        for (names, ds) in leaves {
            let idx = ds.into_iter().map(|d| {
                p.donors.push(d);
                p.donors.len() - 1
            });
            let idx: Vec<usize> = idx.collect();
            p.leaf_keys
                .insert(names.into_iter().map(String::from).collect(), idx);
        }
        p
    }

    /// Every distinct leaf donor, in creation order.
    pub fn leaves(&self) -> Vec<&Donor> {
        self.donors.iter().collect()
    }

    pub fn leaves_mut(&mut self) -> Vec<&mut Donor> {
        self.donors.iter_mut().collect()
    }

    /// The leaf donors by the wanted names they carry (several donors per set).
    pub fn leaf_sets(&self) -> Vec<(&BTreeSet<String>, Vec<&Donor>)> {
        self.leaf_keys
            .iter()
            .map(|(k, v)| (k, v.iter().map(|&i| &self.donors[i]).collect()))
            .collect()
    }

    fn pool(&mut self, db: &ModDB, bid: &str, item_level: i64) -> &BTreeSet<String> {
        self.pools.entry(bid.to_string()).or_insert_with(|| {
            gear::pool(db, bid, &db.item_tags(bid, &BTreeSet::new()), item_level)
                .into_iter()
                .map(|m| m.id)
                .collect()
        })
    }

    fn build(&mut self, db: &ModDB, leaf_size: usize, pricing: Pricing) {
        let ess = if ESSENCE_PADS {
            essence_nnn(db, &self.base_id, self.item_level)
        } else {
            Default::default()
        };
        let parts = partners(db, &self.base_id, self.item_level, 2);
        let room = |carried: &[Want], side: Side| {
            3 - carried.iter().filter(|w| w.side == side).count() as i64
        };
        // Pads that don't share a mod group with what the donor carries (they'd block it).
        let free = |ids: &BTreeSet<String>, carried: &[Want]| -> BTreeSet<String> {
            let taken: BTreeSet<&String> = carried
                .iter()
                .filter_map(|w| w.target.as_ref())
                .flat_map(|t| t.groups.iter())
                .collect();
            ids.iter()
                .filter(|m| !db.mods[m.as_str()].groups.iter().any(|g| taken.contains(g)))
                .cloned()
                .collect()
        };
        let want = self.want.clone();
        let base_id = self.base_id.clone();
        for size in 1..=leaf_size {
            for carried in combinations(&want, size) {
                if SIDES.iter().any(|&s| room(&carried, s) < 0) {
                    continue;
                }
                let key: BTreeSet<String> = carried.iter().map(|w| w.name.clone()).collect();
                let mut opts: Vec<Option<usize>> =
                    vec![self.donor(db, &base_id, &carried, &[], true, pricing)];
                if ESSENCE_PADS {
                    for s in SIDES {
                        if room(&carried, s) > 0 {
                            let ids = free(&ess[s.index()], &carried);
                            if !ids.is_empty() {
                                opts.push(self.donor(
                                    db,
                                    &base_id,
                                    &carried,
                                    &[(s, vec![ids])],
                                    true,
                                    pricing,
                                ));
                            }
                        }
                    }
                }
                for (bid, nnn) in &parts {
                    // partner bases, 0-2 NNN pads per side
                    let choices: Vec<Vec<usize>> = SIDES
                        .iter()
                        .map(|&s| {
                            if free(&nnn[s.index()], &carried).is_empty() {
                                vec![0]
                            } else {
                                let r = room(&carried, s) as usize;
                                BTreeSet::from([0, 1.min(r), MAX_SIDE_PADS.min(r)])
                                    .into_iter()
                                    .collect()
                            }
                        })
                        .collect();
                    for &c0 in &choices[0] {
                        for &c1 in &choices[1] {
                            if size + c0 + c1 > MAX_LEAF_MODS {
                                continue; // big padded donors cost far more than they add
                            }
                            let pads: Vec<(Side, Vec<BTreeSet<String>>)> = SIDES
                                .iter()
                                .zip([c0, c1])
                                .filter(|(_, n)| *n > 0)
                                .map(|(&s, n)| (s, bins(db, &free(&nnn[s.index()], &carried), n)))
                                .collect();
                            if !pads.is_empty() {
                                opts.push(self.donor(db, bid, &carried, &pads, false, pricing));
                            }
                        }
                    }
                }
                self.leaf_keys
                    .insert(key, opts.into_iter().flatten().collect());
            }
        }
    }

    /// The donor for these wanted mods and pads on `bid` (shared when the same one came up before), None if a wanted mod can't roll there.
    fn donor(
        &mut self,
        db: &ModDB,
        bid: &str,
        carried: &[Want],
        pads: &[(Side, Vec<BTreeSet<String>>)],
        on_target: bool,
        pricing: Pricing,
    ) -> Option<usize> {
        let item_level = self.item_level;
        let pool = self.pool(db, bid, item_level).clone();
        let mut targets = Vec::new();
        for w in carried {
            if let Some(t) = &w.target {
                let good: BTreeSet<String> = t.good.intersection(&pool).cloned().collect();
                if good.is_empty() {
                    return None; // this mod can't roll on the donor's base
                }
                targets.push(Target {
                    name: t.name.clone(),
                    side: t.side,
                    good,
                    groups: t.groups.clone(),
                    notable: t.notable,
                });
            }
        }
        let mut padding = BTreeMap::new();
        for (side, bins) in pads {
            for (i, ids) in bins.iter().enumerate() {
                let groups = ids
                    .iter()
                    .flat_map(|m| db.mods[m.as_str()].groups.iter().cloned())
                    .collect();
                targets.push(Target {
                    name: format!("NNN {} {}", side.name(), i + 1),
                    side: *side,
                    good: ids.clone(),
                    groups,
                    notable: false,
                });
            }
            padding.insert(side.name().to_string(), bins.len());
        }
        let mut names: Vec<String> = carried.iter().map(|w| w.name.clone()).collect();
        names.sort();
        let mut pad_key: Vec<(usize, Vec<Vec<String>>)> = pads
            .iter()
            .map(|(s, b)| {
                let mut bs: Vec<Vec<String>> =
                    b.iter().map(|x| x.iter().cloned().collect()).collect();
                bs.sort();
                (s.index(), bs)
            })
            .collect();
        pad_key.sort();
        let key = (bid.to_string(), names, pad_key);
        if let Some(&i) = self.keys.get(&key) {
            return Some(i);
        }
        let mut p = gear::problem_on(
            db,
            bid,
            targets,
            item_level,
            &BTreeSet::new(),
            Some((0, 0)),
            pricing,
        );
        let fillers = carried.iter().filter(|w| w.target.is_none()).count();
        if fillers > 0 {
            p.filler = self
                .filler
                .iter()
                .filter(|m| pool.contains(*m))
                .cloned()
                .collect();
            p.filler_need = fillers;
            p.reset_caches();
        }
        let donor = Donor {
            base: db.bases[bid].name.clone(),
            base_id: bid.to_string(),
            carries: carried.iter().map(|w| w.name.clone()).collect(),
            padding,
            problem: Some(p),
            plan: None,
            error: None,
            on_target,
        };
        self.donors.push(donor);
        self.keys.insert(key, self.donors.len() - 1);
        Some(self.donors.len() - 1)
    }

    /// (new wanted mods, pads) on `side` for the second input; its doubled mods count as pads.
    fn counts_of(&self, item: &Node, side: Side, dup: &[String]) -> (i64, i64) {
        let (wanted, pads) = item.counts(side, &self.side);
        let extra = dup.iter().filter(|n| self.side[n.as_str()] == side).count() as i64;
        (wanted - extra, pads + extra)
    }

    /// The cheapest tree that yields every wanted mod, once the leaves are solved.
    pub fn best(&self) -> Option<Step> {
        if !self.ok {
            return None;
        }
        let names: Vec<String> = self.want.iter().map(|w| w.name.clone()).collect();
        // options[S]: clean items carrying exactly S (target-base leaves, partner leaves, steps)
        let mut options: HashMap<BTreeSet<String>, Vec<Node>> = self
            .leaf_keys
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.iter()
                        .map(|&i| &self.donors[i])
                        .filter(|d| d.cost().is_some())
                        .map(|d| Node::Donor(Arc::new(d.leaf())))
                        .collect(),
                )
            })
            .collect();
        let mut best_step: HashMap<BTreeSet<String>, Arc<Step>> = HashMap::new();
        for size in 2..=names.len() {
            for combo in combinations(&names, size) {
                let full: BTreeSet<String> = combo.into_iter().collect();
                if SIDES
                    .iter()
                    .any(|s| full.iter().filter(|n| self.side[n.as_str()] == *s).count() > 3)
                {
                    continue;
                }
                let mut top: Option<Step> = None;
                let members: Vec<String> = full.iter().cloned().collect();
                for r in 1..size {
                    for left in combinations(&members, r) {
                        let s1: BTreeSet<String> = left.into_iter().collect();
                        let s2: BTreeSet<String> = full.difference(&s1).cloned().collect();
                        // the first input sits on the target base
                        let xs: Vec<&Node> = options
                            .get(&s1)
                            .into_iter()
                            .flatten()
                            .filter(|x| x.on_target())
                            .collect();
                        if xs.is_empty() {
                            continue;
                        }
                        let s1_sorted: Vec<String> = s1.iter().cloned().collect();
                        // The second input carries the rest, plus optionally copies of some of s1 (doubling).
                        for d in 0..if DOUBLING { s1.len() + 1 } else { 1 } {
                            for dup in combinations(&s1_sorted, d) {
                                let want_y: BTreeSet<String> =
                                    s2.iter().chain(dup.iter()).cloned().collect();
                                for y in options.get(&want_y).into_iter().flatten() {
                                    let cy: Counts = SIDES.map(|s| self.counts_of(y, s, &dup));
                                    for &x in &xs {
                                        let cx: Counts = SIDES.map(|s| x.counts(s, &self.side));
                                        let c = chance(&cx, &cy, y.on_target());
                                        if c <= 0.0 {
                                            continue;
                                        }
                                        // Doubled mods aren't uniform picks, so the exact-salvage odds don't hold: no credit.
                                        let back = if dup.is_empty() {
                                            (
                                                salvage(&cx, &cy, y.on_target()),
                                                if y.on_target() {
                                                    salvage(&cy, &cx, true)
                                                } else {
                                                    0.0
                                                },
                                            )
                                        } else {
                                            (0.0, 0.0)
                                        };
                                        let step = Step::new(
                                            x.clone(),
                                            y.clone(),
                                            c,
                                            back.0,
                                            back.1,
                                            self.chaos_per_hour,
                                        );
                                        let better = match (&top, step.cost) {
                                            (None, _) => true,
                                            (Some(t), Some(c)) => t.cost.is_none_or(|tc| c < tc),
                                            (Some(_), None) => false,
                                        };
                                        if better {
                                            top = Some(step);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if let Some(t) = top {
                    let t = Arc::new(t);
                    best_step.insert(full.clone(), t.clone());
                    options.entry(full).or_default().push(Node::Step(t));
                }
            }
        }
        best_step
            .get(&names.iter().cloned().collect::<BTreeSet<_>>())
            .map(|s| (**s).clone())
    }

    /// Solve the leaves in parallel (see [`solve_donors`]); plans are not cached here.
    pub fn solve(&mut self, pricing: Pricing) {
        solve_donors(self.leaves_mut(), pricing);
    }
}

/// Solve every donor that has a problem but no plan yet, in parallel; a failure is kept in its `error`.
///
/// Biggest problems go first so no core idles at the end. Plan caching belongs to the
/// pipeline: it can fill `plan` itself first and call this for the rest.
pub fn solve_donors(mut donors: Vec<&mut Donor>, pricing: Pricing) {
    donors.retain(|d| d.problem.is_some() && d.plan.is_none() && d.error.is_none());
    donors.sort_by_key(|d| std::cmp::Reverse(d.problem.as_ref().map_or(0, |p| p.targets.len())));
    donors.into_par_iter().for_each(|d| {
        match solve(
            d.problem.as_mut().expect("filtered"),
            pricing.prices,
            pricing.settings,
        ) {
            Ok(plan) => d.plan = Some(plan),
            Err(e) => d.error = Some(e.to_string()),
        }
    });
}

/// Solve the leaves (in parallel) and return the cheapest tree.
pub fn plan(
    db: &ModDB,
    problem: &Problem,
    base_id: &str,
    item_level: i64,
    pricing: Pricing,
) -> Option<Step> {
    let mut planner = Planner::new(db, problem, base_id, item_level, pricing);
    planner.solve(pricing);
    planner.best()
}

/// One draw of the recombinator model on explicit mod lists: None when the base flip loses,
/// else the mods kept (prefix list, suffix list per input; `legal`: ids that can roll on the target base).
pub fn draw<'a>(
    rng: &mut Rng,
    a: &[Vec<&'a str>; 2],
    b: &[Vec<&'a str>; 2],
    legal: &HashSet<&str>,
    same_base: bool,
) -> Option<HashSet<&'a str>> {
    if !same_base && rng.random() >= 0.5 {
        return None;
    }
    let mut kept: HashSet<&str> = HashSet::new();
    for side in 0..2 {
        let pool: Vec<&str> = a[side].iter().chain(&b[side]).copied().collect();
        let probs = row(pool.len());
        let total: f64 = probs.iter().map(|(_, p)| p).sum();
        let mut r = rng.random() * total;
        let mut k = probs[probs.len() - 1].0;
        for (n, p) in probs {
            if r < *p {
                k = *n;
                break;
            }
            r -= p;
        }
        let mut eligible: Vec<&str> = pool.into_iter().filter(|m| legal.contains(m)).collect();
        for i in 0..k.min(eligible.len()) {
            let j = i + (rng.random() * (eligible.len() - i) as f64) as usize;
            eligible.swap(i, j);
            kept.insert(eligible[i]);
        }
    }
    Some(kept)
}

/// Monte Carlo of the recombinator as modelled above, on explicit mod lists, to check `chance`.
///
/// `a`/`b`: (prefix, suffix) mod ids on each input; `legal`: ids that can roll on the target
/// base (the rest are NNN); success: the target base and every `wanted` id kept. Python's
/// generator is Mersenne Twister; this one is not, so the estimates agree statistically.
pub fn simulate(
    a: &[Vec<&str>; 2],
    b: &[Vec<&str>; 2],
    legal: &HashSet<&str>,
    same_base: bool,
    wanted: &HashSet<&str>,
    trials: usize,
    seed: u64,
) -> f64 {
    let mut rng = Rng::new(seed);
    let wins = (0..trials)
        .filter(|_| {
            draw(&mut rng, a, b, legal, same_base).is_some_and(|kept| wanted.is_subset(&kept))
        })
        .count();
    wins as f64 / trials as f64
}
