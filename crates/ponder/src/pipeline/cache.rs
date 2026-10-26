use std::io::{Read, Write};
use std::path::PathBuf;

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use indexmap::IndexMap;
use ponder_core::{JUNK_SLOTS, MAX_TARGETS, Plan, Prices, Problem, SelfFracture, Settings, SolveError, State, StateKind};
use serde::{Deserialize, Serialize};

/// Bump when the solver's answers (or this file format) change, to drop cached plans.
pub const PLAN_VERSION: u32 = 22;

/// `float(f"{x:.{sig}g}")`: `x` to `sig` significant figures.
pub fn round_sig(x: f64, sig: usize) -> f64 {
    if x == 0.0 || !x.is_finite() {
        return x;
    }
    format!("{:.*e}", sig - 1, x).parse().unwrap_or(x)
}

/// A stable hash of a problem and the prices its actions use (to 3 significant figures).
pub fn plan_key(problem: &Problem, table: &Prices, settings: &Settings) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(problem.fingerprint().as_bytes());
    let used: std::collections::BTreeSet<&str> = problem.actions.iter().flat_map(|a| a.cost.iter().map(|(pid, _)| pid.as_str())).collect();
    for pid in used {
        if let Some(p) = table.get(pid) {
            hasher.update(format!("|{pid}={}", round_sig(*p, 3)).as_bytes());
        }
    }
    hasher.update(format!("|{}|v{PLAN_VERSION}", settings.chaos_per_hour).as_bytes());
    hasher.finalize().to_hex().to_string()
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

    pub fn failure(&self, key: &str) -> Option<SolveError> {
        serde_json::from_slice(&std::fs::read(self.dir.as_ref()?.join(format!("{key}.failure"))).ok()?).ok()
    }

    pub fn put_failure(&self, key: &str, error: &SolveError) {
        if matches!(error, SolveError::Unreachable | SolveError::ResourceLimit { .. })
            && let Ok(raw) = serde_json::to_vec(error)
        {
            self.store(&format!("{key}.failure"), &raw);
        }
    }

    pub fn get_or_solve(&self, key: &str, solve: impl FnOnce() -> Result<Plan, SolveError>) -> Result<Plan, SolveError> {
        if let Some(plan) = self.get(key) {
            return Ok(plan);
        }
        if let Some(error) = self.failure(key) {
            return Err(error);
        }
        let result = solve();
        match &result {
            Ok(plan) => self.put(key, plan),
            Err(error) => self.put_failure(key, error),
        }
        result
    }

    pub fn self_fractures(&self, problem: &Problem, table: &Prices, settings: &Settings) -> Vec<Option<SelfFracture>> {
        let used: std::collections::BTreeSet<&str> = problem
            .actions
            .iter()
            .flat_map(|a| a.cost.iter().map(|(pid, _)| pid.as_str()))
            .chain(std::iter::once(ponder_core::FRACTURE_ORB))
            .collect();
        let prices: Vec<_> = used.into_iter().map(|pid| (pid, table.get(pid))).collect();
        let raw = serde_json::to_vec(&(PLAN_VERSION, problem.fingerprint().as_bytes(), prices, settings.chaos_per_hour))
            .expect("fracture cache key serialises");
        let name = format!("{}.fractures", blake3::hash(&raw).to_hex());
        if let Some(cached) = self
            .dir
            .as_ref()
            .and_then(|dir| std::fs::read(dir.join(&name)).ok())
            .and_then(|raw| postcard::from_bytes::<Vec<Option<SelfFracture>>>(&raw).ok())
            .filter(|cached| cached.len() == problem.targets.len())
        {
            return cached;
        }
        let made: Vec<_> = (0..problem.targets.len()).map(|i| ponder_core::self_fracture(problem, i, table, settings)).collect();
        if let Ok(bytes) = postcard::to_stdvec(&made) {
            self.store(&name, &bytes);
        }
        made
    }

    /// Save a plan (atomically; a read-only or full disk only costs the cache).
    pub fn put(&self, key: &str, plan: &Plan) {
        if self.dir.is_none() {
            return;
        }
        let Some(raw) = encode(plan) else { return };
        let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
        if z.write_all(&raw).is_err() {
            return;
        }
        let Ok(bytes) = z.finish() else { return };
        self.store(&format!("{key}.plan"), &bytes);
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_file(dir.join(format!("{key}.failure")));
        }
    }

    fn store(&self, name: &str, bytes: &[u8]) {
        let Some(dir) = &self.dir else { return };
        let _ = std::fs::create_dir_all(dir);
        let tmp = dir.join(format!("{name}.{}.{:?}.tmp", std::process::id(), std::thread::current().id()));
        if std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, dir.join(name)).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

