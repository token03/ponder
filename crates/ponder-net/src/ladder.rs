//! Characters from the poe.ninja ladder, with their items as exact mod ids.
//!
//! poe.ninja's builds API serves a search (protobuf: dimensions with counts, and
//! the top 100 characters for a filter) and one JSON document per character,
//! whose items list every explicit mod by id. That makes ladder items exact:
//! no text matching, just ids into the mod database.
//!
//! `Ladder::sample` takes the top characters of every main skill with enough
//! players, so many classes, skills and weapon types are covered, and caches
//! each character's main skill, class and rare items and jewels (nothing else)
//! under .cache/ladder/<snapshot>/.

use crate::env::Clock;
use crate::http::{self, Transport};
use anyhow::{Context, Result, anyhow, bail};
use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

pub const API: &str = "https://poe.ninja/poe1/api";
pub const USER_AGENT: &str = "ponder/0.1 (crafting research; caches everything)";
pub const CACHE: &str = ".cache/ladder";
/// Bump when `lite` keeps more fields; older .lite copies are rebuilt.
pub const LITE_VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Snapshot {
    /// display name, e.g. "Allflame"
    pub league: String,
    /// url name, e.g. "allflame"
    pub overview: String,
    pub version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Row {
    pub account: String,
    pub name: String,
}

impl Row {
    pub fn new(account: &str, name: &str) -> Row {
        Row { account: account.into(), name: name.into() }
    }
}

/// Every dimension's values with character counts (e.g. "skills" -> [(skill, count)]), most players first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dims(pub Vec<(String, Vec<(String, u64)>)>);

impl Dims {
    pub fn get(&self, name: &str) -> &[(String, u64)] {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }

    fn insert(&mut self, name: String, counts: Vec<(String, u64)>) {
        match self.0.iter_mut().find(|(n, _)| *n == name) {
            Some(slot) => slot.1 = counts,
            None => self.0.push((name, counts)),
        }
    }
}

// -- lite characters ----------------------------------------------------------------

/// One mod line of a poe.ninja item: `{"id": ..., ...}`. Kept whole, so nothing poe.ninja sends is lost.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ModEntry(pub Map<String, Value>);

impl ModEntry {
    pub fn id(&self) -> Option<&str> {
        self.0.get("id").and_then(Value::as_str)
    }
}

/// An item's mods by kind ("explicit", "implicit", "fractured", "crafted", "enchant", ...), in the order poe.ninja lists them.
pub type Mods = IndexMap<String, Vec<ModEntry>>;

/// A field that Python's `lite` copies only `if k in item`: `None` is absent, `Some(None)` is JSON null.
pub type Present<T> = Option<Option<T>>;

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Present<T>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

/// A rare item or jewel as `lite` keeps it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RareItem {
    #[serde(rename = "baseType", default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub base_type: Present<String>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub ilvl: Present<i64>,
    /// poe.ninja's influence list/object, passed through untouched.
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub influences: Present<Value>,
    #[serde(default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub mods: Present<Mods>,
    #[serde(rename = "inventoryId", default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub inventory_id: Present<String>,
    #[serde(rename = "enchantMods", default, deserialize_with = "present", skip_serializing_if = "Option::is_none")]
    pub enchant_mods: Present<Vec<String>>,
}

impl RareItem {
    pub fn base_type(&self) -> Option<&str> {
        self.base_type.as_ref()?.as_deref()
    }

    /// Mod entries of one kind, e.g. `mods_of("explicit")`.
    pub fn mods_of(&self, kind: &str) -> &[ModEntry] {
        self.mods.as_ref().and_then(|m| m.as_ref()).and_then(|m| m.get(kind)).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// What crafting analysis needs from a character: who, class, main skill, and rare items and jewels by mod id.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lite {
    pub v: u32,
    pub account: Option<String>,
    pub name: Option<String>,
    pub class: Option<String>,
    pub skill: String,
    pub rares: Vec<RareItem>,
}

impl Lite {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("lite is plain data")
    }
}

// -- the API --------------------------------------------------------------------------

