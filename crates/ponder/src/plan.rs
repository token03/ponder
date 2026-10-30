use crate::candidate::Candidate;
use anyhow::{Result, anyhow};
use ponder_plan::{
    market::Market,
    session::{Method, Request},
    supply::Stock,
};
use ponder_rules::{
    game::{Game, Influences, Kind, Side},
    goal::{Influence, Rarity, Source},
    item::{Affix, Item},
    ops::{self, Operation},
    target::{Need, Target},
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Summary {
    pub cost: f64,
    pub hours: f64,
    pub bases: f64,
    pub states: usize,
    pub actions: usize,
    pub outcomes: usize,
    pub usage: Vec<(String, f64)>,
    pub unpriced: Vec<String>,
}

fn methods(game: &Game, kind: Kind, target: &Target, market: &Market) -> Vec<Method> {
    let class = &game.bases[kind.base.0 as usize].class;
    let wanted: HashSet<_> = target.needs.iter().flat_map(|need| need.ok.iter().copied()).collect();
    let avoid: Vec<_> = wanted.iter().flat_map(|&id| game.def(id).groups.iter().copied()).collect();
    let mut names: Vec<_> = ops::currency::Currency::ALL.iter().map(|operation| operation.name()).collect();
    names.push("bench:remove-crafted".into());
    names.extend(
        ops::essence::Essence::all(game, class)
            .into_iter()
            .filter(|essence| wanted.contains(&essence.forced()))
            .map(|essence| essence.name()),
    );
    names.extend(
        ops::harvest::reforges(game)
            .into_iter()
            .filter(|(_, tag)| wanted.iter().any(|&id| game.def(id).tags.contains(tag)))
            .map(|(operation, _)| operation.name()),
    );
    names.extend(wanted.iter().filter(|&&id| game.def(id).source == Source::Crafted).filter_map(|&id| {
        let name = format!("bench:{}", game.def(id).key);
        ops::parse(game, class, &name, &avoid).map(|operation| operation.name())
    }));
    if target.needs.iter().filter(|need| need.ok.iter().any(|&id| game.def(id).source == Source::Crafted)).count() > 1 {
        names.push("bench:multimod".into());
    }
    let pool = game.pool(kind);
    let rolls: Vec<_> = pool
        .mods
        .iter()
        .filter(|&&(id, _)| {
            let def = game.def(id);
            def.level <= kind.ilvl as u32 && matches!(def.source, Source::Natural | Source::Influence)
        })
        .copied()
        .collect();
    // Temporary blockers exclude unwanted groups. Keep the largest combined roll
    // weight on each side, breaking ties by the actual priced cost.
    for side in Side::BOTH {
        let blocker = game
            .bench
            .iter()
            .filter(|option| option.item_classes.contains(class))
            .filter_map(|option| game.mod_id(option.actions.add_explicit_mod.as_deref()?))
            .filter(|&id| {
                let def = game.def(id);
                def.source == Source::Crafted
                    && def.side == side
                    && !game.metamods.contains(&id)
                    && !def.groups.iter().any(|g| avoid.contains(g))
            })
            .filter_map(|id| {
                let name = format!("bench:{}", game.def(id).key);
                let operation = ops::parse(game, class, &name, &avoid)?;
                let price = market.price(&operation.cost())?;
                let weight: f64 = rolls
                    .iter()
                    .filter(|&&(other, _)| game.def(other).groups.iter().any(|group| game.def(id).groups.contains(group)))
                    .map(|(_, weight)| weight)
                    .sum();
                Some((operation.name(), weight, price))
            })
            .max_by(|a, b| a.1.total_cmp(&b.1).then_with(|| b.2.total_cmp(&a.2)).then_with(|| b.0.cmp(&a.0)));
        if let Some((name, _, _)) = blocker {
            names.push(name);
        }
    }
    if target.influences.0 == 0 && matches!(class.as_str(), "Body Armour" | "Boots" | "Gloves" | "Helmet") {
        names.extend(
            [
                "eldritch-implicit:lesser-ichor",
                "eldritch-implicit:lesser-ember",
                "eldritch-chaos",
                "eldritch-exalted",
                "eldritch-annulment",
            ]
            .map(String::from),
        );
    }
    for name in ["bench:prefixes-cannot-be-changed", "bench:suffixes-cannot-be-changed"] {
        if ops::parse(game, class, name, &avoid).is_some() {
            names.push(name.into());
        }
    }
    for tag in ["attack", "caster"] {
        let name = format!("bench:cannot-roll-{tag}");
        if game.tag_id(tag).is_some_and(|tag| {
            rolls.iter().any(|&(id, _)| game.def(id).tags.contains(&tag)) || wanted.iter().any(|&id| game.def(id).tags.contains(&tag))
        }) && ops::parse(game, class, &name, &avoid).is_some()
        {
            names.push(name);
        }
    }
    names.extend(
        game.fossils
            .iter()
            .filter(|fossil| {
                fossil
                    .added_mods
                    .iter()
                    .chain(&fossil.forced_mods)
                    .filter_map(|key| game.mod_id(key))
                    .any(|id| wanted.contains(&id) && game.def(id).source == Source::Fossil)
            })
            .filter_map(|fossil| {
                let name = format!("fossil:{}", fossil.name.to_lowercase().strip_suffix(" fossil")?);
                ops::parse(game, class, &name, &avoid).map(|_| name)
            }),
    );
    let unveiled: Vec<_> = wanted.iter().filter(|&&id| game.def(id).source == Source::Veiled).copied().collect();
    if !unveiled.is_empty() {
        names.extend(["veiled-chaos".into(), "veiled-exalted".into()]);
        names.extend(unveiled.iter().map(|&id| format!("unveil:{}", game.def(id).key)));
    }
    names.sort();
    names.dedup();
    names.into_iter().map(Method::Named).collect()
}

pub fn prepare(game: &Game, candidate: &Candidate, market: &Market) -> Result<Request> {
    let base = game.base(&candidate.base).ok_or_else(|| anyhow!("unknown crafting base {:?}", candidate.base))?;
    let influences =
        candidate.influences.iter().map(|name| serde_json::from_value::<Influence>(name.clone().into())).collect::<Result<Vec<_>, _>>()?;
    let kind = Kind { base, ilvl: candidate.item_level, influences: Influences::of(&influences) };
    let needs = candidate
        .needs
        .iter()
        .map(|need| {
            Ok(Need {
                ok: need
                    .mods
                    .iter()
                    .map(|id| game.mod_id(id).ok_or_else(|| anyhow!("unsupported modifier {id}")))
                    .collect::<Result<_>>()?,
                count: need.count,
                fractured: need.fractured,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let target = Target {
        base: Some(base),
        ilvl: candidate.item_level,
        quality: 0,
        influences: kind.influences,
        rarity: None,
        needs,
        open: [0; 2],
        max_side: [None; 2],
        max_mods: None,
        min_mods: candidate.min_mods,
    };
    let mut item = Item::new(kind);
    for id in &candidate.fixed {
        let id = game.mod_id(id).ok_or_else(|| anyhow!("unsupported fractured modifier {id}"))?;
        item.affixes.push(Affix { id, crafted: game.def(id).source == ponder_rules::goal::Source::Crafted, fractured: true });
    }
    if !item.affixes.is_empty() {
        item.rarity = Rarity::Rare;
    }
    let methods = methods(game, kind, &target, market);

    Ok(Request { kind, goal: target, methods, starts: vec![Arc::new(Stock::new("base", market.base, item))] })
}
