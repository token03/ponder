//! Raw RePoE exports, as pinned in `fixtures/repoe`. Only the fields the rules read.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use serde::Deserialize;
use serde::de::DeserializeOwned;

#[derive(Debug, Clone, Deserialize)]
pub struct Weight {
    pub tag: String,
    pub weight: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Mod {
    pub domain: String,
    pub generation_type: String,
    #[serde(default)]
    pub groups: Vec<String>,
    #[serde(default)]
    pub implicit_tags: Vec<String>,
    #[serde(default)]
    pub is_essence_only: bool,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub required_level: u32,
    #[serde(default)]
    pub spawn_weights: Vec<Weight>,
    #[serde(default)]
    pub generation_weights: Vec<Weight>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(rename = "type", default)]
    pub family: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Base {
    pub domain: String,
    pub item_class: String,
    pub name: String,
    pub release_state: String,
    #[serde(default)]
    pub implicits: Vec<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// A cluster jewel size and the small passives its enchant can grant.
#[derive(Debug, Clone, Deserialize)]
pub struct ClusterJewel {
    pub name: String,
    pub passive_skills: Vec<ClusterPassive>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ClusterPassive {
    pub tag: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Essence {
    pub name: String,
    pub level: u32,
    /// Item class -> forced mod id.
    #[serde(default)]
    pub mods: BTreeMap<String, Option<String>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Fossil {
    pub name: String,
    #[serde(default)]
    pub positive_mod_weights: Vec<Weight>,
    #[serde(default)]
    pub negative_mod_weights: Vec<Weight>,
    #[serde(default)]
    pub added_mods: Vec<String>,
    #[serde(default)]
    pub forced_mods: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BenchActions {
    #[serde(default)]
    pub add_explicit_mod: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BenchOption {
    pub actions: BenchActions,
    #[serde(default)]
    pub item_classes: Vec<String>,
    /// Currency metadata id -> amount.
    #[serde(default)]
    pub cost: BTreeMap<String, u32>,
}

/// Every export the rules read, keyed by RePoE id.
pub struct Repoe {
    pub mods: BTreeMap<String, Mod>,
    pub bases: BTreeMap<String, Base>,
    pub essences: BTreeMap<String, Essence>,
    pub fossils: BTreeMap<String, Fossil>,
    pub bench: Vec<BenchOption>,
    pub clusters: BTreeMap<String, ClusterJewel>,
}

impl Repoe {
    pub fn load(dir: &Path) -> std::io::Result<Repoe> {
        Ok(Repoe {
            mods: read(dir, "mods")?,
            bases: read(dir, "base_items")?,
            essences: read(dir, "essences")?,
            fossils: read(dir, "fossils")?,
            bench: read(dir, "crafting_bench_options")?,
            clusters: read(dir, "cluster_jewels")?,
        })
    }

    /// A released base by display name.
    pub fn base(&self, name: &str) -> Option<&Base> {
        self.bases.values().find(|b| b.name == name && b.release_state == "released")
    }
}

fn read<T: DeserializeOwned>(dir: &Path, name: &str) -> std::io::Result<T> {
    let path = dir.join(format!("{name}.json.gz"));
    let mut json = String::new();
    flate2::read::GzDecoder::new(std::fs::File::open(&path)?).read_to_string(&mut json)?;
    serde_json::from_str(&json).map_err(|e| std::io::Error::other(format!("{}: {e}", path.display())))
}

/// A mod's text with every number or `(a-b)` range replaced by `#`, lines joined by " / ".
/// Guides and goals name mods this way.
pub fn template(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push_str(" / ");
        }
        let b = line.as_bytes();
        let mut j = 0;
        while j < b.len() {
            let end = match b[j] {
                b'(' => range_end(b, j + 1).filter(|&e| b.get(e) == Some(&b')')).map(|e| e + 1),
                b'-' => number_end(b, j + 1),
                _ => number_end(b, j),
            };
            match end {
                Some(e) => {
                    out.push('#');
                    j = e;
                }
                None => {
                    let c = line[j..].chars().next().unwrap();
                    out.push(c);
                    j += c.len_utf8();
                }
            }
        }
    }
    out
}

/// End of `n` or `n-m` starting at `j` (numbers may be signed or decimal).
fn range_end(b: &[u8], j: usize) -> Option<usize> {
    let e = number_end(b, j).or_else(|| (b.get(j) == Some(&b'-')).then(|| number_end(b, j + 1)).flatten())?;
    if b.get(e) == Some(&b'-') {
        let m = if b.get(e + 1) == Some(&b'-') { e + 2 } else { e + 1 };
        return number_end(b, m).or(Some(e));
    }
    Some(e)
}

fn number_end(b: &[u8], j: usize) -> Option<usize> {
    let digits = |k: usize| b[k..].iter().take_while(|c| c.is_ascii_digit()).count();
    let n = digits(j);
    if n == 0 {
        return None;
    }
    let e = j + n;
    if b.get(e) == Some(&b'.') && b.get(e + 1).is_some_and(u8::is_ascii_digit) { Some(e + 1 + digits(e + 1)) } else { Some(e) }
}

#[cfg(test)]
mod tests {
    use super::template;

    #[test]
    fn templates() {
        assert_eq!(template("+(8-12) to Strength"), "+# to Strength");
        assert_eq!(template("Adds (1-2) to (3-4) Fire Damage"), "Adds # to # Fire Damage");
        assert_eq!(template("-(8-6) to Total Mana Cost"), "-# to Total Mana Cost");
        assert_eq!(template("Nearby Enemies have -9% to Chaos Resistance"), "Nearby Enemies have #% to Chaos Resistance");
        assert_eq!(template("(0.2-0.4)% of Life\n+1 to Level"), "#% of Life / +# to Level");
    }
}
