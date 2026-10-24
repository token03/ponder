//! Parse Path of Building item text into a structured item.
//!
//! PoB stores each item as text close to the in-game copy format, with extra
//! header keys and per-line markers such as `{crafted}`, `{fractured}`,
//! `{range:0.5}` or `{variant:1,2}`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::LazyLock;

use regex::Regex;

/// Header line of an influenced item -> the influence name.
pub const INFLUENCES: &[(&str, &str)] = &[
    ("Shaper Item", "shaper"),
    ("Elder Item", "elder"),
    ("Crusader Item", "crusader"),
    ("Redeemer Item", "redeemer"),
    ("Hunter Item", "hunter"),
    ("Warlord Item", "warlord"),
];
const FLAGS: &[(&str, &str)] = &[
    ("Corrupted", "corrupted"),
    ("Mirrored", "mirrored"),
    ("Split", "split"),
    ("Synthesised Item", "synthesised"),
    ("Fractured Item", "fractured"),
    ("Searing Exarch Item", "exarch"),
    ("Eater of Worlds Item", "eater"),
];

fn influence(line: &str) -> Option<&'static str> {
    INFLUENCES.iter().find(|(k, _)| *k == line).map(|(_, v)| *v)
}
fn flag(line: &str) -> Option<&'static str> {
    FLAGS.iter().find(|(k, _)| *k == line).map(|(_, v)| *v)
}

static MARKER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\{(\w+)(?::([^}]*))?\}").unwrap());
// PoB colour codes, e.g. in author notes
static COLOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\^(x[0-9a-fA-F]{6}|[0-9])").unwrap());

/// A text that is not an item (Python `ValueError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemError(pub String);

impl fmt::Display for ItemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ItemError {}

/// One mod line of an item. Lines are told apart by their position in the item
/// (Python compared them by identity: an item can repeat a line).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ModLine {
    pub text: String,
    pub markers: BTreeMap<String, String>,
}

impl ModLine {
    pub fn new(text: impl Into<String>) -> Self {
        ModLine {
            text: text.into(),
            markers: BTreeMap::new(),
        }
    }

    /// A line with one marker, e.g. `ModLine::with_marker("+1 to X", "crafted")`.
    pub fn with_marker(text: impl Into<String>, marker: &str) -> Self {
        let mut line = ModLine::new(text);
        line.markers.insert(marker.to_string(), String::new());
        line
    }

    pub fn crafted(&self) -> bool {
        self.markers.contains_key("crafted")
    }

    pub fn fractured(&self) -> bool {
        self.markers.contains_key("fractured")
    }

