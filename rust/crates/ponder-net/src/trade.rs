//! Path of Exile trade site: build searches, and run them when allowed.
//!
//! The site's API (the one Awakened PoE Trade uses) refuses some networks and
//! wants a logged-in session: put `POESESSID=...` in a git-ignored `.env` in the
//! directory you run ponder from (or export it). `search_url` always works in a
//! browser. The cookie is only ever put in a request header: it is never logged,
//! printed, or included in an error.
//!
//! The API allows only a few dozen searches per five minutes. Every response
//! says how much of each limit is used (X-Rate-Limit-* headers), so calls wait
//! just long enough to stay under them, and answers are cached on disk for
//! `ttl` seconds so reruns don't search again.
//!
//! # Wiring the repoe lookups
//!
//! `stat_id` / `stat_min` need repoe's `mods.json` and `stat_translations.json`.
//! This crate does not depend on ponder-game, so they work on the raw JSON
//! (`serde_json::Value`, exactly as `data/*.json` parse) or on anything that
//! implements `StatIds`. The pipeline builds one `Repoe::new(&mods, &stat_translations)`
//! from the data ponder-game already loaded (it keeps the raw `Value`s, or
//! re-reads the two files) and passes it to `cluster_query`. A ponder-game
//! `ModDB` can instead implement `StatIds` itself, as one method.

use crate::env::{self, Clock};
use crate::http::{self, Headers, Method, Request, Transport};
use crate::pyjson::{self, quote, truthy};
use anyhow::{Result, bail};
use serde::Deserialize;
use serde_json::{Number, Value, json};
use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SITE: &str = "https://www.pathofexile.com";
pub const USER_AGENT: &str = "ponder/0.1 (crafting cost research)";
/// Added Small Passive Skills grant: #
pub const ENCHANT_STAT: &str = "enchant.stat_3948993189";
/// Adds # Passive Skills
pub const PASSIVES_STAT: &str = "enchant.stat_3086156145";
pub const CACHE: &str = ".cache/trade";

/// Trade categories for item classes; groups of several bases search by category.
#[rustfmt::skip]
pub const CATEGORIES: &[(&str, &str)] = &[
    ("Ring", "accessory.ring"), ("Amulet", "accessory.amulet"), ("Belt", "accessory.belt"),
    ("Gloves", "armour.gloves"), ("Boots", "armour.boots"), ("Helmet", "armour.helmet"),
    ("Body Armour", "armour.chest"), ("Shield", "armour.shield"), ("Quiver", "armour.quiver"),
    ("Jewel", "jewel.base"), ("AbyssJewel", "jewel.abyss"),
    ("Wand", "weapon.wand"), ("Sceptre", "weapon.sceptre"), ("Dagger", "weapon.dagger"),
    ("Rune Dagger", "weapon.runedagger"), ("Claw", "weapon.claw"), ("Bow", "weapon.bow"),
    ("Staff", "weapon.staff"), ("Warstaff", "weapon.warstaff"),
    ("One Hand Sword", "weapon.onesword"), ("Thrusting One Hand Sword", "weapon.onesword"),
    ("Two Hand Sword", "weapon.twosword"), ("One Hand Axe", "weapon.oneaxe"), ("Two Hand Axe", "weapon.twoaxe"),
    ("One Hand Mace", "weapon.onemace"), ("Two Hand Mace", "weapon.twomace"),
];

/// The trade category of an item class (`CATEGORIES[item_class]`).
pub fn category(item_class: &str) -> Option<&'static str> {
    CATEGORIES.iter().find(|(c, _)| *c == item_class).map(|(_, t)| *t)
}

/// Chaos value per currency id (poe.ninja's ids).
pub type Rates = HashMap<String, f64>;

/// A wanted stat: trade stat id and the minimum value (`None`: any roll).
/// `Number` keeps Python's int-versus-float distinction in the query text.
pub type Stat = (String, Option<Number>);

/// `n` as a JSON number the way Python would have it: an int when it is whole.
pub fn num(n: f64) -> Number {
    if n.fract() == 0.0 && n.abs() < 9e15 { Number::from(n as i64) } else { Number::from_f64(n).expect("finite") }
}

// -- repoe lookups ----------------------------------------------------------------------

/// Looks up the trade stat id of a mod, from repoe's stat translations.
pub trait StatIds {
    /// Trade stat id for a mod's (first) stat of this `kind` ("explicit", "fractured", ...).
    fn stat_id(&self, mod_id: &str, kind: &str) -> Option<String>;
}

