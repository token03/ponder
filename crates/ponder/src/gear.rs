use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, OnceLock};

use anyhow::{Result, anyhow, ensure};
use ponder_game::{
    data,
    mods::{ItemMatch, ModDB},
};
use regex::Regex;

use crate::candidate::{Candidate, Need};
use crate::memo::{DatabaseKey, Memo};
pub use ponder_rules::game::Side;

mod bases;
pub use bases::{BaseGroup, base_limits, defence_name, group, pool, shared_pool, spawn_tags};

pub const JEWEL_CLASSES: [&str; 2] = ["Jewel", "AbyssJewel"];
pub const DEFAULT_ITEM_LEVEL: i64 = 84;
pub const FILLER_FAMILIES: [&str; 3] = ["ColdResistance", "FireResistance", "LightningResistance"];
pub const FILLER_TIER: i64 = 3;

pub type Prices = HashMap<String, f64>;

#[derive(Clone, Copy, Debug)]
pub struct Settings {
    pub chaos_per_hour: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Self { chaos_per_hour: 1500.0 }
    }
}

#[derive(Clone, Debug)]
pub struct PoolMod {
    pub id: String,
    pub weight: f64,
    pub groups: Vec<String>,
    pub notable: bool,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub role: &'static str,
    pub side: Side,
    pub text: String,
    pub tier: String,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct BuildOpts {
    pub settle: i64,
    pub item_level: Option<i64>,
    pub exact: bool,
}

#[derive(Clone, Debug)]
pub struct Prepared {
    pub candidate: Candidate,
    pub rows: Vec<Row>,
}

pub fn is_filler_family(family: &str) -> bool {
    FILLER_FAMILIES.contains(&family)
}

pub(crate) fn loaded<T>(value: Result<Arc<T>, data::DataError>) -> Arc<T> {
    value.unwrap_or_else(|error| panic!("{error}"))
}

fn side_of(kind: &str) -> Result<Side> {
    match kind {
        "prefix" => Ok(Side::Prefix),
        "suffix" => Ok(Side::Suffix),
        _ => Err(anyhow!("{kind:?} is not an affix")),
    }
}

pub fn build(game: &ponder_rules::game::Game, db: &ModDB, matched: &ItemMatch, opts: BuildOpts) -> Result<Prepared> {
    use ponder_rules::goal::{Goal, Influence, ModReq, Source};
    use ponder_rules::target::Target;

    let base = matched.base_id.as_deref().ok_or_else(|| anyhow!("unknown base {:?}", matched.item.base))?;
    ensure!(
        !matched.item.flags.contains("corrupted") && !matched.item.flags.contains("mirrored"),
        "corrupted or mirrored items cannot be crafted"
    );
    ensure!(!matched.item.flags.contains("synthesised"), "synthesised bases are not represented by the crafting planner");
    ensure!(matched.unmatched.is_empty(), "item has unrecognized modifiers");
    let mut level = opts.item_level.or(matched.item.item_level).unwrap_or(DEFAULT_ITEM_LEVEL);
    if opts.item_level.is_none() {
        level = matched.mods.iter().filter_map(|m| db.mods.get(&m.mod_id)).map(|m| m.required_level).fold(level, i64::max);
    }
    let influences = matched
        .item
        .influences
        .iter()
        .map(|name| serde_json::from_value::<Influence>(name.clone().into()))
        .collect::<Result<Vec<_>, _>>()?;
    let mut goal =
        Goal { base: Some(group(db, matched).best().into()), ilvl: Some(u8::try_from(level)?), influences, ..Default::default() };
    let mut fixed = Vec::new();
    let mut rows = Vec::new();
    let mut names = Vec::new();
    let mut filler = 0;
    for found in &matched.mods {
        let side = side_of(&found.kind)?;
        let text = found.lines.iter().map(|line| line.text.as_str()).collect::<Vec<_>>().join(" / ");
        if !opts.exact && !found.fractured() && is_filler_family(&found.family) && matches!(found.source, "natural" | "essence") {
            filler += 1;
            rows.push(Row { role: "filler", side, text, tier: format!("T{FILLER_TIER}+") });
            continue;
        }
        let id = game.mod_id(&found.mod_id).ok_or_else(|| anyhow!("unsupported modifier {}", found.mod_id))?;
        let def = game.def(id);
        let source = match found.source {
            "crafted" => Some(Source::Crafted),
            "veiled" => Some(Source::Veiled),
            _ => match def.source {
                Source::Natural | Source::Influence | Source::Essence => None,
                source => Some(source),
            },
        };
        let tier = if source.is_some() { None } else { Some(found.tier.unwrap_or(2).saturating_add(opts.settle).clamp(1, 255) as u8) };
        if found.fractured() {
            fixed.push(found.mod_id.clone());
        }
        goal.mods.push(ModReq { text: vec![def.template.clone()], tier, count: 1, fractured: found.fractured(), source });
        names.push(text.clone());
        rows.push(Row {
            role: if found.fractured() {
                "fractured"
            } else if found.source == "crafted" {
                "bench"
            } else {
                "target"
            },
            side,
            text,
            tier: tier.map(|tier| format!("T{tier}+")).unwrap_or_default(),
        });
    }
    if filler > 0 {
        let mut templates = BTreeSet::new();
        for (id, def) in db.mods.iter().filter(|(_, m)| is_filler_family(&m.type_) && m.domain == db.bases[base].domain) {
            if ModDB::generation_weight(def, &matched.tags) > 0.0
                && let Some(id) = game.mod_id(id)
            {
                templates.insert(game.def(id).template.clone());
            }
        }
        goal.mods.push(ModReq {
            text: templates.into_iter().collect(),
            tier: Some(FILLER_TIER as u8),
            count: filler,
            fractured: false,
            source: None,
        });
        names.push(format!("{filler} elemental resistances at T{FILLER_TIER}+"));
    }
    let compiled = Target::compile(game, &goal).map_err(|error| anyhow!(error))?;
    let mut needs: Vec<_> = names
        .into_iter()
        .zip(compiled.needs)
        .map(|(name, need)| Need {
            name,
            mods: need.ok.iter().map(|&id| game.def(id).key.clone()).collect(),
            count: need.count,
            fractured: need.fractured,
        })
        .collect();
    needs.sort_by(|a, b| (&a.mods, a.count, a.fractured).cmp(&(&b.mods, b.count, b.fractured)));
    fixed.sort();
    Ok(Prepared {
        candidate: Candidate {
            base: goal.base.expect("fixed base"),
            item_level: u8::try_from(level)?,
            influences: matched.item.influences.clone(),
            needs,
            fixed,
            min_mods: 0,
        },
        rows,
    })
}
fn to_mod(id: &str, m: &data::Mod, weight: f64) -> PoolMod {
    PoolMod { id: id.into(), weight, groups: m.groups.clone(), notable: false, label: m.text_or_empty().replace('\n', " / ") }
}
