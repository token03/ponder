//! Find mod combinations that recur across many builds' rare items.
//!
//! A candidate is an item class plus a set of mod families (e.g. Helmet with
//! IncreasedLife + FireResistance + ColdResistance). Tiers are ignored here; the
//! crafting solver decides later what tier is worth settling for.

use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use ponder_game::mods::{ItemMatch, ModMatch};

pub const NOTABLE_PREFIX: &str = "1 Added Passive Skill is ";

#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub item_class: String,
    pub families: Vec<String>,
    /// rare items containing all of these families
    pub count: usize,
    /// distinct builds those items came from
    pub builds: usize,
}

/// `itertools.combinations(items, size)`, in lexicographic index order.
pub fn combinations<T>(items: &[T], size: usize) -> Vec<Vec<&T>> {
    let n = items.len();
    let mut out = Vec::new();
    if size > n {
        return out;
    }
    let mut idx: Vec<usize> = (0..size).collect();
    loop {
        out.push(idx.iter().map(|&i| &items[i]).collect());
        let mut i = size;
        loop {
            if i == 0 {
                return out;
            }
            i -= 1;
            if idx[i] != i + n - size {
                break;
            }
            if i == 0 {
                return out;
            }
        }
        idx[i] += 1;
        for j in i + 1..size {
            idx[j] = idx[j - 1] + 1;
        }
    }
}

type Key = (String, Vec<String>);

/// Count family subsets per item class.
///
/// `matches` pairs each matched rare with an id for the build it came from.
/// Rares have at most six explicit affixes, so enumerating subsets is cheap.
/// With `closed`, a combo is dropped when a larger combo containing it is
/// shared by exactly the same builds, since it adds nothing new.
pub fn mine(
    matches: &[(String, ItemMatch)],
    min_size: usize,
    max_size: usize,
    min_builds: usize,
    closed: bool,
) -> Vec<Candidate> {
    let mut counts: IndexMap<Key, usize> = IndexMap::new();
    let mut builds: HashMap<Key, HashSet<&str>> = HashMap::new();
    for (build_id, m) in matches {
        let Some(_) = m.item_class else { continue };
        if m.item.rarity != "RARE" {
            continue;
        }
        let (item_class, families) = target_key(m);
        for size in min_size..=max_size.min(families.len()) {
            for combo in combinations(&families, size) {
                let key = (
                    item_class.clone(),
                    combo.into_iter().cloned().collect::<Vec<_>>(),
                );
                builds.entry(key.clone()).or_default().insert(build_id);
                *counts.entry(key).or_insert(0) += 1;
            }
        }
    }

    let mut keys: Vec<&Key> = counts
        .keys()
        .filter(|k| builds[*k].len() >= min_builds)
        .collect();
    if closed {
        // Other combos by class, so the absorbed check only scans its own class.
        let mut by_class: HashMap<&str, Vec<&Key>> = HashMap::new();
        for k in &keys {
            by_class.entry(k.0.as_str()).or_default().push(k);
        }
        keys.retain(|key| !absorbed(key, &by_class[key.0.as_str()], &builds));
    }
    let mut out: Vec<Candidate> = keys
        .into_iter()
        .map(|key| Candidate {
            item_class: key.0.clone(),
            families: key.1.clone(),
            count: counts[key],
            builds: builds[key].len(),
        })
        .collect();
    // Prefer combos many builds share, then bigger combos (more specific targets).
    out.sort_by(|a, b| {
        b.builds
            .cmp(&a.builds)
            .then(b.families.len().cmp(&a.families.len()))
            .then(b.count.cmp(&a.count))
            .then_with(|| a.item_class.cmp(&b.item_class))
            .then_with(|| a.families.cmp(&b.families))
    });
    out
}

/// What a rare is "made of" for mining purposes.
///
/// Cluster jewels are keyed by size and enchant, and only their notables
/// count, since the other mods barely matter to buyers.
pub fn target_key(m: &ItemMatch) -> (String, Vec<String>) {
    let enchant = m
        .item
        .header
        .get("Cluster Jewel Skill")
        .filter(|e| !e.is_empty());
    if let Some(enchant) = enchant
        && m.item.base.ends_with("Cluster Jewel")
    {
        let size = m
            .item
            .base
            .split_whitespace()
            .next()
            .unwrap_or("")
            .to_lowercase();
        let mut notables: Vec<String> = m
            .mods
            .iter()
            .filter(|x| is_notable_line(x))
            .map(|x| {
                x.lines[0]
                    .text
                    .strip_prefix(NOTABLE_PREFIX)
                    .unwrap_or(&x.lines[0].text)
                    .to_string()
            })
            .collect();
        notables.sort();
        return (format!("{size} cluster: {enchant}"), notables);
    }
    let mut families: Vec<String> = m
        .mods
        .iter()
        .filter(|x| !x.source.ends_with("crafted"))
        .map(|x| x.family.clone())
        .collect();
    families.sort();
    families.dedup();
    (m.item_class.clone().unwrap_or_default(), families)
}