/// repoe's `stat_translations.json`, indexed by stat id.
#[derive(Clone, Debug, Default)]
pub struct Translations(HashMap<String, Vec<Value>>);

impl Translations {
    pub fn new(stat_translations: &Value) -> Translations {
        let mut index: HashMap<String, Vec<Value>> = HashMap::new();
        for entry in stat_translations.as_array().into_iter().flatten() {
            for sid in entry["ids"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                index.entry(sid.to_string()).or_default().push(entry.clone());
            }
        }
        Translations(index)
    }
}

/// `stat_id(mod_id, kind)`: trade stat id for a mod's (first) stat, from repoe's stat translations.
pub fn stat_id(mods: &Value, translations: &Translations, mod_id: &str, kind: &str) -> Option<String> {
    let first = mods.get(mod_id)?.get("stats")?.as_array()?.first()?.get("id")?.as_str()?;
    for entry in translations.0.get(first)? {
        for t in entry.get("trade_stats").and_then(Value::as_array).into_iter().flatten() {
            if t["type"] == kind {
                return t["id"].as_str().map(str::to_string);
            }
        }
    }
    None
}

/// `stat_min(mod_ids)`: lowest roll of the first stat across `mod_ids`: the value a search must ask for.
pub fn stat_min(mods: &Value, mod_ids: &[&str]) -> Option<Number> {
    let lows = mod_ids.iter().filter_map(|m| match mods.get(*m)?.get("stats")?.as_array()?.first()?.get("min")? {
        Value::Number(n) => Some(n.clone()),
        _ => None,
    });
    lows.fold(None, |best: Option<Number>, n| match best {
        Some(b) if b.as_f64() <= n.as_f64() => Some(b),
        _ => Some(n),
    })
}

/// repoe's `mods.json` plus `stat_translations.json`: what `cluster_query` needs.
pub struct Repoe<'a> {
    pub mods: &'a Value,
    pub translations: Translations,
}

impl<'a> Repoe<'a> {
    pub fn new(mods: &'a Value, stat_translations: &Value) -> Repoe<'a> {
        Repoe { mods, translations: Translations::new(stat_translations) }
    }
}

impl StatIds for Repoe<'_> {
    fn stat_id(&self, mod_id: &str, kind: &str) -> Option<String> {
        stat_id(self.mods, &self.translations, mod_id, kind)
    }
}

// -- queries ------------------------------------------------------------------------------

/// `item_query`'s keyword arguments.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct ItemQuery {
    pub base: Option<String>,
    pub category: Option<String>,
    pub stats: Vec<Stat>,
    pub item_level: Option<i64>,
    pub fractured_only: bool,
    pub extra: Vec<Value>,
    /// at least n of these other stats
    pub count: Option<(i64, Vec<Stat>)>,
    pub rarity: String,
    pub corrupted: Option<bool>,
}

impl Default for ItemQuery {
    fn default() -> Self {
        ItemQuery {
            base: None,
            category: None,
            stats: Vec::new(),
            item_level: None,
            fractured_only: false,
            extra: Vec::new(),
            count: None,
            rarity: "nonunique".into(),
            corrupted: None,
        }
    }
}

/// A search for non-unique items with all of `q.stats` (trade stat id, minimum value),
/// and, with `q.count = (n, stats)`, at least n of those other stats.
pub fn item_query(q: &ItemQuery) -> Value {
    let mut filters: Vec<Value> = q.stats.iter().map(|(sid, low)| stat_filter(sid, low)).collect();
    filters.extend(q.extra.iter().cloned());
    let mut type_filters = json!({"rarity": {"option": q.rarity}});
    if let Some(c) = q.category.as_deref().filter(|c| !c.is_empty()) {
        type_filters["category"] = json!({"option": c});
    }
    let mut query = json!({
        "status": {"option": "available"},
        "stats": [{"type": "and", "filters": filters}],
        "filters": {"type_filters": {"filters": type_filters}},
    });
    if let Some((n, stats)) = q.count.as_ref().filter(|(n, stats)| *n != 0 && !stats.is_empty()) {
        let group = json!({"type": "count", "value": {"min": n}, "filters": stats.iter().map(|(sid, low)| stat_filter(sid, low)).collect::<Vec<_>>()});
        query["stats"].as_array_mut().unwrap().push(group);
    }
    if let Some(b) = q.base.as_deref().filter(|b| !b.is_empty()) {
        query["type"] = json!(b);
    }
    let mut misc = serde_json::Map::new();
    if let Some(l) = q.item_level.filter(|l| *l != 0) {
        misc.insert("ilvl".into(), json!({"min": l}));
    }
    if q.fractured_only {
        misc.insert("fractured_item".into(), json!({"option": "true"}));
    }
    if let Some(c) = q.corrupted {
        misc.insert("corrupted".into(), json!({"option": c.to_string()}));
    }
    if !misc.is_empty() {
        query["filters"]["misc_filters"] = json!({"filters": misc});
    }
    json!({"query": query, "sort": {"price": "asc"}})
}

