use crate::candidate::Candidate;
use anyhow::{Result, anyhow};
pub use ponder_plan::session::{Flow, FlowEdge, FlowKind, FlowNode};
use ponder_plan::{
    market::Market,
    session::{Method, Request},
    supply::Stock,
};
use ponder_rules::ops::Supply;
use ponder_rules::{
    game::{Game, GroupId, Influences, Kind, ModId, Side, TagId},
    goal::{Influence, Rarity, Source},
    item::{self, Affix, Item},
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
    /// How the item is made, part by part, the goal last.
    pub stages: Vec<Stage>,
    pub depth: Depth,
}

/// Which mechanics a solve tries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Depth {
    /// Plain crafting, with imprints and Dominance on the parts of items made with Awakener's
    /// Orbs. Memory strands, Hinekora's Lock and these on whole items cost far more time and
    /// memory than they save.
    #[default]
    Ladder,
    /// Every mechanic on every item, recombining included.
    Deep,
}

impl Depth {
    /// The slow mechanics offered at this depth for an item of `kind`, or for a part of one.
    fn mechanics(self, kind: Kind, part: bool) -> &'static [&'static str] {
        match self {
            Depth::Ladder if part && kind.influences.iter().count() >= 2 => &["imprint", "restore", "dominance"],
            Depth::Ladder => &[],
            Depth::Deep if kind.influences.0 != 0 => &["imprint", "restore", "beast:lock", "remembrance", "dominance"],
            Depth::Deep => &["imprint", "restore", "beast:lock", "remembrance"],
        }
    }
}

/// One item of a recipe and how it is made.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Stage {
    pub item: String,
    pub influences: Vec<String>,
    pub rarity: Option<String>,
    pub needs: Vec<String>,
    /// Per need: the side its mods roll on ("prefix", "suffix" or "either"), and how many it takes.
    pub places: Vec<(String, u8)>,
    /// Most (prefixes, suffixes) the base holds when magic, then when rare.
    pub slots: [[usize; 2]; 2],
    /// Operations in the order a run first uses them, purchases included.
    pub ops: Vec<String>,
    /// Expected uses per item made.
    pub uses: Vec<(String, f64)>,
    /// Expected chaos per item made, by operation.
    pub spend: Vec<(String, f64)>,
    /// Chaos per item made, everything included.
    pub cost: f64,
    pub flow: Flow,
}

impl Summary {
    /// The recipe in the format of `fixtures/guides`: stages, then steps working each item
    /// until it holds its stage.
    pub fn recipe(&self, base: &str) -> String {
        let quote = |list: &[String]| list.iter().map(|s| format!("{s:?}")).collect::<Vec<_>>().join(", ");
        let mut out = format!("base = {base:?}\n");
        let (goal, parts) = self.stages.split_last().expect("a summary has its goal's stage");
        if !goal.influences.is_empty() {
            out += &format!("influences = [{}]\n", quote(&goal.influences));
        }
        let mods = |stage: &Stage| stage.needs.iter().map(|need| format!("{{ text = {need:?} }}")).collect::<Vec<_>>().join(", ");
        out += &format!("\n[goal]\nmods = [{}]\n", mods(goal));
        for stage in parts {
            out += &format!("\n[stages.{}]\ninfluences = [{}]\n", stage.item, quote(&stage.influences));
            if let Some(rarity) = &stage.rarity {
                out += &format!("rarity = {rarity:?}\n");
            }
            out += &format!("mods = [{}]\n", mods(stage));
        }
        for stage in &self.stages {
            let until = if std::ptr::eq(stage, goal) { "goal" } else { &stage.item };
            let uses = stage.uses.iter().map(|(op, n)| format!("{op} {n:.1}")).collect::<Vec<_>>().join(", ");
            out += &format!(
                "\n[[steps]]\nitem = {:?}\ndo = [{}]\nuntil = {until:?}\nnote = \"{:.1}c each; expected uses: {uses}\"\n",
                stage.item,
                quote(&stage.ops),
                stage.cost
            );
        }
        out
    }
}