/// A handle on poe.ninja and the on-disk cache. `Ladder::new()` is the real thing; tests use `with`.
#[derive(Clone)]
pub struct Ladder {
    pub http: Arc<dyn Transport>,
    pub cache: PathBuf,
    pub clock: Clock,
}

impl Default for Ladder {
    fn default() -> Self {
        Ladder::new()
    }
}

/// Settings for `Ladder::sample`.
#[derive(Clone, Debug)]
pub struct Sample {
    pub per_skill: usize,
    pub min_players: u64,
    pub per_class: usize,
    pub workers: usize,
}

impl Default for Sample {
    fn default() -> Self {
        Sample { per_skill: 15, min_players: 40, per_class: 20, workers: 4 }
    }
}

impl Ladder {
    pub fn new() -> Ladder {
        Ladder::with(http::real(), CACHE.into(), Clock::real())
    }

    pub fn with(http: Arc<dyn Transport>, cache: PathBuf, clock: Clock) -> Ladder {
        Ladder { http, cache, clock }
    }

    /// The current experience-ladder snapshot of `league` (default: the first build league).
    pub fn snapshot(&self, league: Option<&str>) -> Result<Snapshot> {
        let index = self.get_json(&format!("{API}/data/index-state"))?;
        let versions = index["snapshotVersions"].as_array().context("index-state has no snapshotVersions")?;
        for s in versions {
            let (name, url) = (s["name"].as_str().unwrap_or(""), s["url"].as_str().unwrap_or(""));
            if s["type"] == "exp" && league.is_none_or(|l| l.to_lowercase() == name.to_lowercase() || l.to_lowercase() == url) {
                return Ok(Snapshot { league: name.into(), overview: url.into(), version: s["version"].as_str().unwrap_or("").into() });
            }
        }
        bail!("no ladder snapshot for {league:?}")
    }