fn stat_filter(sid: &str, low: &Option<Number>) -> Value {
    json!({"id": sid, "value": match low { Some(n) => json!({"min": n}), None => json!({}) }, "disabled": false})
}

/// Python's `str.title()`: each run of letters capitalised.
fn title(s: &str) -> String {
    let mut out = String::new();
    let mut in_word = false;
    for c in s.chars() {
        if c.is_alphabetic() {
            if in_word { out.extend(c.to_lowercase()) } else { out.extend(c.to_uppercase()) }
            in_word = true;
        } else {
            out.push(c);
            in_word = false;
        }
    }
    out
}

/// A cluster jewel search. `notable_ids`: mod ids with an optional minimum roll (for tiered small-passive mods).
pub fn cluster_query(
    ids: &dyn StatIds,
    size: &str,
    enchant_option: Option<&str>,
    passives: Option<i64>,
    notable_ids: &[(String, Option<Number>)],
    item_level: Option<i64>,
    fractured_ids: &[String],
) -> Value {
    let mut extra = Vec::new();
    if let Some(o) = enchant_option.filter(|o| !o.is_empty()) {
        extra.push(json!({"id": ENCHANT_STAT, "value": {"option": o}, "disabled": false}));
    }
    if let Some(p) = passives.filter(|p| *p != 0) {
        extra.push(json!({"id": PASSIVES_STAT, "value": {"min": p, "max": p}, "disabled": false}));
    }
    let mut stats: Vec<Stat> = notable_ids.iter().map(|(m, low)| (ids.stat_id(m, "explicit").unwrap_or_else(|| m.clone()), low.clone())).collect();
    stats.extend(fractured_ids.iter().map(|m| (ids.stat_id(m, "fractured").unwrap_or_else(|| m.clone()), None)));
    item_query(&ItemQuery {
        base: Some(format!("{} Cluster Jewel", title(size))),
        stats,
        item_level,
        fractured_only: !fractured_ids.is_empty(),
        extra,
        ..ItemQuery::default()
    })
}

/// The trade site's search page for a query (the `q=` JSON is compact, like `json.dumps(.., separators=(",", ":"))`).
pub fn search_url(league: &str, query: &Value) -> String {
    let q = quote(&pyjson::dumps_compact(&json!({"query": query["query"]})));
    format!("{SITE}/trade/search/{}?q={q}", quote(league))
}

pub fn chaos_price(listing: &Value, rates: &Rates) -> Option<f64> {
    let p = &listing["listing"]["price"];
    let amount = p.get("amount").filter(|a| truthy(a))?.as_f64()?;
    let rate = *rates.get(currency_id(p.get("currency").and_then(Value::as_str).unwrap_or("")))?;
    (rate != 0.0).then_some(amount * rate)
}

/// Asks are cached in their own currency, so a divine-rate change reprices them.
fn priced(asks: &[(Number, String)], total: i64, rates: &Rates) -> Option<(f64, i64)> {
    let mut chaos: Vec<f64> = asks.iter().filter_map(|(amount, cur)| Some(amount.as_f64()? * rates.get(currency_id(cur))?)).collect();
    if chaos.is_empty() {
        return None;
    }
    chaos.sort_by(f64::total_cmp);
    let n = chaos.len();
    let median = if n % 2 == 1 { chaos[n / 2] } else { (chaos[n / 2 - 1] + chaos[n / 2]) / 2.0 }; // statistics.median
    Some((median, total))
}

fn currency_id(trade_currency: &str) -> &str {
    trade_currency // exalted, divine, chaos, alch, alt map to themselves; so does everything else
}

