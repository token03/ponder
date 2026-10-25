use super::*;

pub fn defence_name(tag: &str) -> Option<&'static str> {
    match tag {
        "str" => Some("Armour"),
        "dex" => Some("Evasion"),
        "int" => Some("ES"),
        _ => None,
    }
}
fn defence_tag() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^((?:str|dex|int)(?:_(?:str|dex|int))*)_(armour|shield)$").unwrap())
}

/// Bases that roll exactly the same mods with the same weights: one craft for all of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BaseGroup {
    pub item_class: String,
    /// the base's tags that some mod's spawn weight uses
    pub signature: BTreeSet<String>,
    /// e.g. "Gloves (ES)", "Ring", "Cobalt Jewel"
    pub label: String,
    /// names, best first (see [`group`])
    pub bases: Vec<String>,
}

impl BaseGroup {
    /// The base to craft on and sell: the same mods roll on it, and it sells for more.
    pub fn best(&self) -> &str {
        &self.bases[0]
    }
}

/// Every prefix or suffix that can roll on a base with these tags at this item level, in `mods.json` order.
///
/// Remembered per (domain, tags, level): recombinator planning asks for the same few bases thousands of times.
pub fn pool(db: &ModDB, base_id: &str, tags: &BTreeSet<String>, item_level: i64) -> Vec<Mod> {
    shared_pool(db, base_id, tags, item_level).as_ref().clone()
}

pub fn shared_pool(db: &ModDB, base_id: &str, tags: &BTreeSet<String>, item_level: i64) -> Arc<Vec<Mod>> {
    type Key = (DatabaseKey, String, BTreeSet<String>, i64);
    static POOLS: Memo<Key, Arc<Vec<Mod>>> = Memo::new();
    let key = (DatabaseKey::new(db), db.bases[base_id].domain.clone(), tags.clone(), item_level);
    POOLS.get_or_init(key, || Arc::new(pool_uncached(db, base_id, tags, item_level)))
}

fn pool_uncached(db: &ModDB, base_id: &str, tags: &BTreeSet<String>, item_level: i64) -> Vec<Mod> {
    let domain = &db.bases[base_id].domain;
    let mut out = Vec::new();
    for (mod_id, m) in db.mods.iter() {
        if m.domain != *domain || (m.generation_type != "prefix" && m.generation_type != "suffix") || m.text_or_empty().is_empty() {
            continue;
        }
        if m.required_level > item_level {
            continue;
        }
        let weight = ModDB::generation_weight(m, tags);
        if weight > 0.0 {
            out.push(to_mod(mod_id, m, weight));
        }
    }
    out
}

/// Every tag that some item mod's spawn weight mentions.
pub fn spawn_tags() -> &'static BTreeSet<String> {
    static TAGS: OnceLock<BTreeSet<String>> = OnceLock::new();
    TAGS.get_or_init(|| {
        loaded(data::mods())
            .values()
            .filter(|m| {
                matches!(m.domain.as_str(), "item" | "misc" | "abyss_jewel") && matches!(m.generation_type.as_str(), "prefix" | "suffix")
            })
            .flat_map(|m| m.spawn_weights.iter().chain(&m.generation_weights).map(|w| w.tag.clone()))
            .collect()
    })
}

type GroupKey = (DatabaseKey, String, BTreeSet<String>, BTreeSet<String>, String);

/// The bases sharing this item's mod pool: same class, same spawn-relevant tags (influences included).
pub fn group(db: &ModDB, m: &ItemMatch) -> Arc<BaseGroup> {
    static CACHE: Memo<GroupKey, Arc<BaseGroup>> = Memo::new();
    let used = spawn_tags();
    let signature: BTreeSet<String> = m.tags.intersection(used).cloned().collect();
    let key = (
        DatabaseKey::new(db),
        m.item_class.clone().unwrap_or_default(),
        signature,
        m.item.influences.clone(),
        base_category(db, m.base_id.as_deref().expect("a known base")),
    );
    let made = || Arc::new(make_group(db, m, &key.2, &key.3));
    CACHE.get_or_init(key.clone(), made)
}