    /// The top 100 characters for `filters` (e.g. `[("skills", "Lightning Arrow")]`, `[("class", "Deadeye")]`),
    /// and every dimension's values with character counts.
    pub fn search(&self, snap: &Snapshot, filters: &[(&str, &str)]) -> Result<(Vec<Row>, Dims)> {
        let mut pairs = vec![("overview", snap.overview.as_str()), ("type", "exp")];
        pairs.extend_from_slice(filters);
        let body = self.get(&format!(
            "{API}/builds/{}/search?{}",
            snap.version,
            form_urlencoded::Serializer::new(String::new()).extend_pairs(&pairs).finish()
        ))?;
        let outer = fields(&body)?;
        let inner = fields(outer.first().and_then(|f| f.2.bytes()).context("empty search answer")?)?;
        fn sub_fields<'a>(f: &Field<'a>) -> Result<Vec<Field<'a>>> {
            fields(f.2.bytes().unwrap_or(&[]))
        }
        let text = |f: Option<&Field>| -> Result<String> {
            Ok(String::from_utf8(f.and_then(|f| f.2.bytes()).context("missing string field")?.to_vec())?)
        };
        let mut dict_hash = Vec::new();
        for f in inner.iter().filter(|f| f.0 == 6) {
            let sub = sub_fields(f)?;
            dict_hash.push((text(sub.first())?, text(sub.get(1))?)); // field 2: the dictionary's hash
        }
        let mut dims = Dims::default();
        for f in inner.iter().filter(|f| f.0 == 2) {
            let sub = sub_fields(f)?;
            let (name, dname) = (text(sub.first())?, text(sub.get(1))?);
            let words = match dict_hash.iter().rev().find(|(k, _)| *k == dname) {
                Some((_, hash)) => self.dictionary(hash)?,
                None => Vec::new(),
            };
            let mut counts = Vec::new();
            for g in sub.iter().filter(|g| g.0 == 3) {
                let (mut i, mut n) = (0u64, 0u64);
                for (a, _, b) in sub_fields(g)? {
                    match (a, b) {
                        (1, Val::Varint(x)) => i = x,
                        (2, Val::Varint(x)) => n = x,
                        _ => {}
                    }
                }
                counts.push((words.get(i as usize).cloned().unwrap_or_else(|| i.to_string()), n));
            }
            counts.sort_by_key(|c| std::cmp::Reverse(c.1)); // stable, like Python's sorted
            dims.insert(name, counts);
        }
        let (mut accounts, mut names) = (Vec::new(), Vec::new());
        for f in inner.iter().filter(|f| f.0 == 12) {
            let sub = sub_fields(f)?;
            let strings = |sub: &[Field]| -> Result<Vec<String>> {
                sub.iter().filter(|g| g.0 == 7).map(|g| Ok(String::from_utf8(g.2.bytes().unwrap_or(&[]).to_vec())?)).collect()
            };
            match sub.first().and_then(|f| f.2.bytes()) {
                Some(b"name") => names = strings(&sub)?,
                Some(b"account") => accounts = strings(&sub)?,
                _ => {}
            }
        }
        let rows = accounts.into_iter().zip(names).map(|(a, n)| Row { account: a, name: n }).collect();
        Ok((rows, dims))
    }

    /// A poe.ninja word list ("NDIC"), downloaded once into `<cache>/dictionaries/<hash>`.
    pub fn dictionary(&self, hash: &str) -> Result<Vec<String>> {
        if hash.contains(['/', '\\']) || hash.contains("..") {
            bail!("bad dictionary hash {hash:?}"); // it becomes a file name
        }
        let path = self.cache.join("dictionaries").join(hash);
        if !path.is_file() {
            std::fs::create_dir_all(path.parent().unwrap())?;
            std::fs::write(&path, self.get(&format!("{API}/builds/dictionary/{hash}"))?)?;
        }
        ndic(&std::fs::read(&path)?)
    }

    /// One character as a `Lite` (main skill, class, rare items and jewels by mod id), cached per snapshot.
    /// `None` if poe.ninja has no usable document for it.
    ///
    /// Only the lite copy is kept: the full document (passive tree, PoB export,
    /// uniques, gems) is ~100x bigger and nothing reads it.
    pub fn character(&self, snap: &Snapshot, row: &Row) -> Result<Option<Lite>> {
        let dir = self.cache.join(&snap.version).join(safe(&row.account));
        let small = dir.join(safe(&row.name)).with_added_extension("lite");
        if let Some(data) = read_lite(&small) {
            return Ok(Some(data));
        }
        let query = form_urlencoded::Serializer::new(String::new())
            .extend_pairs([
                ("account", row.account.as_str()),
                ("name", row.name.as_str()),
                ("overview", snap.overview.as_str()),
                ("type", "exp"),
            ])
            .finish();
        let data = match self.get_json(&format!("{API}/builds/{}/character?{query}", snap.version)).and_then(|doc| lite(&doc)) {
            Ok(data) => data,
            Err(_) => return Ok(None),
        };
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&small, data.to_json())?;
        Ok(Some(data))
    }

    /// Top characters of every main skill with at least `min_players`, plus of every ascendancy.
    ///
    /// Ladder order (level and experience) puts invested characters first, which
    /// is what crafted items are sold to. Duplicates across filters are fetched once.
    /// Characters are fetched by `workers` threads and returned in ladder order.
    pub fn sample(&self, snap: &Snapshot, opts: &Sample, progress: Option<&(dyn Fn(&str) + Sync)>) -> Result<Vec<Lite>> {
        let say = |msg: &str| {
            if let Some(p) = progress {
                p(msg)
            }
        };
        let (_, dims) = self.search(snap, &[])?;
        let mut filters: Vec<(&str, &str)> = dims.get("skills").iter().filter(|(_, n)| *n >= opts.min_players).map(|(s, _)| ("skills", s.as_str())).collect();
        filters.extend(dims.get("class").iter().filter(|(_, n)| *n > 0).map(|(c, _)| ("class", c.as_str())));
        let (mut rows, mut seen) = (Vec::new(), HashSet::new());
        for (i, (key, value)) in filters.iter().enumerate() {
            say(&format!("ladder search {}/{}: {value}", i + 1, filters.len()));
            let (found, _) = self.search(snap, &[(key, value)])?;
            for r in found.into_iter().take(if *key == "skills" { opts.per_skill } else { opts.per_class }) {
                if seen.insert(r.clone()) {
                    rows.push(r);
                }
            }
        }
        say(&format!("fetching {} characters", rows.len()));
        let results: Vec<Mutex<Option<Lite>>> = rows.iter().map(|_| Mutex::new(None)).collect();
        let (next, done) = (AtomicUsize::new(0), AtomicUsize::new(0));
        std::thread::scope(|s| {
            for _ in 0..opts.workers.max(1) {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        let Some(row) = rows.get(i) else { break };
                        *results[i].lock().unwrap() = self.character(snap, row).ok().flatten();
                        let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                        if n % 50 == 0 {
                            say(&format!("characters {n}/{}", rows.len()));
                        }
                    }
                });
            }
        });
        Ok(results.into_iter().filter_map(|m| m.into_inner().unwrap()).collect())
    }

    /// Characters already downloaded (for `snap`, else the newest snapshot on disk), as `Lite`s; no network.
    ///
    /// Full documents left by older versions are turned into `.lite` copies and deleted.
    pub fn cached(&self, snap: Option<&Snapshot>) -> Result<Vec<Lite>> {
        let root = match snap {
            Some(s) => self.cache.join(&s.version),
            None => {
                let Ok(entries) = std::fs::read_dir(&self.cache) else { return Ok(Vec::new()) };
                let mut dirs: Vec<PathBuf> =
                    entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n != "dictionaries")).collect();
                dirs.sort();
                match dirs.pop() {
                    Some(d) => d,
                    None => return Ok(Vec::new()),
                }
            }
        };
        // root.glob("*/*.json") | root.glob("*/*.lite"), as stems (suffix stripped the pathlib way)
        let mut stems = std::collections::BTreeSet::new();
        for account in std::fs::read_dir(&root).into_iter().flatten().filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()) {
            for f in std::fs::read_dir(&account).into_iter().flatten().filter_map(|e| e.ok()) {
                let name = f.file_name().to_string_lossy().into_owned();
                if name.ends_with(".json") || name.ends_with(".lite") {
                    stems.insert(account.join(&name).with_extension(""));
                }
            }
        }
        let mut out = Vec::new();
        for stem in stems {
            let (dir, name) = (stem.parent().unwrap(), stem.file_name().unwrap().to_string_lossy().into_owned());
            let (path, small) = (dir.join(&name).with_added_extension("json"), dir.join(&name).with_added_extension("lite"));
            if let Some(data) = read_lite(&small) {
                let _ = std::fs::remove_file(&path);
                out.push(data);
                continue;
            }
            if !path.is_file() {
                continue; // an outdated lite copy with nothing to rebuild it from (`sample` fetches it again)
            }
            let Ok(data) = std::fs::read_to_string(&path).map_err(anyhow::Error::from).and_then(|t| Ok(serde_json::from_str::<Value>(&t)?)).and_then(|doc| lite(&doc)) else {
                continue;
            };
            std::fs::write(&small, data.to_json())?;
            std::fs::remove_file(&path)?;
            out.push(data);
        }
        Ok(out)
    }

    fn get(&self, url: &str) -> Result<Vec<u8>> {
        self.get_retrying(url, 3)
    }

    /// `_get`: three tries, waiting 2 s then 4 s between them.
    fn get_retrying(&self, url: &str, retries: u32) -> Result<Vec<u8>> {
        for attempt in 0..retries {
            match http::get(&*self.http, url, USER_AGENT, 60) {
                Ok(b) => return Ok(b),
                Err(e) if attempt == retries - 1 => return Err(e),
                Err(_) => self.clock.sleep(2.0 * (attempt + 1) as f64),
            }
        }
        Err(anyhow!("no attempts"))
    }

    fn get_json(&self, url: &str) -> Result<Value> {
        Ok(serde_json::from_slice(&self.get(url)?)?)
    }
}