/// The share of a side's junk roll weight a tag needs before banning it is worth offering.
const BAN_JUNK_SHARE: f64 = 0.5;

/// A tag ban pays on a side whose wanted mods all carry the tag, since annuls then skip them, or
/// whose junk mostly carries it while some wanted mod there doesn't, since exalts then skip the junk.
fn ban_pays(game: &Game, target: &Target, rolls: &[(ModId, f64)], avoid: &[GroupId], tag: TagId) -> bool {
    Side::BOTH.into_iter().any(|side| {
        let needs: Vec<_> = target.needs.iter().filter(|need| need.ok.iter().any(|&id| game.def(id).side == side)).collect();
        let tagged = needs.iter().filter(|need| need.ok.iter().any(|&id| game.def(id).tags.contains(&tag))).count();
        let (banned, junk) = rolls
            .iter()
            .filter(|&&(id, _)| game.def(id).side == side && !game.def(id).groups.iter().any(|group| avoid.contains(group)))
            .fold((0.0, 0.0), |(banned, junk), &(id, weight)| {
                (banned + if game.def(id).tags.contains(&tag) { weight } else { 0.0 }, junk + weight)
            });
        let protects = !needs.is_empty() && tagged == needs.len();
        let blocks = tagged < needs.len() && junk > 0.0 && banned >= BAN_JUNK_SHARE * junk;
        protects || blocks
    })
}