fn make_group(db: &ModDB, m: &ItemMatch, signature: &BTreeSet<String>, influences: &BTreeSet<String>) -> BaseGroup {
    let used = spawn_tags();
    let item_class = m.item_class.clone().unwrap_or_default();
    let base_id = m.base_id.as_deref().expect("group needs a known base");
    let domain = &db.bases[base_id].domain; // jewels and abyss jewels have their own
    let mut ranked: Vec<(bool, bool, i64, String)> = Vec::new();
    for (id, base) in db.bases.iter() {
        if base.item_class != item_class
            || base.domain != *domain
            || base.release_state != "released"
            || base_category(db, id) != base_category(db, base_id)
        {
            continue;
        }
        let tags: BTreeSet<String> = db.item_tags(id, influences).intersection(used).cloned().collect();
        if tags == *signature {
            let has = |t: &str| base.tags.iter().any(|x| x == t);
            // Best first: real drops over experimental bases, end-game bases, then highest drop level.
            ranked.push((!has("experimental_base"), has("top_tier_base_item_type"), base.drop_level, base.name.clone()));
        }
    }
    ranked.sort_by(|a, b| b.cmp(a));
    let mut names: Vec<String> = Vec::new();
    for (.., n) in ranked {
        if !names.contains(&n) {
            names.push(n);
        }
    }
    if names.is_empty() {
        names.push(m.item.base.clone());
    }
    let mut label = if names.len() == 1 {
        names[0].clone()
    } else {
        match signature.iter().find(|t| defence_tag().is_match(t)) {
            Some(t) => format!("{item_class} ({})", defence(t)),
            None => item_class.clone(),
        }
    };
    if !influences.is_empty() {
        let mut titled: Vec<String> = influences.iter().map(|i| title(i)).collect();
        titled.sort();
        label += &format!(" · {}", titled.join("/"));
    }
    BaseGroup { item_class, signature: signature.clone(), label, bases: names }
}

/// Python's `str.title()`.
fn title(s: &str) -> String {
    let mut out = String::new();
    let mut prev_letter = false;
    for c in s.chars() {
        if c.is_alphabetic() {
            if prev_letter {
                out.extend(c.to_lowercase());
            } else {
                out.extend(c.to_uppercase());
            }
            prev_letter = true;
        } else {
            out.push(c);
            prev_letter = false;
        }
    }
    out
}

fn defence(tag: &str) -> String {
    let caps = defence_tag().captures(tag).expect("a defence tag");
    caps[1].split('_').map(|p| defence_name(p).unwrap_or("?")).collect::<Vec<_>>().join("/")
}

pub fn base_limits(db: &ModDB, base_id: &str) -> [usize; 2] {
    let base = &db.bases[base_id];
    let initial = if JEWEL_CLASSES.contains(&base.item_class.as_str()) { 2 } else { 3 };
    let mut limits = [initial; 2];
    for id in &base.implicits {
        if let Some(m) = db.mods.get(id) {
            for stat in &m.stats {
                let side = match stat.id.as_str() {
                    "local_maximum_prefixes_allowed_+" => 0,
                    "local_maximum_suffixes_allowed_+" => 1,
                    _ => continue,
                };
                limits[side] = (limits[side] as i64 + stat.min as i64).clamp(0, 6) as usize;
            }
        }
    }
    limits
}

fn base_category(db: &ModDB, base_id: &str) -> String {
    let base = &db.bases[base_id];
    if matches!(base.item_class.as_str(), "Ring" | "Amulet" | "Belt") && base.tags.iter().any(|t| t == "experimental_base") {
        base.name.clone()
    } else {
        base.item_class.clone()
    }
}
