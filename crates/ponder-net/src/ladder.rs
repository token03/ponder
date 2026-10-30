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
//! under data/ladder/<snapshot>/.

use crate::env::Clock;
use crate::http::{self, Transport};
use anyhow::{Context, Result, anyhow, bail};
use indexmap::IndexMap;
use rayon::prelude::*;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

pub const API: &str = "https://poe.ninja/poe1/api";
pub const USER_AGENT: &str = "ponder/0.1 (crafting research; caches everything)";
pub const DATA_DIR: &str = "data/ladder";
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
        Ladder::with(http::real(), DATA_DIR.into(), Clock::real())
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
        let mut filters: Vec<(&str, &str)> =
            dims.get("skills").iter().filter(|(_, n)| *n >= opts.min_players).map(|(s, _)| ("skills", s.as_str())).collect();
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
        let pool = rayon::ThreadPoolBuilder::new().num_threads(opts.workers.max(1)).build()?;
        let done = AtomicUsize::new(0);
        let results: Vec<_> = pool.install(|| {
            rows.par_iter()
                .map(|row| {
                    let character = self.character(snap, row).ok().flatten();
                    let n = done.fetch_add(1, Ordering::Relaxed) + 1;
                    if n.is_multiple_of(50) {
                        say(&format!("characters {n}/{}", rows.len()));
                    }
                    character
                })
                .collect()
        });
        Ok(results.into_iter().flatten().collect())
    }

    /// Characters already downloaded (for `snap`, else the newest snapshot on disk), as `Lite`s; no network.
    ///
    /// Full documents left by older versions are turned into `.lite` copies and deleted.
    pub fn cached(&self, snap: Option<&Snapshot>) -> Result<Vec<Lite>> {
        let root = match snap {
            Some(s) => self.cache.join(&s.version),
            None => {
                let Ok(entries) = std::fs::read_dir(&self.cache) else {
                    return Ok(Vec::new());
                };
                let mut dirs: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.is_dir() && p.file_name().is_some_and(|n| n != "dictionaries"))
                    .collect();
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
            let Ok(data) = std::fs::read_to_string(&path)
                .map_err(anyhow::Error::from)
                .and_then(|t| Ok(serde_json::from_str::<Value>(&t)?))
                .and_then(|doc| lite(&doc))
            else {
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
            let kept: Map<String, Value> = ["baseType", "ilvl", "influences", "mods", "inventoryId", "enchantMods"]
                .iter()
                .filter_map(|k| item.get(*k).map(|v| (k.to_string(), v.clone())))
                .collect();
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