fn methods(game: &Game, kind: Kind, target: &Target, market: &Market, mechanics: &[&str]) -> Vec<Method> {
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
    names.push("beast:wild-bristle-matron".into());
    if target.influences.0 != 0 {
        names.push("harvest:reforge-influence".into());
    }
    // Only an Orb of Dominance makes elevated mods.
    if wanted.iter().any(|&id| game.def(id).source == Source::Elevated) && !mechanics.contains(&"dominance") {
        names.push("dominance".into());
    }
    names.extend(mechanics.iter().map(|&name| name.to_string()));
    for name in ["bench:prefixes-cannot-be-changed", "bench:suffixes-cannot-be-changed"] {
        if ops::parse(game, class, name, &avoid).is_some() {
            names.push(name.into());
        }
    }
    for tag in ["attack", "caster"] {
        let name = format!("bench:cannot-roll-{tag}");
        if game.tag_id(tag).is_some_and(|tag| ban_pays(game, target, &rolls, &avoid, tag))
            && ops::parse(game, class, &name, &avoid).is_some()
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
    let over: Vec<String> = names
        .iter()
        .filter_map(|name| name.strip_prefix("bench:"))
        .filter(|name| !matches!(*name, "remove-crafted" | "multimod"))
        .map(|name| format!("bench-over:{name}"))
        .collect();
    names.extend(over);
    names.sort();
    names.dedup();
    names.into_iter().map(Method::Named).collect()
}

pub fn prepare(game: &Game, candidate: &Candidate, market: &Market, depth: Depth) -> Result<Request> {
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
    let methods = methods(game, kind, &target, market, depth.mechanics(kind, false));

    Ok(Request { kind, goal: target, methods, starts: vec![Arc::new(Stock::new("base", market.base, item))] })
}

/// An item made on its own for a later stage to consume.
pub struct Part {
    pub name: String,
    /// The goal's needs it carries, by index.
    pub needs: Vec<usize>,
    pub request: Request,
}

/// A craft in stages: parts made separately, then the finish worked from an heir's item with
/// operations that consume a partner's.
pub struct Chain {
    pub mechanic: &'static str,
    pub heirs: Vec<Part>,
    pub partners: Vec<Part>,
    finish: Request,
}

impl Chain {
    /// The finishing request, given the products of the heirs and partners in order.
    pub fn finish(&self, heirs: Vec<Arc<dyn Supply>>, partners: Vec<Arc<dyn Supply>>) -> Request {
        let mut request = Request { starts: heirs, ..self.finish.clone() };
        request.methods.extend(self.partners.iter().zip(partners).map(|(part, supply)| match self.mechanic {
            "awakener" => Method::Awaken(part.name.clone(), supply),
            _ => Method::Recombine(part.name.clone(), supply),
        }));
        request
    }
}

/// The staged ways to make the request's item. A two-influence item can only be made by
/// awakening: an item of each influence carrying one of its needs, magic where the base allows. A request with rolled
/// needs on both sides can also be recombined from an item with the prefixes and one with the
/// suffixes.
pub fn chains(game: &Game, request: &Request, market: &Market, depth: Depth) -> Vec<Chain> {
    if request.starts.iter().flat_map(|s| s.items()).any(|item| !item.affixes.is_empty()) {
        return Vec::new();
    }
    let needs = &request.goal.needs;
    let part = |name: String, influences: Influences, rarity: Option<Rarity>, carried: Vec<usize>| {
        let kind = Kind { influences, ..request.kind };
        let wanted =
            carried.iter().map(|&i| Need { count: if rarity == Some(Rarity::Magic) { 1 } else { needs[i].count }, ..needs[i].clone() });
        let goal = Target { influences, rarity, needs: wanted.collect(), min_mods: 0, ..request.goal.clone() };
        let methods = methods(game, kind, &goal, market, depth.mechanics(request.kind, true));
        let starts: Vec<Arc<dyn Supply>> = vec![Arc::new(Stock::new("base", market.base, Item::new(kind)))];
        Part { name, needs: carried, request: Request { kind, goal, methods, starts } }
    };
    let carrying =
        |keep: &dyn Fn(ModId) -> bool| -> Vec<usize> { (0..needs.len()).filter(|&i| needs[i].ok.iter().all(|&id| keep(id))).collect() };
    let mut chains = Vec::new();
    let influences: Vec<Influence> = request.kind.influences.iter().collect();
    if let [receiver, donor] = influences[..] {
        let parts = |influence: Influence| -> Vec<Part> {
            let one = Influences::of(&[influence]);
            let name = format!("{influence:?}").to_lowercase();
            let carried = carrying(&|id| game.def(id).influence == Some(influence));
            if carried.is_empty() {
                return vec![part(name, one, None, Vec::new())];
            }
            let magic = |i: usize| {
                let holds = needs[i].ok.iter().all(|&id| item::limit(game, request.kind.base, Rarity::Magic, game.def(id).side) > 0);
                holds.then_some(Rarity::Magic)
            };
            carried.into_iter().map(|i| part(format!("{name}-{i}"), one, magic(i), vec![i])).collect()
        };
        chains.push(Chain { mechanic: "awakener", heirs: parts(receiver), partners: parts(donor), finish: request.clone() });
    }
    let rolled =
        |side: Side| carrying(&|id| game.def(id).side == side && matches!(game.def(id).source, Source::Natural | Source::Influence));
    let (prefixes, suffixes) = (rolled(Side::Prefix), rolled(Side::Suffix));
    let count = |carried: &[usize]| carried.iter().map(|&i| needs[i].count as usize).sum::<usize>();
    if influences.len() < 2 && count(&prefixes) >= RECOMBINED_SIDE && count(&suffixes) >= RECOMBINED_SIDE {
        let influences = request.kind.influences;
        chains.push(Chain {
            mechanic: "recombinator",
            heirs: vec![part("prefixes".into(), influences, None, prefixes)],
            partners: vec![part("suffixes".into(), influences, None, suffixes)],
            finish: request.clone(),
        });
    }
    chains
}

/// Rolled needs each side must hold before recombining the sides pays.
const RECOMBINED_SIDE: usize = 2;