    /// PoB's position within the roll range (0..1), if it recorded one.
    pub fn roll(&self) -> Option<f64> {
        let value = self.markers.get("range")?;
        if value.is_empty() {
            None
        } else {
            value.parse().ok()
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Item {
    pub rarity: String,
    pub name: String,
    pub base: String,
    pub item_level: Option<i64>,
    pub quality: i64,
    pub influences: BTreeSet<String>,
    pub flags: BTreeSet<String>,
    pub implicits: Vec<ModLine>,
    pub explicits: Vec<ModLine>,
    /// Mod ids PoB's item crafter recorded directly ("Prefix: IncreasedLife7"): (side, id).
    pub pob_affixes: Vec<(String, String)>,
    pub header: BTreeMap<String, String>,
}

impl Item {
    pub fn new(
        rarity: impl Into<String>,
        name: impl Into<String>,
        base: impl Into<String>,
    ) -> Self {
        Item {
            rarity: rarity.into(),
            name: name.into(),
            base: base.into(),
            ..Default::default()
        }
    }
}

// -- Python string semantics ------------------------------------------------------------

/// `str.strip()`: Python also counts the ASCII separators 0x1c-0x1f as whitespace.
pub(crate) fn py_strip(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() || ('\x1c'..='\x1f').contains(&c))
}

/// `str.splitlines()`.
pub(crate) fn py_splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if matches!(
            c,
            '\n' | '\r'
                | '\x0b'
                | '\x0c'
                | '\x1c'
                | '\x1d'
                | '\x1e'
                | '\u{85}'
                | '\u{2028}'
                | '\u{2029}'
        ) {
            out.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' && matches!(chars.peek(), Some((_, '\n'))) {
                chars.next();
                end += 1;
            }
            start = end;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// `int(value)` or None.
fn int(value: Option<&str>) -> Option<i64> {
    py_strip(value?).replace('_', "").parse().ok()
}

// -- parsing --------------------------------------------------------------------------------

/// A PoB item, or an item copied in game (Ctrl+C, or Ctrl+Alt+C with mod tiers).
pub fn parse_item(text: &str) -> Result<Item, ItemError> {
    if text.contains(SEPARATOR) {
        return parse_copied(text);
    }
    let lines: Vec<String> = py_splitlines(py_strip(text))
        .into_iter()
        .map(|l| py_strip(&COLOR.replace_all(l, "")).to_string())
        .filter(|l| !l.is_empty())
        .collect();
    if lines.is_empty() || !lines[0].starts_with("Rarity:") {
        return Err(ItemError(format!(
            "not a PoB item: {:?}",
            &lines[..lines.len().min(1)]
        )));
    }
    let rarity = py_strip(lines[0].split_once(':').unwrap().1).to_uppercase();
    let (name, base, rest): (String, String, &[String]);
    if matches!(rarity.as_str(), "RARE" | "UNIQUE" | "RELIC") {
        let get = |i: usize| {
            lines
                .get(i)
                .cloned()
                .ok_or_else(|| ItemError("item has no name".into()))
        };
        name = get(1)?;
        base = get(2)?;
        rest = lines.get(3..).unwrap_or(&[]);
    } else {
        let n = lines
            .get(1)
            .cloned()
            .ok_or_else(|| ItemError("item has no name".into()))?;
        name = n.clone();
        base = n;
        rest = &lines[2..];
    }
    let mut item = Item::new(rarity, name, base);

    let mut implicit_count: Option<i64> = None;
    let mut i = 0;
    while i < rest.len() {
        let line = rest[i].as_str();
        i += 1;
        if let Some(inf) = influence(line) {
            item.influences.insert(inf.to_string());
        } else if let Some(f) = flag(line) {
            item.flags.insert(f.to_string());
        } else if line.contains(':') && !line.starts_with('{') {
            let (key, value) = line.split_once(':').unwrap();
            let (key, value) = (py_strip(key), py_strip(value));
            if key == "Implicits" {
                implicit_count = Some(
                    py_strip(value)
                        .parse()
                        .map_err(|_| ItemError(format!("invalid literal for int(): {value:?}")))?,
                );
                break;
            }
            item.header.insert(key.to_string(), value.to_string());
            if (key == "Prefix" || key == "Suffix") && value != "None" {
                item.pob_affixes
                    .push((key.to_lowercase(), strip_markers(value).0));
            }
        } else {
            i -= 1; // no Implicits header; the mod lines start here
            break;
        }
    }

    item.item_level = int(item.header.get("Item Level").map(String::as_str));
    item.quality = int(item.header.get("Quality").map(String::as_str)).unwrap_or(0);
    let variant = item.header.get("Selected Variant").cloned();

    let body: Vec<&str> = rest[i..].iter().map(String::as_str).collect();
    for line in &body {
        if let Some(f) = flag(line) {
            item.flags.insert(f.to_string());
        }
    }
    let body: Vec<&str> = body.into_iter().filter(|l| flag(l).is_none()).collect();

    // The implicit count includes lines of every variant, so split before filtering.
    let n = implicit_count.unwrap_or(0);
    let cut = if n >= 0 {
        (n as usize).min(body.len())
    } else {
        (body.len() as i64 + n).max(0) as usize
    };
    item.implicits = mod_lines(&body[..cut], variant.as_deref());
    item.explicits = mod_lines(&body[cut..], variant.as_deref());
    Ok(item)
}

const SEPARATOR: &str = "--------";
static COPIED_MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\s*\((implicit|crafted|fractured|enchant|scourge|crucible)\)$").unwrap()
});
// advanced copy: "105(100-109)"
static COPIED_RANGE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"([0-9]+(?:\.[0-9]+)?)\(-?[0-9]+(?:\.[0-9]+)?-[0-9]+(?:\.[0-9]+)?\)").unwrap()
});
const COPIED_SKIP: &[&str] = &[
    "Requirements:",
    "Sockets:",
    "Item Level:",
    "Quality",
    "Note:",
    "Level:",
    "Str:",
    "Dex:",
    "Int:",
];