/// A cached `.lite` file, if it is readable and current.
fn read_lite(path: &Path) -> Option<Lite> {
    if !path.is_file() {
        return None;
    }
    let lite: Lite = serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()?;
    (lite.v == LITE_VERSION).then_some(lite)
}

/// What crafting analysis needs from a character document (poe.ninja's, or a `Lite` as JSON).
pub fn lite(data: &Value) -> Result<Lite> {
    let rares = rare_items(data)
        .into_iter()
        .map(|item| {
            let kept: Map<String, Value> =
                ["baseType", "ilvl", "influences", "mods", "inventoryId", "enchantMods"].iter().filter_map(|k| item.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
            Ok(serde_json::from_value(Value::Object(kept))?)
        })
        .collect::<Result<Vec<RareItem>>>()?;
    let text = |k: &str| data.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(Lite { v: LITE_VERSION, account: text("account"), name: text("name"), class: text("class"), skill: main_skill(data), rares })
}

/// The character's main skill: the first active gem of the first skill group with DPS numbers
/// (`demand.main_skill`; only reads `data["skills"]`).
pub fn main_skill(data: &Value) -> String {
    let empty = Vec::new();
    let groups = data.get("skills").and_then(Value::as_array).unwrap_or(&empty);
    let (with_dps, without): (Vec<&Value>, Vec<&Value>) =
        groups.iter().partition(|g| g.get("dps").and_then(Value::as_array).is_some_and(|dps| !dps.is_empty()));
    for group in with_dps.into_iter().chain(without) {
        for gem in group.get("allGems").and_then(Value::as_array).unwrap_or(&empty) {
            let name = gem.get("name").and_then(Value::as_str).unwrap_or("");
            if !name.is_empty() && !name.ends_with("Support") {
                return name.to_string();
            }
        }
    }
    "?".into()
}

/// A character's rare equipment and jewels (poe.ninja `itemData`, or a `lite` dict's).
pub fn rare_items(data: &Value) -> Vec<&Value> {
    if let Some(rares) = data.get("rares") {
        return rares.as_array().map(|a| a.iter().collect()).unwrap_or_default();
    }
    let list = |k: &str| data.get(k).and_then(Value::as_array).map(|a| a.iter()).into_iter().flatten();
    list("items")
        .chain(list("jewels"))
        .filter_map(|entry| entry.get("itemData"))
        .filter(|item| item.get("frameType") == Some(&Value::from(2)) && item.get("identified").is_none_or(|v| v.as_bool() == Some(true)))
        .collect()
}

// -- transport and formats ------------------------------------------------------------

/// `re.sub(r"[^\w.-]", "_", name)`
pub fn safe(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || c == '_' || c == '.' || c == '-' { c } else { '_' }).collect()
}

