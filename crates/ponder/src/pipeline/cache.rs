use std::io::{Read, Write};
use std::path::PathBuf;

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use indexmap::IndexMap;
use ponder_core::{Bounds, JUNK_SLOTS, MAX_TARGETS, Plan, Prices, Problem, Settings, State};
use serde::{Deserialize, Serialize};

/// Bump when the solver's answers (or this file format) change, to drop cached plans.
pub const PLAN_VERSION: u32 = 10;

/// `float(f"{x:.{sig}g}")`: `x` to `sig` significant figures.
pub fn round_sig(x: f64, sig: usize) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{:.*e}", sig - 1, x).parse().unwrap_or(x)
}

/// A stable hash of a problem and the prices its actions use (to 3 significant figures).
pub fn plan_key(problem: &Problem, table: &Prices, settings: &Settings) -> String {
    let mut bytes = serde_json::to_vec(problem).expect("a problem serialises");
    let used: std::collections::BTreeSet<&str> = problem.actions.iter().flat_map(|a| a.cost.iter().map(|(pid, _)| pid.as_str())).collect();
    for pid in used {
        if let Some(p) = table.get(pid) {
            bytes.extend_from_slice(format!("|{pid}={}", round_sig(*p, 3)).as_bytes());
        }
    }
    bytes.extend_from_slice(format!("|{}|v{PLAN_VERSION}", settings.chaos_per_hour).as_bytes());
    blake3::hash(&bytes).to_hex().to_string()
}

/// Solved plans on disk. `disabled()` keeps none.
#[derive(Clone, Debug)]
pub struct PlanCache {
    dir: Option<PathBuf>,
}

impl Default for PlanCache {
    fn default() -> PlanCache {
        PlanCache::new(".cache/plans")
    }
}

impl PlanCache {
    pub fn new(dir: impl Into<PathBuf>) -> PlanCache {
        PlanCache { dir: Some(dir.into()) }
    }

    pub fn disabled() -> PlanCache {
        PlanCache { dir: None }
    }

    pub fn dir(&self) -> Option<&std::path::Path> {
        self.dir.as_deref()
    }

    /// The plan saved under `key`; `None` when missing, damaged or from another format.
    pub fn get(&self, key: &str) -> Option<Plan> {
        let bytes = std::fs::read(self.dir.as_ref()?.join(format!("{key}.plan"))).ok()?;
        let mut raw = Vec::new();
        ZlibDecoder::new(&bytes[..]).read_to_end(&mut raw).ok()?;
        decode(&raw)
    }