/// The game's copy format: sections between "--------" lines.
///
/// The first section names the item; a section of "Key: value" lines is
/// properties or requirements; mod lines carry "(implicit)", "(crafted)" or
/// "(fractured)"; advanced copies (Ctrl+Alt+C) add `{ Prefix Modifier ... }`
/// headers and roll ranges, which are dropped.
fn parse_copied(text: &str) -> Result<Item, ItemError> {
    let sections: Vec<Vec<&str>> = py_strip(text)
        .split(SEPARATOR)
        .map(|part| {
            py_splitlines(py_strip(part))
                .into_iter()
                .map(py_strip)
                .filter(|l| !l.is_empty())
                .collect()
        })
        .collect();
    let head: Vec<&str> = sections[0]
        .iter()
        .copied()
        .filter(|l| !l.starts_with("Item Class:"))
        .collect();
    if head.is_empty() || !head[0].starts_with("Rarity:") {
        return Err(ItemError(format!(
            "not a copied item: {:?}",
            &head[..head.len().min(1)]
        )));
    }
    let rarity = py_strip(head[0].split_once(':').unwrap().1).to_uppercase();
    let names = &head[1..];
    let (name, base) = match (names.first(), names.last()) {
        (Some(a), Some(b)) => (a.to_string(), b.to_string()),
        _ => (String::new(), String::new()),
    };
    let mut item = Item::new(rarity, name, base);
    for section in &sections[1..] {
        for line in section {
            if line.starts_with("Item Level:") {
                item.item_level = int(Some(line.split_once(':').unwrap().1));
            } else if line.starts_with("Quality:") {
                let value = line.split_once(':').unwrap().1.split('(').next().unwrap();
                let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
                item.quality = int(Some(&digits)).unwrap_or(0);
            }
        }
        if section.iter().all(|l| l.contains(':'))
            || COPIED_SKIP.iter().any(|p| section[0].starts_with(p))
        {
            continue; // properties, requirements, sockets, item level, notes
        }
        for &line in section {
            if let Some(inf) = influence(line) {
                item.influences.insert(inf.to_string());
            } else if let Some(f) = flag(line) {
                item.flags.insert(f.to_string());
            } else if line.starts_with('{') {
                continue; // advanced copy: { Prefix Modifier "Virile" (Tier: 1) — Life }
            } else {
                let kind = COPIED_MARKER
                    .captures(line)
                    .map(|c| c.get(1).unwrap().as_str())
                    .unwrap_or("");
                let stripped = COPIED_MARKER.replace(line, "");
                let ranged = COPIED_RANGE.replace_all(&stripped, "$1");
                let line = ranged.split(" — ").next().unwrap().to_string();
                match kind {
                    "implicit" => item.implicits.push(ModLine::new(line)),
                    "" => item.explicits.push(ModLine::new(line)),
                    "crafted" | "fractured" => {
                        item.explicits.push(ModLine::with_marker(line, kind))
                    }
                    _ => {}
                }
            }
        }
    }
    if item.explicits.iter().any(ModLine::fractured) {
        item.flags.insert("fractured".into());
    }
    Ok(item)
}

fn mod_lines(lines: &[&str], variant: Option<&str>) -> Vec<ModLine> {
    let mut out = Vec::with_capacity(lines.len());
    for line in lines {
        let (text, markers) = strip_markers(line);
        if let (Some(v), Some(vm)) = (variant, markers.get("variant"))
            && !v.is_empty()
            && !vm.split(',').any(|x| x == v)
        {
            continue;
        }
        out.push(ModLine { text, markers });
    }
    out
}

fn strip_markers(line: &str) -> (String, BTreeMap<String, String>) {
    let mut markers = BTreeMap::new();
    let mut line = line;
    while let Some(m) = MARKER.captures(line) {
        markers.insert(
            m[1].to_string(),
            m.get(2).map_or("", |g| g.as_str()).to_string(),
        );
        line = &line[m.get(0).unwrap().end()..];
    }
    (py_strip(line).to_string(), markers)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitlines_like_python() {
        assert_eq!(
            py_splitlines("a\nb\r\nc\rd\u{2028}e\n"),
            ["a", "b", "c", "d", "e"]
        );
        assert_eq!(py_splitlines(""), Vec::<&str>::new());
        assert_eq!(py_splitlines("a\n\nb"), ["a", "", "b"]);
    }

    #[test]
    fn strip_like_python() {
        assert_eq!(py_strip(" \t\x1fx\u{a0}\n"), "x");
    }
}