/// A protobuf field value (wire types 0 and 2; fixed-width ones are skipped over).
#[derive(Clone, Debug, PartialEq)]
pub enum Val<'a> {
    Varint(u64),
    Bytes(&'a [u8]),
}

impl<'a> Val<'a> {
    pub fn bytes(&self) -> Option<&'a [u8]> {
        match self {
            Val::Bytes(b) => Some(b),
            Val::Varint(_) => None,
        }
    }
}

/// (field number, wire type, value)
pub type Field<'a> = (u32, u8, Val<'a>);

pub fn varint(b: &[u8], mut i: usize) -> Result<(u64, usize)> {
    let (mut r, mut s) = (0u64, 0u32);
    loop {
        let x = *b.get(i).ok_or_else(|| anyhow!("varint runs past the end"))?;
        i += 1;
        if s < 64 {
            r |= u64::from(x & 0x7F) << s;
        }
        s += 7;
        if x < 0x80 {
            return Ok((r, i));
        }
    }
}

/// Protobuf wire format, schema-less: (field number, wire type, value).
pub fn fields(b: &[u8]) -> Result<Vec<Field<'_>>> {
    let (mut i, mut out) = (0, Vec::new());
    let slice = |i: usize, n: usize| &b[i.min(b.len())..i.saturating_add(n).min(b.len())];
    while i < b.len() {
        let (key, j) = varint(b, i)?;
        i = j;
        let (f, wt) = ((key >> 3) as u32, (key & 7) as u8);
        let v = match wt {
            0 => {
                let (v, j) = varint(b, i)?;
                i = j;
                Val::Varint(v)
            }
            2 => {
                let (n, j) = varint(b, i)?;
                let n = usize::try_from(n).unwrap_or(usize::MAX);
                i = j;
                let v = slice(i, n);
                i = i.saturating_add(n);
                Val::Bytes(v)
            }
            1 | 5 => {
                let n = if wt == 1 { 8 } else { 4 };
                let v = slice(i, n);
                i += n;
                Val::Bytes(v)
            }
            _ => bail!("wire type {wt}"),
        };
        out.push((f, wt, v));
    }
    Ok(out)
}

