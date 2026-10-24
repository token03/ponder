//! The plan cache: solved plans on disk, keyed by problem and prices.
//!
//! Plans live in `.cache/plans` as a small binary layout, zlib-compressed.
//! The key is a sha1 of the problem's canonical JSON (struct fields in declaration order,
//! sets and maps sorted, so it is the same across runs), the prices of the actions it uses
//! rounded to 3 significant figures, the clicking rate and `PLAN_VERSION`.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use ponder_core::{Plan, Prices, Problem, Settings, State};
use ponder_net::pyjson::sha1_hex;

/// Bump when the solver's answers (or this file format) change, to drop cached plans.
pub const PLAN_VERSION: u32 = 1;

const MAGIC: &[u8; 4] = b"PLN1";

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
    sha1_hex(&bytes)
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

// -- the binary layout ----------------------------------------------------------------------

fn put_u32(out: &mut Vec<u8>, n: usize) {
    out.extend_from_slice(&(n as u32).to_le_bytes());
}

fn put_f64(out: &mut Vec<u8>, x: f64) {
    out.extend_from_slice(&x.to_le_bytes());
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len());
    out.extend_from_slice(s.as_bytes());
}

/// `None` when the plan's maps don't all cover exactly the states in `order` (nothing to cache then).
fn encode(plan: &Plan) -> Option<Vec<u8>> {
    let n = plan.order.len();
    if plan.policy.len() != n || plan.visits.len() != n || plan.values.len() != n {
        return None;
    }
    let mut names: Vec<&str> = Vec::new();
    let mut at: HashMap<&str, usize> = HashMap::new();
    let mut policy = Vec::with_capacity(n);
    for s in &plan.order {
        let a = plan.policy.get(s)?.as_str();
        let i = *at.entry(a).or_insert_with(|| {
            names.push(a);
            names.len() - 1
        });
        policy.push(i);
    }
    let mut out = Vec::with_capacity(64 + n * 36);
    out.extend_from_slice(MAGIC);
    put_f64(&mut out, plan.cost);
    put_f64(&mut out, plan.hours);
    out.extend_from_slice(&(plan.states as u64).to_le_bytes());
    put_u32(&mut out, plan.unpriced.len());
    for u in &plan.unpriced {
        put_str(&mut out, u);
    }
    let mut usage: Vec<(&String, &f64)> = plan.usage.iter().collect();
    usage.sort_by(|a, b| a.0.cmp(b.0));
    put_u32(&mut out, usage.len());
    for (k, v) in usage {
        put_str(&mut out, k);
        put_f64(&mut out, *v);
    }
    put_u32(&mut out, names.len());
    for a in &names {
        put_str(&mut out, a);
    }
    put_u32(&mut out, n);
    for (s, p) in plan.order.iter().zip(&policy) {
        out.push(s.rarity);
        out.extend_from_slice(&s.status);
        out.extend_from_slice(&s.junk);
        put_u32(&mut out, *p);
        put_f64(&mut out, *plan.visits.get(s)?);
        put_f64(&mut out, *plan.values.get(s)?);
    }
    Some(out)
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u32(&mut self) -> Option<usize> {
        Some(u32::from_le_bytes(self.take(4)?.try_into().ok()?) as usize)
    }

    fn f64(&mut self) -> Option<f64> {
        Some(f64::from_le_bytes(self.take(8)?.try_into().ok()?))
    }

    fn string(&mut self) -> Option<String> {
        let n = self.u32()?;
        String::from_utf8(self.take(n)?.to_vec()).ok()
    }
}

fn decode(raw: &[u8]) -> Option<Plan> {
    let mut r = Reader(raw);
    if r.take(4)? != MAGIC {
        return None;
    }
    let cost = r.f64()?;
    let hours = r.f64()?;
    let states = u64::from_le_bytes(r.take(8)?.try_into().ok()?) as usize;
    let unpriced = (0..r.u32()?).map(|_| r.string()).collect::<Option<Vec<_>>>()?;
    let n_usage = r.u32()?;
    let mut usage = HashMap::with_capacity(n_usage);
    for _ in 0..n_usage {
        let k = r.string()?;
        usage.insert(k, r.f64()?);
    }
    let names = (0..r.u32()?).map(|_| r.string()).collect::<Option<Vec<_>>>()?;
    let n = r.u32()?;
    let mut plan = Plan { cost, hours, states, unpriced, usage, ..Plan::default() };
    plan.order.reserve(n);
    for _ in 0..n {
        let b = r.take(15)?;
        let s = State::new(b[0], &b[1..9], &b[9..15]);
        let name = names.get(r.u32()?)?.clone();
        plan.visits.insert(s, r.f64()?);
        plan.values.insert(s, r.f64()?);
        plan.policy.insert(s, name);
        plan.order.push(s);
    }
    r.0.is_empty().then_some(plan)
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
        let t = State::new(1, &[], &[]);
        let mut plan = Plan { cost: 12.5, hours: 0.25, states: 9, unpriced: vec!["x".into()], ..Plan::default() };
        plan.usage.insert("chaos".into(), 3.5);
        for (i, st) in [s, t].into_iter().enumerate() {
            plan.order.push(st);
            plan.policy.insert(st, ["chaos", "alt"][i].into());
            plan.visits.insert(st, 1.0 / 3.0);
            plan.values.insert(st, 7.0 + i as f64);
        }
        let dir = std::env::temp_dir().join(format!("ponder-plans-{}", std::process::id()));
        let cache = PlanCache::new(&dir);
        cache.put("abc", &plan);
        let back = cache.get("abc").expect("saved");
        assert_eq!((back.cost, back.hours, back.states, &back.unpriced, &back.usage), (12.5, 0.25, 9, &plan.unpriced, &plan.usage));
        assert_eq!((back.order.clone(), &back.policy, &back.visits, &back.values), (plan.order.clone(), &plan.policy, &plan.visits, &plan.values));
        assert!(cache.get("missing").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
