//! Decode Path of Building export codes and pull out the items.
//!
//! A PoB code is URL-safe base64 of zlib-compressed XML. Share sites (pobb.in,
//! pastebin) serve the same code at a `/raw` URL; downloading it is the net
//! crate's job (see [`raw_url`] and [`load_with`]).

use std::fmt;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::LazyLock;

use base64::Engine;
use base64::alphabet;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use regex::Regex;
use roxmltree::{Document, Node};

use crate::item::{py_splitlines, py_strip};

/// Share-site URL patterns and the raw-code URL each maps to (`{}` is the captured id).
static RAW_URLS: LazyLock<[(Regex, &'static str); 2]> = LazyLock::new(|| {
    [
        (Regex::new(r"^https?://pobb\.in/([\w-]+)/?$").unwrap(), "https://pobb.in/{}/raw"),
        (Regex::new(r"^https?://pastebin\.com/(?:raw/)?(\w+)/?$").unwrap(), "https://pastebin.com/raw/{}"),
    ]
});

static COLOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\^(x[0-9a-fA-F]{6}|[0-9])").unwrap());

/// Anything that stops a build from loading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PobError(pub String);

impl fmt::Display for PobError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for PobError {}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PobItem {
    pub id: String,
    pub text: String,
    /// slot in the active item set, else the first set using it
    pub slot: Option<String>,
    /// titles of item sets using it, e.g. "Early game"
    pub sets: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Build {
    pub class_name: String,
    pub ascendancy: String,
    pub level: i64,
    pub main_skill: Option<String>,
    pub items: Vec<PobItem>,
}

fn engine() -> GeneralPurpose {
    GeneralPurpose::new(
        &alphabet::STANDARD,
        GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent).with_decode_allow_trailing_bits(true),
    )
}

/// Turn a PoB export code into its XML string.
pub fn decode(code: &str) -> Result<String, PobError> {
    // Python's b64decode drops characters outside the alphabet; so do we.
    let clean: String = code
        .chars()
        .map(|c| match c {
            '-' => '+',
            '_' => '/',
            c => c,
        })
        .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/')
        .collect();
    let raw = engine().decode(clean).map_err(|e| PobError(format!("bad PoB code: {e}")))?;
    let mut xml = Vec::new();
    flate2::read::ZlibDecoder::new(raw.as_slice()).read_to_end(&mut xml).map_err(|e| PobError(format!("bad PoB code: {e}")))?;
    String::from_utf8(xml).map_err(|e| PobError(format!("bad PoB code: {e}")))
}

/// Inverse of [`decode`], producing a code PoB can import.
pub fn encode(xml: &str) -> String {
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(xml.as_bytes()).unwrap();
    let raw = z.finish().unwrap();
    base64::engine::general_purpose::STANDARD.encode(raw).replace('+', "-").replace('/', "_")
}

/// The raw-code URL for a share-site link (pobb.in, pastebin), if `source` is one.
///
/// NETWORK: the caller downloads it; this crate does no I/O of that kind.
pub fn raw_url(source: &str) -> Option<String> {
    let source = py_strip(source);
    RAW_URLS.iter().find_map(|(re, raw)| re.captures(source).map(|m| raw.replace("{}", &m[1])))
}

/// A PoB code from a file path, or the code itself (share URLs go through [`raw_url`] first).
pub fn read_code(source: &str) -> String {
    let path = Path::new(source);
    if !source.is_empty() && path.is_file() {
        if let Ok(text) = std::fs::read_to_string(path) {
            return py_strip(&text).to_string();
        }
    }
    py_strip(source).to_string()
}

/// A build from text already in hand: PoB XML, or an export code (what a download of a share URL holds).
pub fn load_text(code_or_xml: &str) -> Result<Build, PobError> {
    let text = py_strip(code_or_xml);
    if text.starts_with('<') { parse(text) } else { parse(&decode(text)?) }
}

/// Load a build from a share URL, a file holding a code or XML, or a code.
///
/// `fetch` downloads a URL's body (the net crate supplies it); it is called only for share URLs.
pub fn load_with(source: &str, fetch: impl Fn(&str) -> Result<String, String>) -> Result<Build, PobError> {
    let text = match raw_url(source) {
        Some(url) => py_strip(&fetch(&url).map_err(PobError)?).to_string(),
        None => read_code(source),
    };
    load_text(&text)
}

fn children<'a, 'b>(node: Node<'a, 'b>, tag: &'static str) -> impl Iterator<Item = Node<'a, 'b>> {
    node.children().filter(move |c| c.is_element() && c.tag_name().name() == tag)
}