/// poe.ninja's dictionary format ("NDIC"): a header, a block index, varint string lengths, the strings.
///
/// Header (little-endian u32s): magic, version, 0, count, then an 8-byte hash,
/// block size, block count, and the byte length of the varint length table;
/// then (offset, first index) per block.
pub fn ndic(b: &[u8]) -> Result<Vec<String>> {
    if b.get(..4) != Some(b"NDIC") {
        bail!("not an NDIC dictionary");
    }
    let u32_at = |o: usize| -> Result<usize> {
        Ok(u32::from_le_bytes(b.get(o..o + 4).ok_or_else(|| anyhow!("NDIC header too short"))?.try_into().unwrap()) as usize)
    };
    let count = u32_at(12)?;
    let (blocks, length_bytes) = (u32_at(28)?, u32_at(32)?);
    let mut i = 36 + 8 * blocks;
    let end = i + length_bytes;
    let mut lengths = Vec::new();
    while i < end {
        let (n, j) = varint(b, i)?;
        lengths.push(n as usize);
        i = j;
    }
    if lengths.len() != count {
        bail!("NDIC: {} lengths for {count} strings", lengths.len());
    }
    let mut out = Vec::with_capacity(count);
    for n in lengths {
        let s = &b[i.min(b.len())..i.saturating_add(n).min(b.len())];
        out.push(String::from_utf8_lossy(s).into_owned());
        i = i.saturating_add(n);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Request, Response};
    use crate::testing::TempDir;
    use serde_json::json;

    fn put_varint(mut n: usize) -> Vec<u8> {
        let mut out = Vec::new();
        loop {
            let (b, rest) = ((n & 0x7F) as u8, n >> 7);
            out.push(if rest > 0 { b | 0x80 } else { b });
            if rest == 0 {
                return out;
            }
            n = rest;
        }
    }

    /// The NDIC bytes of `words` (one block).
    fn make_ndic(words: &[&str]) -> Vec<u8> {
        let lengths: Vec<u8> = words.iter().flat_map(|w| put_varint(w.len())).collect();
        let mut out = b"NDIC".to_vec();
        for n in [2u32, 0, words.len() as u32] {
            out.extend(n.to_le_bytes());
        }
        out.extend([0u8; 8]);
        for n in [16u32, 1, lengths.len() as u32, 0, 0] {
            out.extend(n.to_le_bytes());
        }
        out.extend(lengths);
        out.extend(words.iter().flat_map(|w| w.bytes()));
        out
    }

    fn field_bytes(f: usize, b: &[u8]) -> Vec<u8> {
        [put_varint(f << 3 | 2), put_varint(b.len()), b.to_vec()].concat()
    }

    fn field_int(f: usize, n: usize) -> Vec<u8> {
        [put_varint(f << 3), put_varint(n)].concat()
    }

    #[test]
    fn ndic_round_trip_with_long_strings() {
        let long = "x".repeat(300);
        let words = ["Ascendant", "", long.as_str(), "\u{dc}n\u{ef}code"];
        assert_eq!(ndic(&make_ndic(&words)).unwrap(), words);
        assert!(ndic(b"NOPE").is_err());
    }

    #[test]
    fn protobuf_fields() {
        let mut msg = vec![0x08, 0x96, 0x01, 0x12, 0x03];
        msg.extend(b"abc");
        assert_eq!(fields(&msg).unwrap(), vec![(1, 0, Val::Varint(150)), (2, 2, Val::Bytes(b"abc"))]);
        assert!(fields(&[0x0B]).is_err()); // wire type 3
    }

    fn fake_ninja() -> (Arc<dyn Transport>, Arc<Mutex<Vec<String>>>) {
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = log.clone();
        let full = json!({"account": "a", "name": "n", "class": "Witch", "skills": [], "passives": (0..1000).collect::<Vec<_>>(),
            "items": [{"itemData": {"frameType": 2, "baseType": "Bone Ring", "mods": {"explicit": [{"id": "IncreasedLife7"}]}}}]});
        let http = move |req: &Request| {
            l.lock().unwrap().push(req.url.clone());
            let u = req.url.as_str();
            Ok(if u.ends_with("/data/index-state") {
                Response::json(200, &json!({"snapshotVersions": [{"type": "depth", "name": "X", "url": "x", "version": "0"}, {"type": "exp", "name": "Allflame", "url": "allflame", "version": "v9"}]}))
            } else if u.contains("/builds/dictionary/") {
                Response::new(200, make_ndic(&["Cyclone", "Lightning Arrow", "Witch"]))
            } else if u.contains("/search?") {
                let dict = [field_bytes(1, b"skill_dict"), field_bytes(2, b"hash1")].concat();
                let dim = |name: &[u8], counts: &[(usize, usize)]| {
                    let mut d = [field_bytes(1, name), field_bytes(2, b"skill_dict")].concat();
                    for (i, n) in counts {
                        d.extend(field_bytes(3, &[field_int(1, *i), field_int(2, *n)].concat()));
                    }
                    d
                };
                let col = |name: &[u8], vals: &[&str]| [field_bytes(1, name), vals.iter().flat_map(|v| field_bytes(7, v.as_bytes())).collect()].concat();
                let inner = [
                    field_bytes(6, &dict),
                    field_bytes(2, &dim(b"skills", &[(0, 10), (1, 77), (7, 3)])),
                    field_bytes(12, &col(b"account", &["a", "b"])),
                    field_bytes(12, &col(b"name", &["n", "m"])),
                ]
                .concat();
                Response::new(200, field_bytes(1, &inner))
            } else if u.contains("/character?") {
                Response::json(200, &full)
            } else {
                Response::new(404, "")
            })
        };
        (Arc::new(http), log)
    }

    #[test]
    fn snapshot_and_search_read_poe_ninja() {
        let dir = TempDir::new("ladder-search");
        let (http, log) = fake_ninja();
        let ladder = Ladder::with(http, dir.path().into(), Clock::new(|| 0.0, |_| {}));
        let snap = ladder.snapshot(Some("ALLFLAME")).unwrap();
        assert_eq!(snap, Snapshot { league: "Allflame".into(), overview: "allflame".into(), version: "v9".into() });
        assert!(ladder.snapshot(Some("nope")).is_err());
        let (rows, dims) = ladder.search(&snap, &[("class", "Dead eye")]).unwrap();
        assert_eq!(rows, vec![Row::new("a", "n"), Row::new("b", "m")]);
        assert_eq!(dims.get("skills"), [("Lightning Arrow".to_string(), 77), ("Cyclone".to_string(), 10), ("7".to_string(), 3)]);
        assert!(log.lock().unwrap().iter().any(|u| u.ends_with("/builds/v9/search?overview=allflame&type=exp&class=Dead+eye")));
        assert!(dir.path().join("dictionaries/hash1").is_file());
    }

    #[test]
    fn failed_requests_are_retried_with_backoff() {
        let (n, slept) = (Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(Vec::new())));
        let (c, s) = (n.clone(), slept.clone());
        let http = move |_: &Request| if c.fetch_add(1, Ordering::SeqCst) < 2 { Err(anyhow!("reset")) } else { Ok(Response::new(200, "[]")) };
        let ladder = Ladder::with(Arc::new(http), "unused".into(), Clock::new(|| 0.0, move |x| s.lock().unwrap().push(x)));
        assert_eq!(ladder.get_json("u").unwrap(), json!([]));
        assert_eq!(*slept.lock().unwrap(), [2.0, 4.0]);
    }

    #[test]
    fn cache_keeps_only_the_lite_character() {
        // Fetched characters are stored as their lite record only; full documents left on disk become lite.
        let dir = TempDir::new("ladder-cache");
        let (http, _) = fake_ninja();
        let ladder = Ladder::with(http, dir.path().into(), Clock::new(|| 0.0, |_| {}));
        let snap = Snapshot { league: "L".into(), overview: "l".into(), version: "v1".into() };
        let got = ladder.character(&snap, &Row::new("a", "n")).unwrap().unwrap();
        assert_eq!(got.rares[0].base_type(), Some("Bone Ring"));
        assert_eq!(got.rares[0].mods_of("explicit")[0].id(), Some("IncreasedLife7"));
        let files: Vec<_> = walk(&dir.path().join("v1"));
        assert_eq!(files, ["lite"]);
        let old = dir.path().join("v1/b/m.with.dots.json");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, json!({"account": "b", "name": "m.with.dots", "class": "Witch", "skills": [], "items": []}).to_string()).unwrap();
        let mut names: Vec<_> = ladder.cached(Some(&snap)).unwrap().into_iter().filter_map(|c| c.name).collect();
        names.sort();
        assert_eq!(names, ["m.with.dots", "n"]);
        assert!(!old.exists() && old.with_extension("lite").is_file());
        // newest snapshot on disk when none is named
        assert_eq!(ladder.cached(None).unwrap().len(), 2);
    }

    fn walk(dir: &Path) -> Vec<String> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() { out.extend(walk(&p)) } else { out.push(p.extension().unwrap().to_string_lossy().into_owned()) }
        }
        out
    }

    #[test]
    fn sample_fetches_each_character_once() {
        let dir = TempDir::new("ladder-sample");
        let (http, log) = fake_ninja();
        let ladder = Ladder::with(http, dir.path().into(), Clock::new(|| 0.0, |_| {}));
        let snap = Snapshot { league: "L".into(), overview: "l".into(), version: "v1".into() };
        let said = Mutex::new(Vec::new());
        let progress = |m: &str| said.lock().unwrap().push(m.to_string());
        let opts = Sample { min_players: 40, ..Sample::default() };
        let got = ladder.sample(&snap, &opts, Some(&progress)).unwrap();
        // the fake search answers the same two rows for every filter: 1 skill (>= 40 players) + 0 classes
        assert_eq!(got.len(), 2);
        assert_eq!(log.lock().unwrap().iter().filter(|u| u.contains("/character?")).count(), 2);
        assert_eq!(said.lock().unwrap()[0], "ladder search 1/1: Lightning Arrow");
    }

    #[test]
    fn lite_preserves_missing_and_null_fields() {
        let text = r#"{"v": 2, "account": "a", "name": null, "class": "Witch", "skill": "Cyclone", "rares": [{"baseType": "Bone Ring", "ilvl": null, "influences": {"shaper": true}, "mods": {"explicit": [{"tier": 1.5, "id": "X", "magnitudes": [{"min": 1, "max": 2}]}], "enchant": []}, "enchantMods": ["a"]}, {}]}"#;
        let lite: Lite = serde_json::from_str(text).unwrap();
        assert_eq!(lite.rares[0].ilvl, Some(None)); // present and null stays so
        assert_eq!(lite.rares[1].ilvl, None); // absent stays absent
        assert_eq!(serde_json::from_str::<Value>(&lite.to_json()).unwrap(), serde_json::from_str::<Value>(text).unwrap());
    }

    #[test]
    fn main_skill_is_the_first_active_gem_of_the_first_group_with_dps() {
        let data = json!({"skills": [
            {"allGems": [{"name": "Cyclone"}]},
            {"dps": [1], "allGems": [{"name": "Added Fire Support"}, {"name": ""}, {"name": "Lightning Arrow"}]},
        ]});
        assert_eq!(main_skill(&data), "Lightning Arrow");
        assert_eq!(main_skill(&json!({})), "?");
    }

    #[test]
    fn rare_items_are_identified_rares_and_jewels() {
        let data = json!({"items": [{"itemData": {"frameType": 2, "baseType": "A"}}, {"itemData": {"frameType": 3}}, {"itemData": {"frameType": 2, "identified": false}}],
            "jewels": [{"itemData": {"frameType": 2, "baseType": "J"}}, {}]});
        assert_eq!(lite(&data).unwrap().rares.iter().filter_map(|r| r.base_type()).collect::<Vec<_>>(), ["A", "J"]);
    }

    #[test]
    fn safe_names_keep_word_characters() {
        assert_eq!(safe("Name#1234 x.y-z"), "Name_1234_x.y-z");
    }
}
