//! Game data exported by repoe-fork, loaded from the local data directory.
//!
//! The JSON exports are big (mods 34 MB, base items 8 MB, stat translations
//! 12 MB), so each file is read once per process into typed structs holding only
//! the fields the rest of the tool reads, and shared as `Arc`s. Downloading them
//! is network work and lives in the net crate, which reuses [`SOURCE`] and
//! [`FILES`].

use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde::Deserialize;
use serde::de::DeserializeOwned;

/// Where repoe-fork publishes its exports (`{SOURCE}/data/{name}.json`, `{SOURCE}/exported-version.txt`).
pub const SOURCE: &str = "https://raw.githubusercontent.com/repoe-fork/repoe-fork.github.io/master";
/// The data files the tool needs, by name (without `.json`).
pub const FILES: &[&str] = &[
    "mods",
    "base_items",
    "item_classes",
    "tags",
    "essences",
    "fossils",
    "crafting_bench_options",
    "cluster_jewels",
    "cluster_jewel_notables",
    "stat_translations",
];

/// Why a data file could not be loaded.
#[derive(Debug)]
pub enum DataError {
    /// The file is not in the data directory (Python: `FileNotFoundError`).
    Missing(PathBuf),
    Io(PathBuf, std::io::Error),
    Parse(PathBuf, serde_json::Error),
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DataError::Missing(p) => {
                write!(f, "{} missing; run `ponder data fetch` first", p.display())
            }
            DataError::Io(p, e) => write!(f, "{}: {e}", p.display()),
            DataError::Parse(p, e) => write!(f, "{}: {e}", p.display()),
        }
    }
}

impl std::error::Error for DataError {}

// -- settings -------------------------------------------------------------------------

static DATA_DIR: OnceLock<PathBuf> = OnceLock::new();

pub fn set_data_dir(dir: PathBuf) -> Result<(), PathBuf> {
    DATA_DIR.set(dir)
}

pub fn data_dir() -> PathBuf {
    DATA_DIR
        .get_or_init(|| match std::env::var_os("PONDER_DATA") {
            Some(p) if !p.is_empty() => PathBuf::from(p),
            _ => std::env::current_dir().unwrap_or_default().join("data").join("repoe"),
        })
        .clone()
}

/// Are the files matching needs (mods and base items) in `dest` (default: [`data_dir`])?
pub fn available(dest: Option<&Path>) -> bool {
    let dest = dest.map(Path::to_path_buf).unwrap_or_else(data_dir);
    ["mods", "base_items"].iter().all(|n| dest.join(format!("{n}.json")).is_file() || dest.join(format!("{n}.json.gz")).is_file())
}

/// The game version the data was exported from, if recorded.
pub fn version(dest: Option<&Path>) -> Option<String> {
    let dest = dest.map(Path::to_path_buf).unwrap_or_else(data_dir);
    std::fs::read_to_string(dest.join("version.txt")).ok().map(|s| s.trim().to_string())
}

// -- loading --------------------------------------------------------------------------

/// Parse `{dir}/{name}.json` into `T`, uncached.
pub fn load_from<T: DeserializeOwned>(dir: &Path, name: &str) -> Result<T, DataError> {
    let path = dir.join(format!("{name}.json"));
    let path = if path.is_file() { path } else { path.with_added_extension("gz") };
    if !path.is_file() {
        return Err(DataError::Missing(path));
    }
    let bytes = std::fs::read(&path).map_err(|e| DataError::Io(path.clone(), e))?;
    let bytes = if path.extension().is_some_and(|e| e == "gz") {
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(bytes.as_slice()).read_to_end(&mut raw).map_err(|e| DataError::Io(path.clone(), e))?;
        raw
    } else {
        bytes
    };
    serde_json::from_slice(&bytes).map_err(|e| DataError::Parse(path, e))
}