fn child<'a, 'b>(node: Node<'a, 'b>, tag: &'static str) -> Option<Node<'a, 'b>> {
    children(node, tag).next()
}

/// Read the build, its items (with the sets and slots that use them) and main skill from PoB XML.
pub fn parse(xml: &str) -> Result<Build, PobError> {
    let doc = Document::parse(xml).map_err(|e| PobError(format!("bad PoB XML: {e}")))?;
    let root = doc.root_element();
    let build_el = child(root, "Build");
    let attr = |name: &str| build_el.and_then(|b| b.attribute(name));
    let level_text = attr("level").unwrap_or("0");
    let mut build = Build {
        class_name: attr("className").unwrap_or("").to_string(),
        ascendancy: attr("ascendClassName").unwrap_or("").replace("None", ""),
        level: if level_text.is_empty() {
            0
        } else {
            py_strip(level_text).parse().map_err(|_| PobError(format!("bad level {level_text:?}")))?
        },
        main_skill: main_skill(root, attr("mainSocketGroup")),
        items: Vec::new(),
    };

    let Some(items_el) = child(root, "Items") else { return Ok(build) };
    let usage = item_usage(items_el);
    for item_el in children(items_el, "Item") {
        let id = item_el.attribute("id").unwrap_or("").to_string();
        let raw: String = item_el.descendants().filter(|n| n.is_text()).map(|n| n.text().unwrap_or("")).collect();
        let text = py_splitlines(py_strip(&raw)).into_iter().map(py_strip).collect::<Vec<_>>().join("\n");
        let uses = usage.iter().find(|(k, _)| *k == id).map(|(_, v)| v.as_slice()).unwrap_or(&[]);
        let slot = uses.iter().find(|u| u.2).map(|u| u.1.clone()).or_else(|| uses.first().map(|u| u.1.clone()));
        let mut sets: Vec<String> = Vec::new();
        for (title, _, _) in uses {
            if !title.is_empty() && !sets.contains(title) {
                sets.push(title.clone());
            }
        }
        build.items.push(PobItem { id, text, slot, sets });
    }
    Ok(build)
}

/// (item set title, slot, is active set) for one item.
type Use = (String, String, bool);

/// Item id -> every set slot equipping it, in document order.
fn item_usage(items_el: Node) -> Vec<(String, Vec<Use>)> {
    let active = items_el.attribute("activeItemSet");
    let mut sets: Vec<(String, bool, Node)> = children(items_el, "ItemSet")
        .map(|s| {
            let title = match s.attribute("title") {
                Some(t) if !t.is_empty() => t.to_string(),
                _ => format!("Set {}", s.attribute("id").unwrap_or("None")),
            };
            (title, s.attribute("id") == active, s)
        })
        .collect();
    if sets.is_empty() {
        // older exports keep slots at top level
        sets = vec![(String::new(), true, items_el)];
    }
    let mut usage: Vec<(String, Vec<Use>)> = Vec::new();
    for (title, is_active, el) in sets {
        for slot in children(el, "Slot") {
            let name = slot.attribute("name").unwrap_or("");
            let Some(item_id) = slot.attribute("itemId") else { continue };
            if item_id == "0" || name.contains("Swap") {
                continue;
            }
            let entry = (py_strip(&COLOR.replace_all(&title, "")).to_string(), name.to_string(), is_active);
            match usage.iter_mut().find(|(k, _)| k == item_id) {
                Some((_, v)) => v.push(entry),
                None => usage.push((item_id.to_string(), vec![entry])),
            }
        }
    }
    usage
}

/// The build's main skill: the first non-support gem of the main socket group.
fn main_skill(root: Node, main_group: Option<&str>) -> Option<String> {
    let skills = child(root, "Skills")?;
    let active = skills.attribute("activeSkillSet");
    let skill_sets: Vec<Node> = children(skills, "SkillSet").collect();
    let container = skill_sets
        .iter()
        .find(|s| s.attribute("id") == active)
        .or_else(|| skill_sets.first())
        .copied()
        .unwrap_or(skills);
    let groups: Vec<Node> = children(container, "Skill").collect();
    // Python indexes groups[int(group) - 1]: a negative index counts from the end.
    let mut i: i64 = py_strip(main_group?).parse::<i64>().ok()? - 1;
    if i < 0 {
        i += groups.len() as i64;
    }
    let group = groups.get(usize::try_from(i).ok()?)?;
    let gems: Vec<Node> = children(*group, "Gem").collect();
    let gem = gems.iter().find(|g| !g.attribute("skillId").unwrap_or("").contains("Support")).or_else(|| gems.first())?;
    gem.attribute("nameSpec").map(String::from)
}