pub fn is_notable_line(m: &ModMatch) -> bool {
    let text = &m.lines[0].text;
    text.starts_with(NOTABLE_PREFIX) && !text.contains("Jewel Socket")
}

fn absorbed(key: &Key, same_class: &[&Key], builds: &HashMap<Key, HashSet<&str>>) -> bool {
    let (_, combo) = key;
    same_class.iter().any(|other| {
        other.1.len() > combo.len()
            && combo.iter().all(|f| other.1.contains(f))
            && builds[*other] == builds[key]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ponder_game::item::Item;

    fn rare(item_class: &str, families: &[&str], crafted: &[&str]) -> ItemMatch {
        let mk = |f: &str, kind: &str, source: &'static str, tier| ModMatch {
            mod_id: f.into(),
            kind: kind.into(),
            family: f.into(),
            source,
            tier,
            lines: vec![],
            line_ids: vec![],
            alternatives: 0,
            merged: false,
        };
        let mut mods: Vec<ModMatch> = families
            .iter()
            .map(|f| mk(f, "prefix", "natural", Some(1)))
            .collect();
        mods.extend(crafted.iter().map(|f| mk(f, "suffix", "crafted", None)));
        ItemMatch {
            item: Item::new("RARE", "x", "y"),
            base_id: Some("b".into()),
            item_class: Some(item_class.into()),
            mods,
            ..Default::default()
        }
    }

    fn mined(matches: &[(&str, ItemMatch)], closed: bool) -> Vec<Candidate> {
        let owned: Vec<(String, ItemMatch)> = matches
            .iter()
            .map(|(b, m)| (b.to_string(), m.clone()))
            .collect();
        mine(&owned, 2, 4, 2, closed)
    }

    #[test]
    fn shared_combos_ranked_by_builds() {
        let matches = [
            ("a", rare("Helmet", &["Life", "FireRes", "ColdRes"], &[])),
            ("b", rare("Helmet", &["Life", "FireRes", "ChaosRes"], &[])),
            (
                "c",
                rare("Helmet", &["Life", "FireRes", "ColdRes"], &["ChaosRes"]),
            ),
            ("c", rare("Amulet", &["Life", "Crit"], &[])),
        ];
        let out = mined(&matches, true);
        let top = &out[0];
        assert_eq!(
            (top.item_class.as_str(), top.families.clone(), top.builds),
            ("Helmet", vec!["FireRes".to_string(), "Life".to_string()], 3)
        );
        assert!(
            out.iter()
                .any(|c| c.item_class == "Helmet" && c.families == ["ColdRes", "FireRes", "Life"])
        );
        // Crafted mods don't count, and single-build combos are dropped.
        assert!(
            out.iter()
                .all(|c| !c.families.contains(&"ChaosRes".to_string()) || c.builds >= 2)
        );
        assert!(!out.iter().any(|c| c.item_class == "Amulet"));
    }

    #[test]
    fn closed_drops_redundant_subsets() {
        let matches = [
            ("a", rare("Ring", &["A", "B", "C"], &[])),
            ("b", rare("Ring", &["A", "B", "C"], &[])),
        ];
        let out = mined(&matches, true);
        assert_eq!(
            out.iter().map(|c| c.families.clone()).collect::<Vec<_>>(),
            vec![vec!["A", "B", "C"]]
        );
        assert_eq!(mined(&matches, false).len(), 4);
    }

    #[test]
    fn combinations_match_itertools() {
        let v = [1, 2, 3, 4];
        let got: Vec<Vec<i32>> = combinations(&v, 2)
            .into_iter()
            .map(|c| c.into_iter().copied().collect())
            .collect();
        assert_eq!(
            got,
            vec![
                vec![1, 2],
                vec![1, 3],
                vec![1, 4],
                vec![2, 3],
                vec![2, 4],
                vec![3, 4]
            ]
        );
        assert_eq!(combinations(&v, 4).len(), 1);
        assert!(combinations(&v, 5).is_empty());
        assert_eq!(combinations(&v, 0).len(), 1);
    }
}