/// `load_from` in [`data_dir`], remembered in `slot` for the life of the process.
fn cached<T: DeserializeOwned>(slot: &'static OnceLock<Arc<T>>, name: &str) -> Result<Arc<T>, DataError> {
    if let Some(v) = slot.get() {
        return Ok(v.clone());
    }
    let v = Arc::new(load_from::<T>(&data_dir(), name)?);
    Ok(slot.get_or_init(|| v).clone())
}

/// Any data file as untyped JSON (`tags`, `item_classes`, or one this module has no struct for); cached.
pub fn load(name: &str) -> Result<Arc<serde_json::Value>, DataError> {
    static CACHE: OnceLock<Mutex<HashMap<String, Arc<serde_json::Value>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().unwrap().get(name) {
        return Ok(v.clone());
    }
    let v = Arc::new(load_from::<serde_json::Value>(&data_dir(), name)?);
    Ok(cache.lock().unwrap().entry(name.to_string()).or_insert(v).clone())
}

pub type OrderedMap<T> = indexmap::IndexMap<String, T>;

// -- typed files -------------------------------------------------------------------------

/// One entry of a mod's `spawn_weights`: the mod may roll on items carrying `tag` at `weight`.
#[derive(Debug, Clone, Deserialize)]
pub struct SpawnWeight {
    pub tag: String,
    pub weight: i64,
}

/// One stat of a mod with its roll range.
#[derive(Debug, Clone, Deserialize)]
pub struct ModStat {
    pub id: String,
    #[serde(default)]
    pub min: f64,
    #[serde(default)]
    pub max: f64,
}

/// A repoe mod (`mods.json`), with the fields the tool reads.
#[derive(Debug, Clone, Deserialize)]
pub struct Mod {
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub generation_type: String,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub implicit_tags: Vec<String>,
    #[serde(default)]
    pub adds_tags: Vec<String>,
    #[serde(default)]
    pub is_essence_only: bool,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub required_level: i64,
    #[serde(default)]
    pub spawn_weights: Vec<SpawnWeight>,
    #[serde(default)]
    pub generation_weights: Vec<SpawnWeight>,
    #[serde(default)]
    pub stats: Vec<ModStat>,
    /// Display template, lines joined by `\n`; `None` for mods without text.
    #[serde(default)]
    pub text: Option<String>,
    /// The mod family shared by every tier (Python key `type`).
    #[serde(rename = "type", default)]
    pub type_: String,
}

impl Mod {
    /// The text, or "" for none (Python `mod.get("text") or ""`).
    pub fn text_or_empty(&self) -> &str {
        self.text.as_deref().unwrap_or("")
    }
}

/// All mods by id, in file order.
pub type Mods = OrderedMap<Mod>;

/// A base item type (`base_items.json`).
#[derive(Debug, Clone, Deserialize)]
pub struct BaseItem {
    #[serde(default)]
    pub domain: String,
    #[serde(default)]
    pub drop_level: i64,
    #[serde(default)]
    pub item_class: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub release_state: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub implicits: Vec<String>,
}

/// All base items by id, in file order.
pub type BaseItems = OrderedMap<BaseItem>;

#[derive(Debug, Clone, Deserialize)]
pub struct EssenceType {
    #[serde(default)]
    pub is_corruption_only: bool,
    #[serde(default)]
    pub tier: i64,
}

/// An essence: the mod it forces per item class.
#[derive(Debug, Clone, Deserialize)]
pub struct Essence {
    pub name: String,
    #[serde(default)]
    pub level: i64,
    #[serde(default)]
    pub item_level_restriction: Option<i64>,
    #[serde(default)]
    pub spawn_level_min: i64,
    /// Item class -> forced mod id.
    #[serde(default)]
    pub mods: BTreeMap<String, Option<String>>,
    #[serde(rename = "type")]
    pub type_: EssenceType,
}