// -- listings as items --------------------------------------------------------------------

/// A mod line of a listed item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedLine {
    pub text: String,
    pub fractured: bool,
    pub crafted: bool,
}

/// A trade listing's item, plain: the pipeline turns it into its `Item` (for `ModDB.match`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListingItem {
    /// "NORMAL", "MAGIC", "RARE", "UNIQUE" (or the site's own word, upper-cased)
    pub rarity: String,
    pub name: String,
    pub base: String,
    pub ilvl: Option<i64>,
    pub influences: BTreeSet<String>,
    /// any of "corrupted", "fractured", "mirrored", "split", in that order
    pub flags: Vec<String>,
    pub implicits: Vec<String>,
    /// fractured lines, then explicit, then crafted
    pub explicits: Vec<ListedLine>,
}

/// A trade listing's item, its mod lines marked fractured or crafted.
pub fn listing_item(listing: &Value) -> ListingItem {
    let empty = json!({});
    let it = listing.get("item").filter(|i| truthy(i)).unwrap_or(&empty);
    let str_of = |k: &str| it.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let frame = ["NORMAL", "MAGIC", "RARE", "UNIQUE"].get(it.get("frameType").and_then(Value::as_u64).unwrap_or(99) as usize).copied().unwrap_or("");
    let influences = match it.get("influences") {
        Some(Value::Object(m)) => m.keys().cloned().collect(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        _ => BTreeSet::new(),
    };
    let flags = ["corrupted", "fractured", "mirrored", "split"].iter().filter(|k| it.get(**k).is_some_and(truthy)).map(|k| k.to_string()).collect();
    let lines = |key: &str| -> Vec<String> { it.get(key).and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect() };
    let mut explicits = Vec::new();
    for (key, fractured, crafted) in [("fracturedMods", true, false), ("explicitMods", false, false), ("craftedMods", false, true)] {
        explicits.extend(lines(key).into_iter().map(|text| ListedLine { text, fractured, crafted }));
    }
    ListingItem {
        rarity: str_of("rarity").unwrap_or(frame).to_uppercase(),
        name: str_of("name").unwrap_or("").to_string(),
        base: str_of("baseType").or_else(|| str_of("typeLine")).unwrap_or("").to_string(),
        ilvl: it.get("ilvl").and_then(Value::as_i64),
        influences,
        flags,
        implicits: lines("implicitMods"),
        explicits,
    }
}

/// POESESSID from the environment, else from `.env` in the working directory. Never log the result.
pub fn cookie() -> Option<String> {
    env::env("POESESSID")
}

// -- the client: rate limits, cache -------------------------------------------------------

/// (max hits, period s, hits now, restricted s)
type Rule = (i64, i64, i64, i64);

/// The trade site, with its disk cache and the rate-limit state of this run.
pub struct Trade {
    pub http: Arc<dyn Transport>,
    pub cache: PathBuf,
    /// seconds an answer stays fresh (`CACHE_TTL`: env `PONDER_TRADE_TTL`, default 3600)
    pub ttl: f64,
    pub clock: Clock,
    /// told how long a rate-limit wait will be
    pub on_wait: Option<Arc<dyn Fn(f64) + Send + Sync>>,
    /// where the session cookie comes from; replaceable so tests need no `.env`
    pub cookie: Arc<dyn Fn() -> Option<String> + Send + Sync>,
    /// endpoint ("search"/"fetch") -> (when seen, rules)
    limits: Mutex<HashMap<String, (f64, Vec<Rule>)>>,
    searches: AtomicUsize,
}

impl Default for Trade {
    fn default() -> Self {
        Trade::new()
    }
}

impl Trade {
    pub fn new() -> Trade {
        let ttl = std::env::var("PONDER_TRADE_TTL").ok().and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(3600);
        let mut t = Trade::with(http::real(), CACHE.into(), Clock::real());
        t.ttl = ttl as f64;
        t
    }

    pub fn with(http: Arc<dyn Transport>, cache: PathBuf, clock: Clock) -> Trade {
        Trade { http, cache, ttl: 3600.0, clock, on_wait: None, cookie: Arc::new(cookie), limits: Mutex::new(HashMap::new()), searches: AtomicUsize::new(0) }
    }

    /// Searches sent to the site this run (cached answers don't count).
    pub fn searches(&self) -> usize {
        self.searches.load(Ordering::SeqCst)
    }

    /// Median asking price (chaos) of the cheapest `listings`, and the total found.
    ///
    /// Answers younger than `max_age` seconds (default `ttl`) come from the disk cache.
    pub fn price(&self, league: &str, query: &Value, rates: &Rates, listings: usize, max_age: Option<f64>) -> Result<Option<(f64, i64)>> {
        let key = pyjson::sha1_hex(pyjson::dumps_sorted(&json!([league, query, listings])).as_bytes());
        let path = self.cache.join(format!("{key}.json"));
        if let Some(saved) = self.fresh(&path, max_age) {
            let asks: Vec<(Number, String)> = saved["asks"].as_array().into_iter().flatten().filter_map(|a| Some((a[0].as_number()?.clone(), a[1].as_str()?.to_string()))).collect();
            return Ok(priced(&asks, saved["total"].as_i64().unwrap_or(0), rates));
        }
        self.searches.fetch_add(1, Ordering::SeqCst);
        let found = self.call(&format!("{SITE}/api/trade/search/{}", quote(league)), Some(query), "search", 2)?;
        let ids: Vec<&str> = found["result"].as_array().into_iter().flatten().filter_map(Value::as_str).take(listings).collect();
        let mut asks: Vec<(Number, String)> = Vec::new();
        if !ids.is_empty() {
            let fetched = self.call(&format!("{SITE}/api/trade/fetch/{}?query={}", ids.join(","), query_id(&found)), None, "fetch", 2)?;
            for r in fetched["result"].as_array().into_iter().flatten() {
                let p = &r["listing"]["price"];
                if let (Some(a), Some(c)) = (p.get("amount").filter(|a| truthy(a)).and_then(Value::as_number), p.get("currency").and_then(Value::as_str).filter(|c| !c.is_empty())) {
                    asks.push((a.clone(), c.to_string()));
                }
            }
        }
        let total = found.get("total").and_then(Value::as_i64).unwrap_or(ids.len() as i64);
        let doc = json!({"time": self.clock.now(), "asks": asks.iter().map(|(a, c)| json!([a, c])).collect::<Vec<_>>(), "total": total});
        self.save(&path, &doc)?;
        Ok(priced(&asks, total, rates))
    }

    /// Up to `count` raw listings (`{"item": ..., "listing": ...}`) for a search, cheapest first; cached.
    ///
    /// One search plus one fetch per 10 listings.
    pub fn listings(&self, league: &str, query: &Value, count: usize, max_age: Option<f64>) -> Result<Vec<Value>> {
        let key = pyjson::sha1_hex(pyjson::dumps_sorted(&json!(["listings", league, query, count])).as_bytes());
        let path = self.cache.join(format!("{key}.json"));
        if let Some(saved) = self.fresh(&path, max_age) {
            return Ok(saved["results"].as_array().cloned().unwrap_or_default());
        }
        self.searches.fetch_add(1, Ordering::SeqCst);
        let found = self.call(&format!("{SITE}/api/trade/search/{}", quote(league)), Some(query), "search", 2)?;
        let ids: Vec<&str> = found["result"].as_array().into_iter().flatten().filter_map(Value::as_str).take(count).collect();
        let mut results = Vec::new();
        for chunk in ids.chunks(10) {
            let fetched = self.call(&format!("{SITE}/api/trade/fetch/{}?query={}", chunk.join(","), query_id(&found)), None, "fetch", 2)?;
            results.extend(fetched["result"].as_array().into_iter().flatten().filter(|r| truthy(r)).cloned());
        }
        self.save(&path, &json!({"time": self.clock.now(), "results": results}))?;
        Ok(results)
    }

    /// A cache file's document if it exists and is younger than `max_age` (a damaged file counts as missing).
    fn fresh(&self, path: &std::path::Path, max_age: Option<f64>) -> Option<Value> {
        let saved: Value = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
        (self.clock.now() - saved["time"].as_f64()? < max_age.unwrap_or(self.ttl)).then_some(saved)
    }

    fn save(&self, path: &std::path::Path, doc: &Value) -> Result<()> {
        std::fs::create_dir_all(&self.cache)?;
        std::fs::write(path, pyjson::dumps(doc))?;
        Ok(())
    }

    /// One API call, waiting for rate limits first; a 429 is waited out (`Retry-After`) and retried `retries` times.
    pub(crate) fn call(&self, url: &str, body: Option<&Value>, endpoint: &str, retries: u32) -> Result<Value> {
        let mut retries = retries;
        loop {
            self.wait_for(endpoint);
            let mut headers = vec![("User-Agent".to_string(), USER_AGENT.to_string()), ("Content-Type".into(), "application/json".into())];
            if let Some(c) = (self.cookie)() {
                headers.push(("Cookie".into(), format!("POESESSID={c}")));
            }
            let body = body.filter(|b| truthy(b)).map(|b| pyjson::dumps(b).into_bytes());
            let req = Request { method: if body.is_some() { Method::Post } else { Method::Get }, url: url.to_string(), headers, body, timeout: Duration::from_secs(30) };
            let resp = self.http.send(&req)?;
            self.remember_limits(endpoint, &resp.headers);
            if resp.ok() {
                return Ok(serde_json::from_slice(&resp.body)?);
            }
            if resp.status == 429 && retries > 0 {
                self.sleep(resp.headers.get("Retry-After").filter(|v| !v.is_empty()).and_then(|v| v.trim().parse().ok()).unwrap_or(60.0));
                retries -= 1;
                continue;
            }
            bail!("HTTP {} from {endpoint}", resp.status);
        }
    }

    /// Parse e.g. `X-Rate-Limit-Ip: 8:10:60,15:60:300` and `X-Rate-Limit-Ip-State: 1:10:0,1:60:0`.
    pub(crate) fn remember_limits(&self, endpoint: &str, headers: &Headers) {
        let Some(rules_header) = headers.get("X-Rate-Limit-Rules").filter(|r| !r.is_empty()) else { return };
        let mut rules = Vec::new();
        for rule in rules_header.split(',') {
            let limits = headers.get(&format!("X-Rate-Limit-{rule}")).unwrap_or("");
            let states = headers.get(&format!("X-Rate-Limit-{rule}-State")).unwrap_or("");
            for (lim, st) in limits.split(',').zip(states.split(',')) {
                // int() of exactly three colon-separated numbers each, else the pair is skipped
                let (Some([max_hits, period, _]), Some([hits, _, restricted])) = (triple(lim), triple(st)) else { continue };
                rules.push((max_hits, period, hits, restricted));
            }
        }
        self.limits.lock().unwrap().insert(endpoint.to_string(), (self.clock.now(), rules));
    }

    /// Sleep until one more call fits under every limit the last response reported.
    pub(crate) fn wait_for(&self, endpoint: &str) {
        let Some((seen, rules)) = self.limits.lock().unwrap().get(endpoint).cloned() else { return };
        let elapsed = self.clock.now() - seen;
        let mut wait: f64 = 0.0;
        for (max_hits, period, hits, restricted) in rules {
            if restricted != 0 {
                wait = wait.max(restricted as f64 - elapsed);
            } else if hits >= max_hits - 1 {
                // keep one call spare: other tools may share the IP
                wait = wait.max(period as f64 - elapsed);
            }
        }
        if wait > 0.0 {
            self.sleep(wait);
        }
    }

    fn sleep(&self, seconds: f64) {
        if let Some(f) = &self.on_wait {
            f(seconds);
        }
        self.clock.sleep(seconds);
    }
}

fn triple(s: &str) -> Option<[i64; 3]> {
    let v: Vec<i64> = s.split(':').map(|x| x.trim().parse().ok()).collect::<Option<_>>()?;
    v.try_into().ok()
}

/// The search id a fetch refers back to (`found['id']`).
fn query_id(found: &Value) -> String {
    match &found["id"] {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::Response;
    use crate::testing::TempDir;

    fn clock_at(now: f64) -> (Clock, Arc<Mutex<Vec<f64>>>) {
        let slept = Arc::new(Mutex::new(Vec::new()));
        let s = slept.clone();
        (Clock::new(move || now, move |x| s.lock().unwrap().push(x)), slept)
    }

    fn headers(state: &str) -> Headers {
        Headers::new([("X-Rate-Limit-Rules", "Ip"), ("X-Rate-Limit-Ip", "8:10:60,15:60:300"), ("X-Rate-Limit-Ip-State", state)])
    }

    #[test]
    fn rate_limit_waits_until_a_call_fits() {
        let (clock, slept) = clock_at(1000.0);
        let t = Trade::with(http::real(), "unused".into(), clock);
        t.remember_limits("search", &headers("7:10:0,3:60:0"));
        t.wait_for("search"); // 7 of 8 used in the 10 s window: wait it out
        assert_eq!(*slept.lock().unwrap(), [10.0]);
        t.remember_limits("search", &headers("1:10:0,1:60:0"));
        t.wait_for("search");
        assert_eq!(*slept.lock().unwrap(), [10.0]); // plenty left: no wait
        t.remember_limits("search", &headers("8:10:45,8:60:0"));
        t.wait_for("search");
        assert_eq!(*slept.lock().unwrap(), [10.0, 45.0]); // restricted: wait the penalty
    }

    #[test]
    fn on_wait_hears_about_waits_and_429_retries() {
        let (clock, slept) = clock_at(1000.0);
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let http = move |_: &Request| {
            Ok(if c.fetch_add(1, Ordering::SeqCst) == 0 { Response::new(429, "").with_headers([("Retry-After", "7")]) } else { Response::json(200, &json!({"ok": true})) })
        };
        let mut t = Trade::with(Arc::new(http), "unused".into(), clock);
        let told = Arc::new(Mutex::new(Vec::new()));
        let tw = told.clone();
        t.on_wait = Some(Arc::new(move |s| tw.lock().unwrap().push(s)));
        t.cookie = Arc::new(|| None);
        assert_eq!(t.call("u", None, "search", 2).unwrap(), json!({"ok": true}));
        assert_eq!((slept.lock().unwrap().clone(), told.lock().unwrap().clone(), calls.load(Ordering::SeqCst)), (vec![7.0], vec![7.0], 2));
    }

    #[test]
    fn prices_are_cached_in_their_own_currency() {
        let dir = TempDir::new("trade-prices");
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        let http = move |req: &Request| {
            l.lock().unwrap().push(req.url.clone());
            Ok(Response::json(
                200,
                &if req.url.contains("/search/") {
                    json!({"id": "q", "result": ["a", "b", "c"], "total": 3})
                } else {
                    let ask = |c: &str, n: i64| json!({"listing": {"price": {"currency": c, "amount": n}}});
                    json!({"result": [ask("divine", 1), ask("divine", 2), ask("chaos", 100)]})
                },
            ))
        };
        let mut t = Trade::with(Arc::new(http), dir.path().into(), Clock::new(|| 5000.0, |_| {}));
        t.cookie = Arc::new(|| None);
        let query = item_query(&ItemQuery { base: Some("Warlock Gloves".into()), stats: vec![("explicit.stat_1".into(), Some(num(40.0)))], ..Default::default() });
        let rates = |d: f64| Rates::from([("divine".into(), d), ("chaos".into(), 1.0)]);
        assert_eq!(t.price("L", &query, &rates(200.0), 10, None).unwrap(), Some((200.0, 3)));
        assert_eq!(t.price("L", &query, &rates(300.0), 10, None).unwrap(), Some((300.0, 3))); // cached asks, new divine rate
        assert_eq!(log.lock().unwrap().len(), 2); // one search, one fetch
        assert_eq!(log.lock().unwrap()[1], format!("{SITE}/api/trade/fetch/a,b,c?query=q"));
        assert_eq!(t.searches(), 1);
    }

    #[test]
    fn listings_fetch_in_tens_and_expire() {
        let dir = TempDir::new("trade-listings");
        let now = Arc::new(Mutex::new(1000.0));
        let (n, fetches) = (now.clone(), Arc::new(AtomicUsize::new(0)));
        let f = fetches.clone();
        let http = move |req: &Request| {
            Ok(Response::json(
                200,
                &if req.url.contains("/search/") {
                    assert!(req.body.is_some() && req.method == Method::Post);
                    json!({"id": "q", "result": (0..25).map(|i| format!("r{i}")).collect::<Vec<_>>(), "total": 25})
                } else {
                    f.fetch_add(1, Ordering::SeqCst);
                    json!({"result": [{"item": {}}, null, {"item": {"name": "x"}}]})
                },
            ))
        };
        let clock = Clock::new(move || *n.lock().unwrap(), |_| {});
        let mut t = Trade::with(Arc::new(http), dir.path().into(), clock);
        t.cookie = Arc::new(|| None);
        let q = item_query(&ItemQuery::default());
        assert_eq!(t.listings("L", &q, 25, None).unwrap().len(), 6); // 3 fetches, nulls dropped
        assert_eq!(fetches.load(Ordering::SeqCst), 3);
        t.listings("L", &q, 25, None).unwrap();
        assert_eq!(fetches.load(Ordering::SeqCst), 3);
        *now.lock().unwrap() = 1000.0 + 3601.0;
        t.listings("L", &q, 25, None).unwrap();
        assert_eq!(fetches.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn cookie_from_dotenv() {
        let dir = TempDir::new("trade-cookie");
        let dotenv = dir.path().join(".env");
        let none = |_: &str| None;
        assert_eq!(env::env_in("POESESSID", &none, &dotenv), None);
        std::fs::write(&dotenv, "# secrets\nexport POESESSID=\"abc123\"\n").unwrap();
        assert_eq!(env::env_in("POESESSID", &none, &dotenv).as_deref(), Some("abc123"));
        assert_eq!(env::env_in("POESESSID", &|_| Some("env-wins".into()), &dotenv).as_deref(), Some("env-wins"));
    }

    #[test]
    fn cookie_goes_in_the_header_only() {
        let seen = Arc::new(Mutex::new(String::new()));
        let s = seen.clone();
        let http = move |req: &Request| {
            *s.lock().unwrap() = req.header("Cookie").unwrap_or("").to_string();
            assert!(!format!("{req:?}").contains("secret"));
            Ok(Response::new(403, ""))
        };
        let mut t = Trade::with(Arc::new(http), "unused".into(), Clock::real());
        t.cookie = Arc::new(|| Some("secret".into()));
        let err = t.call("u", None, "search", 0).unwrap_err();
        assert_eq!(*seen.lock().unwrap(), "POESESSID=secret");
        assert!(!err.to_string().contains("secret"));
    }

    #[test]
    fn item_query_shape() {
        let q = item_query(&ItemQuery {
            category: Some("accessory.ring".into()),
            stats: vec![("fractured.stat_9".into(), Some(num(5.0)))],
            item_level: Some(84),
            fractured_only: true,
            ..Default::default()
        });
        let q = &q["query"];
        assert_eq!(q["filters"]["type_filters"]["filters"]["category"], json!({"option": "accessory.ring"}));
        assert_eq!(q["filters"]["misc_filters"]["filters"], json!({"ilvl": {"min": 84}, "fractured_item": {"option": "true"}}));
        assert_eq!(q["stats"][0]["filters"][0]["value"], json!({"min": 5}));
    }

    #[test]
    fn listing_item_marks_fractured_and_crafted_lines() {
        let listing = json!({"item": {"rarity": "Rare", "name": "Gale Loop", "baseType": "Bone Ring", "ilvl": 84, "influences": {"shaper": true},
            "fracturedMods": ["+40 to maximum Energy Shield"], "explicitMods": ["+105 to maximum Life"], "craftedMods": ["+12 to Dexterity"]}});
        let item = listing_item(&listing);
        assert_eq!((item.rarity.as_str(), item.base.as_str(), item.ilvl, item.influences), ("RARE", "Bone Ring", Some(84), BTreeSet::from(["shaper".to_string()])));
        let lines: Vec<_> = item.explicits.iter().map(|m| (m.text.as_str(), m.fractured, m.crafted)).collect();
        assert_eq!(lines, [("+40 to maximum Energy Shield", true, false), ("+105 to maximum Life", false, false), ("+12 to Dexterity", false, true)]);
    }

    #[test]
    fn cluster_query_looks_up_trade_stats() {
        let mods = json!({"Notable1": {"stats": [{"id": "s1", "min": 1, "max": 1}]}});
        let tr = json!([{"ids": ["s1"], "trade_stats": [{"type": "explicit", "id": "explicit.stat_1"}, {"type": "fractured", "id": "fractured.stat_1"}]}]);
        let repoe = Repoe::new(&mods, &tr);
        assert_eq!(stat_min(&mods, &["Notable1", "Nope"]), Some(Number::from(1)));
        let q = cluster_query(&repoe, "large", Some("opt"), Some(8), &[("Notable1".into(), None), ("Unknown".into(), Some(num(2.0)))], Some(84), &["Notable1".into()]);
        let ids: Vec<_> = q["query"]["stats"][0]["filters"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["explicit.stat_1", "Unknown", "fractured.stat_1", ENCHANT_STAT, PASSIVES_STAT]);
        assert_eq!(q["query"]["type"], "Large Cluster Jewel");
    }
}