type CacheState = (StateKind, u8, [u8; MAX_TARGETS], [u8; JUNK_SLOTS], u8, u8, u32);
type CacheRow = (CacheState, usize, f64, f64);

fn pack_state(s: State) -> CacheState {
    (s.kind, s.rarity, s.status, s.junk, s.meta, s.veiled, s.unveil)
}

fn unpack_state((kind, rarity, status, junk, meta, veiled, unveil): CacheState) -> State {
    State { kind, rarity, status, junk, meta, veiled, unveil }
}

#[derive(Serialize, Deserialize)]
struct CachedPlan {
    search_work: ponder_core::search::Work,
    risk: Option<ponder_core::PolicyRisk>,
    initial: Vec<(f64, CacheState)>,
    recovery: Vec<(f64, CacheState)>,
    restarts: f64,
    sales: f64,
    sale_proceeds: f64,
    sell_states: Vec<(CacheState, f64)>,
    cost: f64,
    hours: f64,
    states: usize,
    approximate: bool,
    unpriced: Vec<String>,
    usage: std::collections::BTreeMap<String, f64>,
    actions: Vec<String>,
    rows: Vec<CacheRow>,
    restart_values: Vec<(CacheState, f64)>,
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
            let name = plan.policy.get(s)?.as_ref();
            let next = actions.len();
            let action = *actions.entry(name).or_insert(next);
            Some((pack_state(*s), action, *plan.visits.get(s)?, *plan.values.get(s)?))
        })
        .collect::<Option<Vec<_>>>()?;
    let cached = CachedPlan {
        search_work: plan.search_work,
        risk: plan.risk,
        initial: plan.initial.iter().map(|&(p, s)| (p, pack_state(s))).collect(),
        recovery: plan.recovery.iter().map(|&(p, s)| (p, pack_state(s))).collect(),
        restarts: plan.restarts,
        sales: plan.sales,
        sale_proceeds: plan.sale_proceeds,
        sell_states: plan.sell_states.iter().map(|(&s, &v)| (pack_state(s), v)).collect(),
        cost: plan.cost,
        hours: plan.hours,
        states: plan.states,
        approximate: plan.approximate,
        unpriced: plan.unpriced.clone(),
        usage: plan.usage.iter().map(|(k, v)| (k.clone(), *v)).collect(),
        actions: actions.into_keys().map(str::to_owned).collect(),
        rows,
        restart_values: plan.restart_values.iter().map(|(&s, &v)| (pack_state(s), v)).collect(),
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
        search_work: cached.search_work,
        risk: cached.risk,
        initial: cached.initial.into_iter().map(|(p, s)| (p, unpack_state(s))).collect(),
        recovery: cached.recovery.into_iter().map(|(p, s)| (p, unpack_state(s))).collect(),
        restarts: cached.restarts,
        sales: cached.sales,
        sale_proceeds: cached.sale_proceeds,
        sell_states: cached.sell_states.into_iter().map(|(s, v)| (unpack_state(s), v)).collect(),
        cost: cached.cost,
        hours: cached.hours,
        states: cached.states,
        approximate: cached.approximate,
        unpriced: cached.unpriced,
        usage: cached.usage.into_iter().collect(),
        restart_values: cached.restart_values.into_iter().map(|(s, v)| (unpack_state(s), v)).collect(),
        order: Vec::with_capacity(n),
        ..Plan::default()
    };
    plan.policy.reserve(n);
    plan.visits.reserve(n);
    plan.values.reserve(n);
    let actions: Vec<std::sync::Arc<str>> = cached.actions.into_iter().map(std::sync::Arc::from).collect();
    for (state, action, visits, value) in cached.rows {
        let state = unpack_state(state);
        let name = actions.get(action)?.clone();
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
        let u = State::new(1, &[0, 1], &[]);
        let mut plan = Plan { cost: 12.5, hours: 0.25, states: 9, unpriced: vec!["x".into()], approximate: true, ..Plan::default() };
        plan.risk = Some(ponder_core::PolicyRisk { currency: ponder_core::Moments { mean: 12.5, variance: 7.0 }, ..Default::default() });
        plan.usage.insert("chaos".into(), 3.5);
        plan.initial = vec![(0.25, s), (0.75, t)];
        plan.recovery = vec![(1.0, s)];
        plan.restarts = 0.5;
        plan.sales = 0.25;
        plan.sale_proceeds = 30.0;
        plan.restart_values.insert(State::new(2, &[], &[0, 3, 0, 3]), 40.0);
        plan.sell_states = plan.restart_values.clone();
        for (i, st) in [s, t, u].into_iter().enumerate() {
            plan.order.push(st);
            plan.policy.insert(st, ["chaos", "alt", "chaos"][i].into());
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
        cache.put_failure("abc", &SolveError::Unreachable);
        assert_eq!(cache.failure("abc"), Some(SolveError::Unreachable));
        cache.put_failure("numerical", &SolveError::Singular("retry".into()));
        assert!(cache.failure("numerical").is_none());
        cache.put("abc", &plan);
        assert!(cache.failure("abc").is_none());
        let back = cache.get("abc").expect("saved");
        assert_eq!((back.cost, back.hours, back.states, &back.unpriced, &back.usage), (12.5, 0.25, 9, &plan.unpriced, &plan.usage));
        assert!(back.approximate);
        assert_eq!(back.risk, plan.risk);
        assert!(std::sync::Arc::ptr_eq(&back.policy[&s], &back.policy[&u]));
        assert_eq!(back.restart_values, plan.restart_values);
        assert_eq!(back.sell_states, plan.sell_states);
        assert_eq!((back.sales, back.sale_proceeds), (plan.sales, plan.sale_proceeds));
        assert_eq!(back.initial, plan.initial);
        assert_eq!((&back.recovery, back.restarts), (&plan.recovery, plan.restarts));
        assert_eq!(
            (back.order.clone(), &back.policy, &back.visits, &back.values),
            (plan.order.clone(), &plan.policy, &plan.visits, &plan.values)
        );
        assert!(cache.get("missing").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn fracture_prices_reuse_results_and_track_currency_and_time_changes() {
        use ponder_core::{Action, ActionKind, Kind, Mod, ProblemSpec, RARE, Side, Target};
        let pool: Vec<_> = [Side::Prefix, Side::Prefix, Side::Suffix, Side::Suffix]
            .into_iter()
            .enumerate()
            .map(|(i, side)| Mod::new(&i.to_string(), side, 1.0, &[&i.to_string()], false, "", &[]))
            .collect();
        let target =
            Target { name: "wanted".into(), side: Side::Prefix, good: ["0".into()].into(), groups: ["0".into()].into(), notable: false };
        let action = Action::new("chaos", ActionKind::Reforge, &[("chaos", 1.0)], &[RARE]);
        let problem = ProblemSpec::new(pool, vec![target], 1, vec![action], Kind::Gear).compile();
        let dir = std::env::temp_dir().join(format!("ponder-fractures-{}", std::process::id()));
        let cache = PlanCache::new(&dir);
        let settings = Settings { chaos_per_hour: 0.0 };
        let mut prices: Prices = [("chaos".into(), 1.0), ("fracturing-orb".into(), 100.0)].into();
        let first = cache.self_fractures(&problem, &prices, &settings);
        assert_eq!(first, PlanCache::disabled().self_fractures(&problem, &prices, &settings));
        assert_eq!(first, cache.self_fractures(&problem.fresh_transition_cache(), &prices, &settings));
        prices.insert("fracturing-orb".into(), 200.0);
        let expensive = cache.self_fractures(&problem, &prices, &settings);
        assert!(expensive[0].as_ref().unwrap().cost > first[0].as_ref().unwrap().cost);
        let timed = cache.self_fractures(&problem, &prices, &Settings { chaos_per_hour: 3600.0 });
        assert!(timed[0].as_ref().unwrap().cost > expensive[0].as_ref().unwrap().cost);
        prices.remove("fracturing-orb");
        assert_eq!(cache.self_fractures(&problem, &prices, &settings), vec![None]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
