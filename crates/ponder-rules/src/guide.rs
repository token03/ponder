//! A human crafting recipe (`fixtures/guides/*.toml`): the acceptance oracle of docs/SPEC.md §8.
//!
//! A recipe is a small state machine. Each step applies its operations to one item, repeating
//! until a goal holds; `max` bounds the attempts and `else` names the step to fall back to.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::goal::{Goal, Influence};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Guide {
    /// Workbook tab the recipe comes from.
    pub sheet: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub base_tags: Vec<String>,
    /// What the recipe leaves out (catalysts, enchants, ...), in words.
    #[serde(default)]
    pub scope: Option<String>,
    pub base: String,
    #[serde(default)]
    pub ilvl: Option<u8>,
    #[serde(default)]
    pub quality: Option<u8>,
    #[serde(default)]
    pub influences: Vec<Influence>,
    pub goal: Goal,
    /// Named intermediates. They inherit the guide's base, item level, quality and influences.
    #[serde(default)]
    pub stages: BTreeMap<String, Goal>,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Operations applied in order on each attempt: `mechanic` or `mechanic:argument`.
    #[serde(rename = "do")]
    pub ops: Vec<String>,
    /// The item worked on; recipes with donors name each item. Defaults to "main".
    #[serde(default = "main")]
    pub item: String,
    /// Repeat until this stage (or "goal") holds; absent means apply once.
    #[serde(default)]
    pub until: Option<String>,
    /// Most attempts before falling back; absent means unlimited.
    #[serde(default)]
    pub max: Option<u32>,
    /// 1-based step to return to when `max` attempts fail.
    #[serde(rename = "else", default)]
    pub fallback: Option<usize>,
    #[serde(default)]
    pub note: Option<String>,
}

fn main() -> String {
    "main".into()
}

impl Guide {
    /// The base as `Game` names it, enchant tags included.
    pub fn base_name(&self) -> String {
        let tags: Vec<&str> = self.base_tags.iter().map(String::as_str).collect();
        crate::game::enchanted(&self.base, &tags)
    }

    /// A stage or the goal by name, with the guide's item defaults filled in.
    pub fn target(&self, name: &str) -> Option<Goal> {
        let mut g = if name == "goal" { self.goal.clone() } else { self.stages.get(name)?.clone() };
        g.base.get_or_insert_with(|| self.base_name());
        if g.ilvl.is_none() {
            g.ilvl = self.ilvl;
        }
        if g.quality.is_none() {
            g.quality = self.quality;
        }
        if g.influences.is_empty() {
            g.influences = self.influences.clone();
        }
        Some(g)
    }

    /// The mechanic of every operation, e.g. `essence` for `essence:deafening-woe`.
    pub fn mechanics(&self) -> impl Iterator<Item = &str> {
        self.steps.iter().flat_map(|s| &s.ops).map(|op| op.split_once(':').map_or(op.as_str(), |(m, _)| m))
    }
}