    /// Save a plan (atomically; a read-only or full disk only costs the cache).
    pub fn put(&self, key: &str, plan: &Plan) {
        let Some(dir) = &self.dir else { return };
        let Some(raw) = encode(plan) else { return };
        let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
        if z.write_all(&raw).is_err() {
            return;
        }
        let Ok(bytes) = z.finish() else { return };
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join(format!("{key}.{}.{:?}.tmp", std::process::id(), std::thread::current().id()));
        if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, dir.join(format!("{key}.plan"))).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

type CacheState = (u8, [u8; MAX_TARGETS], [u8; JUNK_SLOTS], u8);
type CacheRow = (CacheState, usize, f64, f64);

#[derive(Serialize, Deserialize)]
struct CachedPlan {
    cost: f64,
    hours: f64,
    states: usize,
    bounds: Bounds,
    unpriced: Vec<String>,
    usage: std::collections::BTreeMap<String, f64>,
    actions: Vec<String>,
    rows: Vec<CacheRow>,
}

fn encode(plan: &Plan) -> Option<Vec<u8>> {
    let n = plan.order.len();
    if plan.policy.len() != n || plan.visits.len() != n || plan.values.len() != n {
        return None;
    }
    let mut actions = IndexMap::new();
    let rows = plan
        .order
        .iter()
        .map(|s| {
            let name = plan.policy.get(s)?.as_str();
            let next = actions.len();
            let action = *actions.entry(name).or_insert(next);
            Some(((s.rarity, s.status, s.junk, s.meta), action, *plan.visits.get(s)?, *plan.values.get(s)?))
        })
        .collect::<Option<Vec<_>>>()?;
    let cached = CachedPlan {
        cost: plan.cost,
        hours: plan.hours,
        states: plan.states,
        bounds: plan.bounds?,
        unpriced: plan.unpriced.clone(),
        usage: plan.usage.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        actions: actions.into_keys().map(str::to_owned).collect(),
        rows,
    };
    postcard::to_stdvec(&(PLAN_VERSION, cached)).ok()
}

fn decode(raw: &[u8]) -> Option<Plan> {
    let (version, body): (u32, _) = postcard::take_from_bytes(raw).ok()?;
    if version != PLAN_VERSION {
        return None;
    }
    let (cached, rest): (CachedPlan, _) = postcard::take_from_bytes(body).ok()?;
    if !rest.is_empty() {
        return None;
    }
    let n = cached.rows.len();
    let mut plan = Plan {
        cost: cached.cost,
        hours: cached.hours,
        states: cached.states,
        bounds: Some(cached.bounds),
        unpriced: cached.unpriced,
        usage: cached.usage.into_iter().collect(),
        order: Vec::with_capacity(n),
        ..Plan::default()
    };
    plan.policy.reserve(n);
    plan.visits.reserve(n);
    plan.values.reserve(n);
    for ((rarity, status, junk, meta), action, visits, value) in cached.rows {
        let state = State { rarity, status, junk, meta };
        let name = cached.actions.get(action)?.clone();
        if plan.policy.insert(state, name).is_some() {
            return None;
        }
        plan.visits.insert(state, visits);
        plan.values.insert(state, value);
        plan.order.push(state);
    }
    Some(plan)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_to_significant_figures() {
        assert_eq!(round_sig(123456.0, 3), 123000.0);
        assert_eq!(round_sig(0.0123456, 3), 0.0123);
        assert_eq!(round_sig(0.0, 3), 0.0);
        assert_eq!(round_sig(99.96, 3), 100.0);
    }

    #[test]
    fn plans_survive_the_disk() {
        let s = State::new(2, &[1, 0, 2], &[1, 0, 0, 3]);
        let t = State::new(2, &[], &[]).with_meta(3);
        let bounds = Some(Bounds { lower: 11.0, upper: f64::INFINITY });
        let mut plan = Plan { cost: 12.5, hours: 0.25, states: 9, unpriced: vec!["x".into()], bounds, ..Plan::default() };
        plan.usage.insert("chaos".into(), 3.5);
        for (i, st) in [s, t].into_iter().enumerate() {
            plan.order.push(st);
            plan.policy.insert(st, ["chaos", "alt"][i].into());
            plan.visits.insert(st, 1.0 / 3.0);
            plan.values.insert(st, 7.0 + i as f64);
        }
        let raw = encode(&plan).unwrap();
        for end in 0..raw.len() {
            assert!(decode(&raw[..end]).is_none());
        }
        let mut trailing = raw.clone();
        trailing.push(0);
        assert!(decode(&trailing).is_none());
        let (version, mut cached): (u32, CachedPlan) = postcard::from_bytes(&raw).unwrap();
        assert!(decode(&postcard::to_stdvec(&(version - 1, &cached)).unwrap()).is_none());
        cached.rows[0].1 = cached.actions.len();
        assert!(decode(&postcard::to_stdvec(&(version, cached)).unwrap()).is_none());
        let dir = std::env::temp_dir().join(format!("ponder-plans-{}", std::process::id()));
        let cache = PlanCache::new(&dir);
        cache.put("abc", &plan);
        let back = cache.get("abc").expect("saved");
        assert_eq!((back.cost, back.hours, back.states, &back.unpriced, &back.usage), (12.5, 0.25, 9, &plan.unpriced, &plan.usage));
        assert_eq!(back.bounds, bounds);
        assert_eq!((back.order.clone(), &back.policy, &back.visits, &back.values), (plan.order.clone(), &plan.policy, &plan.visits, &plan.values));
        assert!(cache.get("missing").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
