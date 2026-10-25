use std::collections::{BTreeMap, BTreeSet};

use ponder_net::trade::{self, ItemQuery};
use serde_json::{Value, json};

use super::Context;

pub fn donor_query(
    cx: &Context,
    base: &str,
    wanted: &[BTreeSet<String>],
    item_level: Option<i64>,
    min_mods: usize,
    max_mods: usize,
) -> Option<Value> {
    donor_query_influenced(cx, base, wanted, item_level, min_mods, max_mods, &BTreeSet::new())
}

pub fn donor_query_influenced(
    cx: &Context,
    base: &str,
    wanted: &[BTreeSet<String>],
    item_level: Option<i64>,
    min_mods: usize,
    max_mods: usize,
    required_influences: &BTreeSet<String>,
) -> Option<Value> {
    if wanted.is_empty() || wanted.len() > 6 || wanted.iter().any(BTreeSet::is_empty) {
        return None;
    }
    let mut query = trade::item_query(&ItemQuery { base: Some(base.into()), item_level, corrupted: Some(false), ..Default::default() });
    let groups = query["query"]["stats"].as_array_mut()?;
    for good in wanted {
        let mut by_stat: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for id in good {
            if let Some(stat) = cx.stat_id(id, "explicit") {
                by_stat.entry(stat).or_default().push(id);
            }
        }
        if by_stat.is_empty() {
            return None;
        }
        let filters: Vec<_> = by_stat
            .into_iter()
            .map(|(id, mods)| {
                let value = cx.stat_min(&mods).map_or_else(|| json!({}), |min| json!({"min": min}));
                json!({"id": id, "value": value, "disabled": false})
            })
            .collect();
        groups.push(json!({"type": "count", "value": {"min": 1}, "filters": filters}));
    }
    let influences = ["shaper", "elder", "crusader", "redeemer", "hunter", "warlord"];
    let excluded: Vec<_> = influences
        .iter()
        .filter(|name| !required_influences.contains(**name))
        .map(|name| json!({"id": format!("pseudo.pseudo_has_{name}_influence"), "disabled": false}))
        .collect();
    groups.push(json!({"type": "not", "filters": excluded}));
    for name in required_influences {
        groups.push(json!({"type": "and", "filters": [{"id": format!("pseudo.pseudo_has_{name}_influence"), "disabled": false}]}));
    }
    groups[0]["filters"] = json!([
        {"id": "pseudo.pseudo_number_of_affix_mods", "value": {"min": min_mods.max(wanted.len()).clamp(1, 6), "max": max_mods.max(min_mods).max(wanted.len()).clamp(1, 6)}, "disabled": false},
        {"id": "pseudo.pseudo_number_of_crafted_mods", "value": {"max": 0}, "disabled": false}
    ]);
    for filter in ["fractured_item", "mirrored", "synthesised_item"] {
        query["query"]["filters"]["misc_filters"]["filters"][filter] = json!({"option": "false"});
    }
    Some(query)
}

pub fn donor_mod_range(query: &Value, min_mods: usize, max_mods: usize) -> Value {
    let mut query = query.clone();
    let required = query["query"]["stats"].as_array().map_or(1, |groups| groups.iter().filter(|g| g["type"] == "count").count().max(1));
    if let Some(filters) = query["query"]["stats"][0]["filters"].as_array_mut()
        && let Some(filter) = filters.iter_mut().find(|f| f["id"] == "pseudo.pseudo_number_of_affix_mods")
    {
        filter["value"] = json!({"min": min_mods.max(required).clamp(1, 6), "max": max_mods.max(min_mods).max(required).clamp(1, 6)});
    }
    query
}