/// A tag with the weight multiplier (percent) a fossil gives mods carrying it.
#[derive(Debug, Clone, Deserialize)]
pub struct WeightTag {
    pub tag: String,
    pub weight: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fossil {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub added_mods: Vec<String>,
    #[serde(default)]
    pub forced_mods: Vec<String>,
    #[serde(default)]
    pub mirrors: bool,
    #[serde(default)]
    pub rolls_lucky: bool,
    #[serde(default)]
    pub changes_quality: bool,
    #[serde(default)]
    pub positive_mod_weights: Vec<WeightTag>,
    #[serde(default)]
    pub negative_mod_weights: Vec<WeightTag>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BenchActions {
    /// Mod id the option adds (None for sockets, enchants, ...).
    #[serde(default)]
    pub add_explicit_mod: Option<String>,
}

/// One crafting bench option.
#[derive(Debug, Clone, Deserialize)]
pub struct BenchOption {
    #[serde(default)]
    pub actions: Option<BenchActions>,
    #[serde(default)]
    pub item_classes: Vec<String>,
    /// Currency metadata path -> amount, in file order.
    #[serde(default)]
    pub cost: OrderedMap<f64>,
    #[serde(default)]
    pub master: String,
    #[serde(default)]
    pub bench_tier: i64,
}

impl BenchOption {
    /// The mod id this option adds, if any (Python `(o.get("actions") or {}).get("add_explicit_mod")`).
    pub fn add_explicit_mod(&self) -> Option<&str> {
        self.actions.as_ref().and_then(|a| a.add_explicit_mod.as_deref())
    }
}

/// An enchant a cluster jewel size can carry.
#[derive(Debug, Clone, Deserialize)]
pub struct ClusterPassive {
    pub id: String,
    pub name: String,
    pub tag: String,
    pub stat_text: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClusterJewel {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: String,
    #[serde(default)]
    pub min_skills: i64,
    #[serde(default)]
    pub max_skills: i64,
    #[serde(default)]
    pub passive_skills: Vec<ClusterPassive>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClusterNotable {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub jewel_stat: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TradeStat {
    pub id: String,
    #[serde(default)]
    pub text: String,
    #[serde(rename = "type")]
    pub type_: String,
}

/// A stat translation entry; only the stat ids and their trade-site ids are kept.
#[derive(Debug, Clone, Deserialize)]
pub struct StatTranslation {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub trade_stats: Option<Vec<TradeStat>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ItemClass {
    #[serde(default)]
    pub category: Option<String>,
    #[serde(default)]
    pub category_id: Option<String>,
    #[serde(default)]
    pub name: String,
}

macro_rules! typed {
    ($(#[$doc:meta])* $fn:ident, $name:literal, $ty:ty) => {
        $(#[$doc])*
        pub fn $fn() -> Result<Arc<$ty>, DataError> {
            static SLOT: OnceLock<Arc<$ty>> = OnceLock::new();
            cached(&SLOT, $name)
        }
    };
}

typed!(/// `mods.json`, parsed once per process.
    mods, "mods", Mods);
typed!(/// `base_items.json`, parsed once per process.
    base_items, "base_items", BaseItems);
typed!(/// `essences.json`.
    essences, "essences", OrderedMap<Essence>);
typed!(/// `fossils.json`.
    fossils, "fossils", OrderedMap<Fossil>);
typed!(/// `crafting_bench_options.json`.
    crafting_bench_options, "crafting_bench_options", Vec<BenchOption>);
typed!(/// `cluster_jewels.json`, by base item id.
    cluster_jewels, "cluster_jewels", OrderedMap<ClusterJewel>);
typed!(/// `cluster_jewel_notables.json`.
    cluster_jewel_notables, "cluster_jewel_notables", Vec<ClusterNotable>);
typed!(/// `stat_translations.json` (ids and trade stats only).
    stat_translations, "stat_translations", Vec<StatTranslation>);
typed!(/// `item_classes.json`.
    item_classes, "item_classes", OrderedMap<ItemClass>);
