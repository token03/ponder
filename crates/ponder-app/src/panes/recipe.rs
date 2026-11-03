//! A recipe as in a cookbook: what it makes and costs, the ingredients, then a numbered method.
//! Each step is headed by what it does and what the item has by then, then says which currency
//! to use and, a row each, where every result goes. Steps follow the likeliest route, so most
//! results carry on to the next one. The same method copies out as text or as a
//! Kalandralang-style script.

use std::sync::Arc;

use egui::{Align, RichText, Ui};
use ponder::pipeline::Summary;
use ponder::plan::{Flow, FlowEdge, FlowKind, FlowNode, Stage};
use ponder_game::mods::ModDB;

use crate::state::State;
use crate::tags::{Kind, Tag};
use crate::widgets::{empty, money, need};
use crate::{theme, view};

/// Currencies a step names at most; rarer ones are left to a note.
const ACTIONS: usize = 3;
/// Currencies used less than this often per finished item are left out.
const RARE_USE: f64 = 0.05;
/// Results rarer than this are not spelled out.
const RARE_RESULT: f64 = 0.02;
/// Currencies used for less than this share of a step's uses go unnamed.
const RARE_LINE: f64 = 0.02;
/// A lone result this likely is what the currency does.
const CERTAIN: f64 = 0.99;
/// Results leaving a step less often than this share of the times it is left go unnamed.
const RARE_EXIT: f64 = 0.1;
/// A currency used this many times or more each time its step is arrived at is spammed.
const SPAM: f64 = 1.5;
/// The room a step's number takes before its text.
const NUMBER_WIDTH: f32 = 26.0;
/// The most steps off the route the method lists; the rest are found by the item in hand.
const DETOURS: usize = 8;
/// The most steps the finder follows an item through on its usual way.
const WAY: usize = 6;
/// The most steps the finder shows for one item.
const FOUND: usize = 3;
/// The width of a step's item card.
const CARD_WIDTH: f32 = 210.0;
/// The width of an outcome's share column.
const OUTCOME_SHARE_WIDTH: f32 = 40.0;
/// Steps reached less often than this per finished item are marked as rare.
const RARE_STEP: f64 = 0.05;

/// A bench mod's text by its id, from the game data once it has loaded.
pub fn bench_text(db: Option<Arc<ModDB>>) -> impl Fn(&str) -> Option<String> {
    move |key| db.as_ref()?.affix(key).map(|m| m.text_or_empty().replace('\n', " / ")).filter(|text| !text.is_empty())
}

/// The selected craft's recipe, as large as the pane.
pub fn pane(ui: &mut Ui, state: &mut State) {
    let Some(estimate) = state.selected_estimate() else {
        empty(ui, "⚒", "Select a craft");
        return;
    };
    let Some(summary) = &estimate.summary else {
        empty(ui, "⚒", "Not solved yet");
        return;
    };
    let bench = bench_text(state.db.clone());
    let cx = Context { base: &estimate.base, item_level: estimate.item_level, bench: &bench };
    let mut copied = None;
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&estimate.title).size(16.0).strong());
        if ui.button("📋 text").on_hover_text("copy the recipe as text").clicked() {
            copied = Some(text(summary, &cx));
        }
        if ui.button("📋 Kalandralang").on_hover_text("copy as a Kalandralang-style script (may need edits to run)").clicked() {
            copied = Some(script(summary, &cx));
        }
    });
    let picked = egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self::ui(ui, summary, &cx, state.divine())).inner;
    if let Some(text) = copied {
        ui.ctx().copy_text(text);
        state.announce("recipe copied", false);
    }
    if let Some(op) = picked {
        state.saved.filters.add(Tag::new(Kind::Uses, op));
        state.refilter();
    }
}

/// What a recipe needs from outside it: the base, and readable names for bench crafts.
pub struct Context<'a> {
    pub base: &'a str,
    pub item_level: i64,
    /// a bench mod's text, by its id
    pub bench: &'a dyn Fn(&str) -> Option<String>,
}

/// Where a result sends the item.
#[derive(Clone, Copy, PartialEq)]
enum Next {
    /// still the same step: use the currency again, or go on to the step's next one
    Same,
    Step(usize),
    Done,
    StartOver,
}

struct Branch {
    share: f64,
    /// what the result did, to follow "it"; empty when it only did what the currency always does
    what: Vec<Piece>,
    next: Next,
    /// the step edge it follows
    edge: usize,
}

/// One currency used at a step.
struct Line {
    op: String,
    /// the share of the step's currency uses that are this one
    share: f64,
    /// expected uses each time the step is arrived at
    each: f64,
    /// for alterations, the augmentations used alongside each time the step is arrived at
    augments: f64,
    /// what every item it is used on has beyond the step's own: needs, then open affixes
    when: Vec<Piece>,
    /// the needs every result gets, as the currency forces them
    always: Vec<Piece>,
    /// the crafted mods every result loses: key and side
    dropped: Vec<(String, String)>,
    branches: Vec<Branch>,
}

impl Line {
    /// Whether the currency is used again and again, rather than once.
    fn spam(&self) -> bool {
        self.each >= SPAM
    }

    fn stays(&self) -> f64 {
        self.branches.iter().filter(|branch| branch.next == Next::Same).map(|branch| branch.share).sum()
    }

    fn exits(&self) -> impl Iterator<Item = &Branch> {
        self.branches.iter().filter(|branch| branch.next != Next::Same)
    }
}

/// A need the item holds at a step.
struct Held {
    name: Name,
    fractured: bool,
    /// "prefix", "suffix" or "either"
    side: String,
    count: u8,
}

/// What the item has at a step, as far as the recipe cares.
struct Have {
    kind: FlowKind,
    rarity: Option<String>,
    mods: Vec<Held>,
    /// most (prefixes, suffixes) at its rarity
    slots: [usize; 2],
    /// mods on each side, needed or not; `None` where the step's items differ
    filled: [Option<u8>; 2],
    /// crafted mods: text and side
    crafted: Vec<(String, String)>,
    /// the side Eldritch currency works on, as the dominant implicit sets it
    eldritch: Option<String>,
    /// whether it holds a veiled mod
    veiled: bool,
}

struct Step {
    /// what the step does: "Add maximum Life", "Finish"
    heading: Vec<Piece>,
    have: Have,
    /// times reached per finished item
    visits: f64,
    lines: Vec<Line>,
    /// expected uses per finished item of the currencies too rare to name
    others: f64,
    /// the needs held, and whether fractured
    needs: Vec<(usize, bool)>,
    /// where the item most often goes from here
    usual: Option<Next>,
}

struct Chapter {
    title: String,
    goal: Vec<Piece>,
    cost: f64,
    /// (operation, expected uses, expected chaos) per item made
    ingredients: Vec<(String, f64, f64)>,
    steps: Vec<Step>,
    /// how many of the steps are the usual route; the rest put it back on track
    route: usize,
    needs: Vec<Name>,
}

impl Chapter {
    /// How many steps the method lists: the route, then the few others reached most, while they
    /// are reached often enough.
    pub fn listed(&self) -> usize {
        let common = (self.route..self.steps.len()).find(|&s| self.steps[s].visits < RARE_STEP).unwrap_or(self.steps.len());
        common.min(self.route + DETOURS)
    }
}

/// A piece of a sentence: words, a mod, a currency to name, or a step to point at.
#[derive(Clone, PartialEq)]
enum Piece {
    Words(String),
    /// a mod in few words, its full text, and its role's colour (`theme::role`)
    Mod {
        short: String,
        full: String,
        role: &'static str,
    },
    Op(String),
    Step(usize),
}

fn words(text: impl Into<String>) -> Piece {
    Piece::Words(text.into())
}

/// A need's name in few words, and its full text.
#[derive(Clone)]
struct Name {
    short: String,
    full: String,
}

/// The needs' names: in few words, unless two would read the same.
fn names(needs: &[String]) -> Vec<Name> {
    let shorts: Vec<String> = needs.iter().map(|text| need(text)).collect();
    let clash = |short: &String| shorts.iter().filter(|other| *other == short).count() > 1;
    needs
        .iter()
        .zip(&shorts)
        .map(|(full, short)| Name { short: if clash(short) { full.clone() } else { short.clone() }, full: full.clone() })
        .collect()
}

fn wanted(name: &Name) -> Piece {
    Piece::Mod { short: name.short.clone(), full: name.full.clone(), role: "target" }
}

fn fractured(name: &Name) -> Piece {
    Piece::Mod { short: name.short.clone(), full: format!("{} (fractured)", name.full), role: "fractured" }
}

/// Pieces as a list: "a, b and c".
fn list(items: Vec<Piece>) -> Vec<Piece> {
    let count = items.len();
    let mut out = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        match i {
            0 => {}
            _ if i + 1 == count => out.push(words(" and ")),
            _ => out.push(words(", ")),
        }
        out.push(item);
    }
    out
}

/// The needs held at `to` and not at `from`, named.
fn gained(from: &[usize], to: &[usize], needs: &[Name], piece: fn(&Name) -> Piece) -> Vec<Piece> {
    list(to.iter().filter(|i| !from.contains(i)).filter_map(|&i| needs.get(i)).map(piece).collect())
}

/// What a result made of the item at `from`, to follow "it": "gets life but loses int". Empty
/// when only its rarity changed, as the currency always does.
fn result(
    change: &Change,
    from: &FlowNode,
    to: &FlowNode,
    shift: Shift,
    needs: &[Name],
    bench: &dyn Fn(&str) -> Option<String>,
) -> Vec<Piece> {
    match to.kind {
        FlowKind::StartOver | FlowKind::Buy => vec![words("is ruined")],
        FlowKind::Finished | FlowKind::Work => {
            let (got, lost, fractures) = change.named(needs);
            match (got.is_empty(), lost.is_empty()) {
                _ if !fractures.is_empty() => [vec![words("fractures ")], fractures].concat(),
                (false, true) => [vec![words("gets ")], got].concat(),
                (true, false) => [vec![words("loses ")], lost].concat(),
                (false, false) => [vec![words("gets ")], got, vec![words(" but loses ")], lost].concat(),
                (true, true) if to.kind == FlowKind::Finished => Vec::new(),
                (true, true) => filled(&filler(from, to, shift, bench)),
            }
        }
    }
}

/// What a result did to the mods you don't need.
enum Filler {
    /// crafted mods added, by their text
    Crafts(Vec<String>),
    Uncrafts(Vec<String>),
    /// the sides that gained other mods
    Fills(Vec<&'static str>),
    Empties,
    Shuffles,
    Nothing,
}

/// How a result changes how many mods the item holds, where every item a step merges agrees:
/// the sign per side (prefixes, suffixes), and in all.
#[derive(Clone, Copy, Default)]
struct Shift {
    sides: [Option<i8>; 2],
    total: Option<i8>,
}

impl Shift {
    /// The change in mods other than crafted ones.
    fn of(from: &FlowNode, to: &FlowNode) -> Shift {
        let others = |node: &FlowNode, side: usize| {
            let crafted = node.crafted.iter().filter(|(_, on)| on == ["prefix", "suffix"][side]).count();
            node.filled[side] as i32 - crafted as i32
        };
        let sign = |side: usize| (others(to, side) - others(from, side)).signum() as i8;
        let total = (others(to, 0) + others(to, 1) - others(from, 0) - others(from, 1)).signum() as i8;
        Shift { sides: [Some(sign(0)), Some(sign(1))], total: Some(total) }
    }

    fn agree(self, other: Shift) -> Shift {
        let both = |a: Option<i8>, b: Option<i8>| a.filter(|_| a == b);
        Shift { sides: [0, 1].map(|side| both(self.sides[side], other.sides[side])), total: both(self.total, other.total) }
    }
}

/// What a result does to the needs: those it gets, loses and fractures.
#[derive(Clone, Default)]
struct Change {
    got: Vec<usize>,
    lost: Vec<usize>,
    fractured: Vec<usize>,
}

impl Change {
    /// The needs got, lost and fractured, named.
    fn named(&self, needs: &[Name]) -> (Vec<Piece>, Vec<Piece>, Vec<Piece>) {
        let name = |ids: &[usize], piece: fn(&Name) -> Piece| list(ids.iter().filter_map(|&i| needs.get(i)).map(piece).collect());
        (name(&self.got, wanted), name(&self.lost, wanted), name(&self.fractured, fractured))
    }

    fn agree(&self, other: &Change) -> Change {
        let both = |a: &[usize], b: &[usize]| a.iter().copied().filter(|i| b.contains(i)).collect();
        Change { got: both(&self.got, &other.got), lost: both(&self.lost, &other.lost), fractured: both(&self.fractured, &other.fractured) }
    }
}

/// How often each need is got, lost and fractured over the results a step's edge merges, to
/// name what most of them do.
#[derive(Clone)]
struct Tally {
    flow: f64,
    got: Vec<f64>,
    lost: Vec<f64>,
    fractured: Vec<f64>,
}

impl Tally {
    fn new(needs: usize) -> Tally {
        Tally { flow: 0.0, got: vec![0.0; needs], lost: vec![0.0; needs], fractured: vec![0.0; needs] }
    }

    /// Counts a result from `from` to `to` taken `flow` times; a finished item holds every need.
    fn add(&mut self, from: &FlowNode, to: &FlowNode, flow: f64) {
        self.flow += flow;
        let held: Vec<usize> = match to.kind {
            FlowKind::Finished => (0..self.got.len()).collect(),
            FlowKind::Work => to.held.clone(),
            FlowKind::StartOver | FlowKind::Buy => return,
        };
        for need in 0..self.got.len() {
            let (before, after) = (from.held.contains(&need), held.contains(&need));
            if after && !before {
                self.got[need] += flow;
            }
            if before && !after {
                self.lost[need] += flow;
            }
            if to.fractured.contains(&need) && !from.fractured.contains(&need) {
                self.fractured[need] += flow;
            }
        }
    }

    fn change(&self) -> Change {
        let most = |counts: &[f64]| (0..counts.len()).filter(|&need| counts[need] > self.flow / 2.0).collect();
        Change { got: most(&self.got), lost: most(&self.lost), fractured: most(&self.fractured) }
    }
}

fn filler(from: &FlowNode, to: &FlowNode, shift: Shift, bench: &dyn Fn(&str) -> Option<String>) -> Filler {
    let text = |key: &String| bench(key).unwrap_or_else(|| key.clone());
    let added: Vec<String> = to.crafted.iter().filter(|m| !from.crafted.contains(m)).map(|(key, _)| text(key)).collect();
    let lost: Vec<String> = from.crafted.iter().filter(|m| !to.crafted.contains(m)).map(|(key, _)| text(key)).collect();
    let more: Vec<&'static str> =
        ["prefix", "suffix"].into_iter().enumerate().filter(|&(side, _)| shift.sides[side] == Some(1)).map(|(_, name)| name).collect();
    match shift.total {
        _ if !added.is_empty() => Filler::Crafts(added),
        _ if !lost.is_empty() => Filler::Uncrafts(lost),
        Some(1) => Filler::Fills(more),
        Some(-1) => Filler::Empties,
        Some(0) if !more.is_empty() => Filler::Shuffles,
        _ => Filler::Nothing,
    }
}

/// The affixes an item leaves open, where its fill is known: " (2 prefixes open)", or nothing.
fn open(slots: &[usize; 2], fills: &[Option<u8>; 2]) -> Vec<Piece> {
    opening(slots, fills).map_or_else(Vec::new, |open| vec![words(format!(" ({open})"))])
}

/// The affixes an item leaves open, where its fill is known: "2 prefixes open".
fn opening(slots: &[usize; 2], fills: &[Option<u8>; 2]) -> Option<String> {
    let fills = fills.iter().copied().collect::<Option<Vec<u8>>>()?;
    let sides: Vec<String> = ["prefix", "suffix"]
        .iter()
        .enumerate()
        .filter_map(|(side, name)| match slots[side].saturating_sub(fills[side] as usize) {
            0 => None,
            1 => Some(format!("1 {name}")),
            n => Some(format!("{n} {name}es")),
        })
        .collect();
    let open = if sides.is_empty() { "nothing".to_string() } else { sides.join(" and ") };
    Some(format!("{open} open"))
}

/// The open affixes every item a currency is used on agrees on, where the step's items differ:
/// "has 1 prefix open", "has its suffixes full".
fn room(slots: &[usize; 2], users: &[Option<u8>; 2], step: &[Option<u8>; 2]) -> Vec<Piece> {
    let sides: Vec<Piece> = ["prefix", "suffix"]
        .iter()
        .enumerate()
        .filter(|&(side, _)| step[side].is_none())
        .filter_map(|(side, name)| {
            Some(words(match slots[side].saturating_sub(users[side]? as usize) {
                0 => format!("has its {name}es full"),
                1 => format!("has 1 {name} open"),
                n => format!("has {n} {name}es open"),
            }))
        })
        .collect();
    list(sides)
}

fn crafted(texts: &[String]) -> Vec<Piece> {
    list(texts.iter().map(|text| Piece::Mod { short: text.clone(), full: text.clone(), role: "bench" }).collect())
}

/// A result's `Filler`, to follow "it": "fills a suffix with another mod". Empty when nothing changed.
fn filled(change: &Filler) -> Vec<Piece> {
    match change {
        Filler::Crafts(texts) => [vec![words("gets the crafted ")], crafted(texts)].concat(),
        Filler::Uncrafts(texts) => [vec![words("loses the crafted ")], crafted(texts)].concat(),
        Filler::Fills(sides) if sides.len() == 2 => vec![words("fills both sides with other mods")],
        Filler::Fills(sides) if sides.is_empty() => vec![words("fills an open affix with another mod")],
        Filler::Fills(sides) => vec![words(format!("fills a {} with another mod", sides[0]))],
        Filler::Empties => vec![words("loses another mod")],
        Filler::Shuffles => vec![words("swaps other mods around")],
        Filler::Nothing => Vec::new(),
    }
}

/// The heading of a step: what its main currency does on the way to `to`, as "Add maximum
/// Life"; `hopes` names the needs that currency's other results get.
fn heading(
    change: &Change,
    from: &FlowNode,
    to: &FlowNode,
    shift: Shift,
    hopes: Vec<Piece>,
    needs: &[Name],
    bench: &dyn Fn(&str) -> Option<String>,
) -> Vec<Piece> {
    if from.kind == FlowKind::Buy {
        return vec![words("Buy the base")];
    }
    match to.kind {
        FlowKind::Finished => vec![words("Finish")],
        FlowKind::StartOver | FlowKind::Buy => vec![words("Start over")],
        FlowKind::Work => {
            let (got, lost, fractures) = change.named(needs);
            let others = filler(from, to, shift, bench);
            match others {
                _ if !fractures.is_empty() => [vec![words("Fracture ")], fractures].concat(),
                Filler::Crafts(texts) => match texts[0].strip_suffix(" Cannot Be Changed") {
                    Some(side) => vec![words(format!("Lock its {}", side.to_lowercase()))],
                    None => [vec![words("Craft ")], crafted(&texts)].concat(),
                },
                Filler::Uncrafts(_) if shift.total == Some(-1) => vec![words("Make room")],
                Filler::Uncrafts(_) => vec![words("Clear the crafted mods")],
                _ if !lost.is_empty() && shift.total == Some(-1) => vec![words("Make room")],
                _ if !lost.is_empty() => vec![words("Reroll it")],
                _ if !got.is_empty() => [vec![words("Add ")], got].concat(),
                _ if to.veiled && !from.veiled => vec![words("Add a veiled mod")],
                _ if to.eldritch != from.eldritch => {
                    vec![words(format!("Make the {} implicit dominant", dominant(to.eldritch.as_deref())))]
                }
                _ if to.rarity != from.rarity => vec![words(format!("Make it {}", to.rarity.as_deref().unwrap_or("different")))],
                Filler::Empties => vec![words("Make room")],
                Filler::Fills(_) => vec![words("Fill an open affix")],
                _ if hopes.is_empty() => vec![words("Rework the other mods")],
                _ => [vec![words("Roll for ")], hopes].concat(),
            }
        }
    }
}

/// The Eldritch implicit that makes currency work on a side.
fn dominant(side: Option<&str>) -> &'static str {
    match side {
        Some("prefix") => "Searing Exarch",
        Some("suffix") => "Eater of Worlds",
        _ => "other",
    }
}

/// Each item's group, numbered by first appearance: items with equal keys share one.
fn partition<K: PartialEq>(keys: Vec<K>) -> Vec<usize> {
    let mut seen: Vec<&K> = Vec::new();
    keys.iter()
        .map(|key| match seen.iter().position(|known| *known == key) {
            Some(group) => group,
            None => {
                seen.push(key);
                seen.len() - 1
            }
        })
        .collect()
}

/// What every item a step's currency is used on has: the needs beyond the step's own, and the
/// fill per side where they agree.
#[derive(Clone, Default)]
struct Using {
    op: String,
    extra: Vec<usize>,
    fill: [Option<u8>; 2],
}

/// A flow read as steps.
struct Steps {
    flow: Flow,
    /// each step's fill per side, where its items agree
    fills: Vec<[Option<u8>; 2]>,
    /// for each step, the currencies used there and what every item each is used on has
    when: Vec<Vec<Using>>,
    /// for each edge, the needs every item it reaches has: a step's own may be fewer
    reached: Vec<Vec<usize>>,
    /// for each edge, how its results change the fill
    shifts: Vec<Shift>,
    /// for each edge, how its results change the needs
    changes: Vec<Change>,
}

/// The flow as steps: items are one step while you do the same thing to them and its results
/// lead to the same steps, whichever of your mods or other mods they hold. A step starts as the
/// items given the same currency with the same fractured and crafted mods and dominant implicit
/// (magic items: any currency, as alteration and augmentation are one loop; normal and rare
/// items alike, as an essence makes both rare), and splits until that holds.
/// `needs`: how many the goal has.
fn steps(flow: &Flow, needs: usize) -> Steps {
    let first = |node: &FlowNode| node.actions.first().map(|(op, _)| op.clone());
    let start = |node: &FlowNode| {
        let magic = node.rarity.as_deref() == Some("magic");
        let action = if magic { None } else { first(node) };
        (node.kind, node.fractured.clone(), node.crafted.clone(), node.eldritch.clone(), node.veiled, magic, action)
    };
    let mut leaving: Vec<Vec<&FlowEdge>> = vec![Vec::new(); flow.nodes.len()];
    for edge in &flow.edges {
        leaving[edge.from].push(edge);
    }
    let mut groups = partition(flow.nodes.iter().map(start).collect());
    loop {
        let signature = |i: usize| {
            let total: f64 = leaving[i].iter().map(|edge| edge.flow).sum::<f64>().max(f64::MIN_POSITIVE);
            let mut leads: Vec<(&str, Option<usize>)> = leaving[i]
                .iter()
                .filter(|edge| edge.flow / total >= RARE_RESULT)
                .map(|edge| (edge.action.as_str(), (groups[edge.to] != groups[i]).then_some(groups[edge.to])))
                .collect();
            leads.sort();
            leads.dedup();
            (groups[i], leads)
        };
        let split = partition((0..flow.nodes.len()).map(signature).collect());
        let count = |groups: &[usize]| groups.iter().max().map_or(0, |most| most + 1);
        if count(&split) == count(&groups) {
            break;
        }
        groups = split;
    }
    let mut members: Vec<Vec<usize>> = Vec::new();
    for (i, &group) in groups.iter().enumerate() {
        if members.len() <= group {
            members.resize(group + 1, Vec::new());
        }
        members[group].push(i);
    }
    let common = |items: &mut dyn Iterator<Item = usize>| -> Vec<usize> {
        let Some(start) = items.next() else { return Vec::new() };
        items.fold(flow.nodes[start].held.clone(), |held, i| held.into_iter().filter(|need| flow.nodes[i].held.contains(need)).collect())
    };
    let nodes = members
        .iter()
        .map(|group| {
            let most = group.iter().copied().max_by(|&a, &b| flow.nodes[a].visits.total_cmp(&flow.nodes[b].visits));
            let mut node = flow.nodes[most.expect("a group has members")].clone();
            node.held = common(&mut group.iter().copied());
            node.visits = group.iter().map(|&i| flow.nodes[i].visits).sum();
            node.filled = [0, 1].map(|side| group.iter().map(|&i| flow.nodes[i].filled[side]).min().expect("a group has members"));
            let mut actions: Vec<(String, f64)> = Vec::new();
            for (op, uses) in group.iter().flat_map(|&i| &flow.nodes[i].actions) {
                match actions.iter_mut().find(|(known, _)| known == op) {
                    Some((_, total)) => *total += uses,
                    None => actions.push((op.clone(), *uses)),
                }
            }
            actions.sort_by(|a, b| b.1.total_cmp(&a.1));
            node.actions = actions;
            node
        })
        .collect::<Vec<FlowNode>>();
    let when = members
        .iter()
        .zip(&nodes)
        .map(|(group, node)| {
            node.actions
                .iter()
                .map(|(op, _)| {
                    let using: Vec<usize> =
                        group.iter().copied().filter(|&i| flow.nodes[i].actions.iter().any(|(used, _)| used == op)).collect();
                    let extra = common(&mut using.iter().copied()).into_iter().filter(|need| !node.held.contains(need)).collect();
                    let fill = [0, 1].map(|side| {
                        let first = flow.nodes[using[0]].filled[side];
                        using.iter().all(|&i| flow.nodes[i].filled[side] == first).then_some(first)
                    });
                    Using { op: op.clone(), extra, fill }
                })
                .collect()
        })
        .collect();
    let mut edges: Vec<FlowEdge> = Vec::new();
    let mut targets: Vec<Vec<usize>> = Vec::new();
    let mut shifts: Vec<Shift> = Vec::new();
    let mut tallies: Vec<Tally> = Vec::new();
    for edge in &flow.edges {
        let (from, to) = (groups[edge.from], groups[edge.to]);
        let shift = Shift::of(&flow.nodes[edge.from], &flow.nodes[edge.to]);
        let (source, target) = (&flow.nodes[edge.from], &flow.nodes[edge.to]);
        match edges.iter().position(|known| (known.from, known.to, &known.action) == (from, to, &edge.action)) {
            Some(known) => {
                edges[known].flow += edge.flow;
                targets[known].push(edge.to);
                shifts[known] = shifts[known].agree(shift);
                tallies[known].add(source, target, edge.flow);
            }
            None => {
                edges.push(FlowEdge { from, to, action: edge.action.clone(), flow: edge.flow });
                targets.push(vec![edge.to]);
                shifts.push(shift);
                let mut tally = Tally::new(needs);
                tally.add(source, target, edge.flow);
                tallies.push(tally);
            }
        }
    }
    let reached = targets.iter().map(|to| common(&mut to.iter().copied())).collect();
    let fills = members
        .iter()
        .map(|group| {
            [0, 1].map(|side| {
                let first = flow.nodes[group[0]].filled[side];
                group.iter().all(|&i| flow.nodes[i].filled[side] == first).then_some(first)
            })
        })
        .collect();
    let changes = tallies.iter().map(Tally::change).collect();
    Steps { flow: Flow { nodes, edges }, fills, when, reached, shifts, changes }
}

/// The order to read a flow's steps in: the likeliest route, then the other steps from the
/// most reached; and how many are on the route.
fn order(flow: &Flow) -> (Vec<usize>, usize) {
    let nodes = &flow.nodes;
    let mut rows = flow.route();
    let on_route = rows.len();
    let mut rest: Vec<usize> =
        (0..nodes.len()).filter(|&i| !rows.contains(&i) && matches!(nodes[i].kind, FlowKind::Buy | FlowKind::Work)).collect();
    rest.sort_by(|a, b| nodes[*b].visits.total_cmp(&nodes[*a].visits));
    rows.extend(rest);
    (rows, on_route)
}

fn chapter(stage: &Stage, finish: bool, bench: &dyn Fn(&str) -> Option<String>) -> Chapter {
    let Steps { flow, fills, when, reached, shifts, changes } = steps(&stage.flow, stage.needs.len());
    let reaches = |at: usize| {
        let to = &flow.nodes[flow.edges[at].to];
        let held = if to.kind == FlowKind::Finished { (0..stage.needs.len()).collect() } else { reached[at].clone() };
        FlowNode { held, ..to.clone() }
    };
    let nodes = &flow.nodes;
    let edges = &flow.edges;
    let (rows, route) = order(&flow);
    let likeliest = flow.likeliest();
    let needs = names(&stage.needs);
    let step_of = |node: usize| rows.iter().position(|&row| row == node);
    let onward = |row: usize| rows.iter().position(|&r| r == row).filter(|&at| at + 1 < route).map(|at| rows[at + 1]);
    let slots_of = |node: &FlowNode| match node.rarity.as_deref() {
        Some("magic") => stage.slots[0],
        Some("rare") => stage.slots[1],
        _ => [0, 0],
    };
    let steps: Vec<Step> = rows
        .iter()
        .map(|&row| {
            let node = &nodes[row];
            let arrivals = match node.kind {
                FlowKind::Buy => node.visits,
                _ => edges.iter().filter(|edge| edge.to == row && edge.from != row).map(|edge| edge.flow).sum::<f64>(),
            };
            let mut used: Vec<&(String, f64)> = node.actions.iter().collect();
            used.sort_by(|a, b| b.1.total_cmp(&a.1));
            let all: f64 = used.iter().map(|(_, uses)| uses).sum::<f64>().max(f64::MIN_POSITIVE);
            let named = used
                .iter()
                .enumerate()
                .filter(|(i, (_, uses))| *i == 0 || (*uses >= RARE_USE && uses / all >= RARE_LINE))
                .take(ACTIONS)
                .count();
            let onward = onward(row);
            let carries = onward.and_then(|to| {
                let carrying = edges.iter().filter(|edge| edge.from == row && edge.to == to);
                carrying.max_by(|a, b| a.flow.total_cmp(&b.flow)).map(|edge| edge.action.as_str())
            });
            let used: Vec<&(String, f64)> =
                used.iter().enumerate().filter(|(i, (op, _))| *i < named || Some(op.as_str()) == carries).map(|(_, used)| *used).collect();
            let others = all - used.iter().map(|(_, uses)| uses).sum::<f64>();
            let mut used = used;
            let first_use = |op: &str| stage.ops.iter().position(|name| name == op).unwrap_or(usize::MAX);
            used.sort_by_key(|(op, _)| first_use(op));
            let lines = used
                .iter()
                .map(|(op, uses)| {
                    let from_here: Vec<(usize, &FlowEdge)> =
                        edges.iter().enumerate().filter(|(_, edge)| edge.from == row && &edge.action == op).collect();
                    let total: f64 = from_here.iter().map(|(_, edge)| edge.flow).sum::<f64>().max(f64::MIN_POSITIVE);
                    let leaving: f64 =
                        from_here.iter().filter(|(_, edge)| edge.to != row).map(|(_, edge)| edge.flow).sum::<f64>().max(f64::MIN_POSITIVE);
                    let kept: Vec<(usize, &FlowEdge)> = from_here
                        .into_iter()
                        .filter(|(_, edge)| {
                            edge.flow / total >= RARE_RESULT
                                || (edge.to != row && edge.flow / leaving >= RARE_EXIT)
                                || nodes[edge.to].kind == FlowKind::Finished
                                || (Some(edge.to) == onward && Some(op.as_str()) == carries)
                        })
                        .collect();
                    let Using { extra, fill, .. } = when[row].iter().find(|using| &using.op == op).cloned().unwrap_or_default();
                    let landed: Vec<usize> = kept
                        .iter()
                        .filter(|(_, edge)| matches!(nodes[edge.to].kind, FlowKind::Work | FlowKind::Finished))
                        .map(|&(at, _)| at)
                        .collect();
                    let several = landed.len() > 1;
                    let always: Vec<usize> = match landed.split_first() {
                        Some((&first, rest)) if several => {
                            rest.iter().fold(changes[first].clone(), |change, &at| change.agree(&changes[at])).got
                        }
                        _ => Vec::new(),
                    };
                    let dropped: Vec<(String, String)> = match several {
                        false => Vec::new(),
                        true => {
                            node.crafted.iter().filter(|m| landed.iter().all(|&at| !reaches(at).crafted.contains(m))).cloned().collect()
                        }
                    };
                    let mut from = node.clone();
                    from.crafted.retain(|m| !dropped.contains(m));
                    let from = &from;
                    let results: Vec<(Branch, Vec<Piece>)> = kept
                        .iter()
                        .map(|&(at, edge)| {
                            let to = &reaches(at);
                            let next = match to.kind {
                                _ if edge.to == row => Next::Same,
                                FlowKind::Finished => Next::Done,
                                FlowKind::StartOver | FlowKind::Buy => Next::StartOver,
                                _ => step_of(edge.to).map_or(Next::StartOver, Next::Step),
                            };
                            let what = match opening(&slots_of(to), &fills[edge.to]) {
                                Some(open) if node.kind == FlowKind::Buy => {
                                    let mods = gained(&[], &to.held, &needs, wanted);
                                    let mods = if mods.is_empty() { mods } else { [mods, vec![words(", ")]].concat() };
                                    [vec![words("has ")], mods, vec![words(open)]].concat()
                                }
                                _ => {
                                    let change = Change {
                                        got: changes[at].got.iter().copied().filter(|need| !always.contains(need)).collect(),
                                        ..changes[at].clone()
                                    };
                                    result(&change, from, to, shifts[at], &needs, bench)
                                }
                            };
                            let aside = filled(&filler(from, to, shifts[at], bench));
                            let aside =
                                if aside.is_empty() || aside == what { Vec::new() } else { [vec![words(", and ")], aside].concat() };
                            let aside = [aside, open(&slots_of(to), &fills[edge.to])].concat();
                            (Branch { share: edge.flow / total, what, next, edge: at }, aside)
                        })
                        .collect();
                    let alike = |what: &[Piece]| results.iter().filter(|(other, _)| other.what == what).count() > 1;
                    let mut branches: Vec<Branch> = results
                        .iter()
                        .map(|(branch, aside)| {
                            let what = match () {
                                _ if !alike(&branch.what) => branch.what.clone(),
                                _ if branch.what.is_empty() => [vec![words("gets nothing you need")], aside.clone()].concat(),
                                _ => [branch.what.clone(), aside.clone()].concat(),
                            };
                            Branch { what, ..*branch }
                        })
                        .collect();
                    branches.sort_by(|a, b| b.share.total_cmp(&a.share));
                    let each = uses / arrivals.max(f64::MIN_POSITIVE);
                    let named = |held: &[usize]| list(held.iter().filter_map(|&i| needs.get(i)).map(wanted).collect());
                    let also = if extra.is_empty() { Vec::new() } else { [vec![words("also has ")], named(&extra)].concat() };
                    let when = [also, room(&slots_of(node), &fill, &fills[row])].into_iter().filter(|part| !part.is_empty());
                    let when = when.reduce(|a, b| [a, vec![words(" and ")], b].concat()).unwrap_or_default();
                    Line { op: op.clone(), share: uses / all, each, augments: 0.0, when, always: named(&always), dropped, branches }
                })
                .collect();
            let lines = arrange(alter(lines));
            let last = route > 0 && rows.get(route - 1) == Some(&row);
            let finished = nodes.iter().position(|node| node.kind == FlowKind::Finished).filter(|_| last);
            let towards = onward.or(finished).or(likeliest[row]);
            let main = lines.iter().filter(|line| line.exits().next().is_some()).max_by(|a, b| a.share.total_cmp(&b.share));
            let leaving: Vec<usize> = main.map_or_else(Vec::new, |line| line.exits().map(|branch| branch.edge).collect());
            let likeliest_of = |ats: &mut dyn Iterator<Item = usize>| ats.max_by(|&a, &b| edges[a].flow.total_cmp(&edges[b].flow));
            let on = |at: &usize| matches!(nodes[edges[*at].to].kind, FlowKind::Work | FlowKind::Finished);
            let target = likeliest_of(&mut leaving.iter().copied().filter(|&at| Some(edges[at].to) == towards))
                .or_else(|| likeliest_of(&mut leaving.iter().copied().filter(on)))
                .or_else(|| likeliest_of(&mut leaving.iter().copied()))
                .or_else(|| likeliest_of(&mut (0..edges.len()).filter(|&at| edges[at].from == row && Some(edges[at].to) == towards)));
            let hopes: Vec<usize> = (0..needs.len()).filter(|need| leaving.iter().any(|&at| changes[at].got.contains(need))).collect();
            let hopes = list(hopes.iter().map(|&need| wanted(&needs[need])).collect());
            let mut from = node.clone();
            from.crafted.retain(|m| main.is_none_or(|line| !line.dropped.contains(m)));
            let matron = main.is_some_and(|line| line.op == "beast:wild-bristle-matron" && line.exits().count() > 1);
            let heading = match main.and_then(|line| line.op.strip_prefix("buy:")) {
                _ if matron => vec![words("Lock a side")],
                Some(part) if part != "base" => vec![words(format!("Start with the {part} item"))],
                _ => target.map_or_else(
                    || vec![words("Carry on")],
                    |at| heading(&changes[at], &from, &reaches(at), shifts[at], hopes, &needs, bench),
                ),
            };
            let slots = slots_of(node);
            let blocked: Vec<Piece> = (0..needs.len())
                .filter(|need| !node.held.contains(need))
                .filter(|&need| {
                    let (side, count) = stage.places.get(need).cloned().unwrap_or(("either".into(), 1));
                    let side = ["prefix", "suffix"].iter().position(|name| *name == side);
                    side.and_then(|side| Some(slots[side].saturating_sub(fills[row][side]? as usize)))
                        .is_some_and(|room| room < count as usize)
                })
                .map(|need| wanted(&needs[need]))
                .collect();
            let finishes = main.map_or(0.0, |line| {
                let done: f64 = line.exits().filter(|branch| branch.next == Next::Done).map(|branch| branch.share).sum();
                done / (1.0 - line.stays()).max(f64::MIN_POSITIVE)
            });
            let heading = match heading.as_slice() {
                [Piece::Words(said)] if said == "Finish" && finishes < 0.5 => vec![words("Try to finish")],
                _ if blocked.is_empty() => heading,
                [Piece::Words(said)] if said == "Reroll it" => [vec![words("Reroll it: no room left for ")], list(blocked)].concat(),
                [Piece::Words(said)] if said == "Make room" && blocked.len() <= 2 => {
                    [vec![words("Make room for ")], list(blocked)].concat()
                }
                [Piece::Words(said)] if said == "Clear the crafted mods" => vec![words("Clear the crafted mods to make room")],
                _ => heading,
            };
            let crafted: Vec<(String, String)> =
                node.crafted.iter().map(|(key, side)| (bench(key).unwrap_or_else(|| key.clone()), side.clone())).collect();
            let by_craft = |name: &Name| {
                crafted.iter().any(|(text, _)| text.contains(&name.full) || text.split(" / ").any(|part| need(part) == name.short))
            };
            let mods = node
                .held
                .iter()
                .filter_map(|&i| {
                    let (side, count) = stage.places.get(i).cloned().unwrap_or(("either".into(), 1));
                    Some(Held { name: needs.get(i)?.clone(), fractured: node.fractured.contains(&i), side, count })
                })
                .filter(|held| !by_craft(&held.name))
                .collect();
            let have = Have {
                kind: node.kind,
                rarity: node.rarity.clone(),
                mods,
                slots,
                filled: fills[row],
                crafted,
                eldritch: node.eldritch.clone(),
                veiled: node.veiled,
            };
            let usual =
                edges.iter().filter(|edge| edge.from == row && edge.to != row).max_by(|a, b| a.flow.total_cmp(&b.flow)).map(|edge| {
                    match nodes[edge.to].kind {
                        FlowKind::Finished => Next::Done,
                        FlowKind::StartOver | FlowKind::Buy => Next::StartOver,
                        FlowKind::Work => step_of(edge.to).map_or(Next::StartOver, Next::Step),
                    }
                });
            let held = node.held.iter().map(|&need| (need, node.fractured.contains(&need))).collect();
            Step { heading, have, visits: node.visits, lines, others, needs: held, usual }
        })
        .collect();
    let cost_of = |op: &str| stage.spend.iter().find(|(name, _)| name == op).map_or(0.0, |(_, chaos)| *chaos);
    let mut ingredients: Vec<(String, f64, f64)> =
        stage.uses.iter().filter(|(_, uses)| *uses >= RARE_USE).map(|(op, uses)| (op.clone(), *uses, cost_of(op))).collect();
    ingredients.sort_by(|a, b| b.2.total_cmp(&a.2).then(b.1.total_cmp(&a.1)));
    let bought = stage.flow.nodes.iter().filter(|node| node.kind == FlowKind::Buy);
    let bought = bought.flat_map(|node| node.actions.iter().map(|(op, uses)| (op.clone(), *uses, 0.0)));
    let ingredients = bought.chain(ingredients).collect();
    let title = if finish { "To finish".to_string() } else { format!("For the {} item", stage.item) };
    let goal = list(needs.iter().map(wanted).collect());
    Chapter { title, goal, cost: stage.cost, ingredients, steps, route, needs }
}

/// A step's lines with alterations and augmentations as one: alteration spam, augmenting when
/// an affix is open. Their results are weighed by how often each is used.
fn alter(mut lines: Vec<Line>) -> Vec<Line> {
    let find = |lines: &[Line], op: &str| lines.iter().position(|line| line.op == op);
    let (Some(_), Some(at)) = (find(&lines, "alteration"), find(&lines, "augmentation")) else { return lines };
    let augmentation = lines.remove(at);
    let at = find(&lines, "alteration").expect("alteration is still there");
    let alteration = &mut lines[at];
    let both = (alteration.share + augmentation.share).max(f64::MIN_POSITIVE);
    let (alter_weight, augment_weight) = (alteration.share / both, augmentation.share / both);
    for branch in &mut alteration.branches {
        branch.share *= alter_weight;
    }
    for mut branch in augmentation.branches {
        branch.share *= augment_weight;
        match alteration.branches.iter_mut().find(|known| known.next == branch.next) {
            Some(known) => known.share += branch.share,
            None => alteration.branches.push(branch),
        }
    }
    alteration.branches.sort_by(|a, b| b.share.total_cmp(&a.share));
    alteration.share += augmentation.share;
    alteration.augments = augmentation.each;
    lines
}

/// A step's lines, most used first; a line leaving what you need alone (a lock) stays just
/// before the line it is done ahead of.
fn arrange(lines: Vec<Line>) -> Vec<Line> {
    let mut groups: Vec<Vec<Line>> = vec![Vec::new()];
    for line in lines {
        let acts = line.exits().next().is_some();
        groups.last_mut().expect("a group is open").push(line);
        if acts {
            groups.push(Vec::new());
        }
    }
    groups.retain(|group| !group.is_empty());
    let share = |group: &Vec<Line>| group.iter().map(|line| line.share).sum::<f64>();
    groups.sort_by(|a, b| share(b).total_cmp(&share(a)));
    groups.into_iter().flatten().collect()
}

/// The recipe's chapters, kept between frames while the same plan is shown.
fn book(ui: &Ui, summary: &Summary, cx: &Context) -> Arc<Vec<Chapter>> {
    let id = ui.scope_id().with("recipe book");
    let plan = (std::ptr::from_ref(summary) as usize, summary.cost.to_bits());
    if let Some((shown, book)) = ui.data(|d| d.get_temp::<((usize, u64), Arc<Vec<Chapter>>)>(id))
        && shown == plan
    {
        return book;
    }
    let book = Arc::new(chapters(summary, cx));
    ui.data_mut(|d| d.insert_temp(id, (plan, book.clone())));
    book
}

fn chapters(summary: &Summary, cx: &Context) -> Vec<Chapter> {
    let last = summary.stages.len() - 1;
    summary.stages.iter().enumerate().map(|(i, stage)| chapter(stage, i == last, cx.bench)).collect()
}

fn title_case(words: &str) -> String {
    words
        .split(['-', '_', ' '])
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars.next().map_or(String::new(), |first| first.to_uppercase().chain(chars).collect())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// An operation as a crafter calls it: "Shrieking Essence of Contempt", "Orb of Annulment".
pub fn noun(op: &str, cx: &Context) -> String {
    let (kind, rest) = op.split_once(':').unwrap_or((op, ""));
    match kind {
        "transmutation" => "Orb of Transmutation".into(),
        "alteration" => "Orb of Alteration".into(),
        "augmentation" => "Orb of Augmentation".into(),
        "regal" => "Regal Orb".into(),
        "alchemy" => "Orb of Alchemy".into(),
        "chaos" => "Chaos Orb".into(),
        "exalted" => "Exalted Orb".into(),
        "annulment" => "Orb of Annulment".into(),
        "scouring" => "Orb of Scouring".into(),
        "fracturing" => "Fracturing Orb".into(),
        "dominance" => "Orb of Dominance".into(),
        "remembrance" => "Memory Strand".into(),
        "imprint" => "Imprint (Craicic Croaker)".into(),
        "restore" => "restored imprint".into(),
        "eldritch-chaos" => "Eldritch Chaos Orb".into(),
        "eldritch-exalted" => "Eldritch Exalted Orb".into(),
        "eldritch-annulment" => "Eldritch Orb of Annulment".into(),
        "eldritch-implicit" => format!(
            "{} Eldritch {}",
            title_case(rest.split('-').next().unwrap_or("")),
            title_case(rest.split_once('-').map_or("", |r| r.1))
        ),
        "essence" => match rest.split_once('-') {
            Some((tier, kind)) => format!("{} Essence of {}", title_case(tier), title_case(kind)),
            None => format!("Essence of {}", title_case(rest)),
        },
        "harvest" => format!("Harvest {}", rest.replace('-', " ")),
        "beast" if rest == "lock" => "Hinekora's Lock".into(),
        "beast" => title_case(rest),
        "awakener" => format!("Awakener's Orb with the {rest} item"),
        "recombinator" => format!("Recombinator with the {rest} item"),
        "buy" if rest == "base" => format!("{} (item level {}+)", cx.base, cx.item_level),
        "buy" => format!("{rest} item (made above)"),
        "unveil" => format!("unveil, keeping {}", plain(&unveiled(rest, cx), cx)),
        "bench" => match rest {
            "remove-crafted" => "bench: remove crafted mods".into(),
            "multimod" => "bench: “Can have up to 3 crafted modifiers”".into(),
            "prefixes-cannot-be-changed" => "bench: “Prefixes cannot be changed”".into(),
            "suffixes-cannot-be-changed" => "bench: “Suffixes cannot be changed”".into(),
            "cannot-roll-attack" => "bench: “Cannot roll attack modifiers”".into(),
            "cannot-roll-caster" => "bench: “Cannot roll caster modifiers”".into(),
            key => format!("bench: “{}”", (cx.bench)(key).unwrap_or_else(|| key.to_string())),
        },
        "bench-over" => format!("bench: “{}” over a crafted mod", (cx.bench)(rest).unwrap_or_else(|| rest.to_string())),
        _ => title_case(op),
    }
}

/// What an unveil keeps: the veiled mod's text, or the guide's name for its choice.
fn unveiled(keep: &str, cx: &Context) -> Vec<Piece> {
    let text = (cx.bench)(keep).unwrap_or_else(|| keep.replace('-', " "));
    vec![Piece::Mod { short: format!("“{text}”"), full: text, role: "target" }]
}

fn article(noun: &str) -> &'static str {
    if noun.starts_with(['A', 'E', 'I', 'O', 'U']) { "an" } else { "a" }
}

/// A count, rounded as a recipe would: "29", "1.5".
fn count(uses: f64) -> String {
    if uses < 9.5 && (uses - uses.round()).abs() >= 0.05 { format!("{uses:.1}") } else { format!("{uses:.0}") }
}

/// How much of an ingredient, as a recipe says it: "2", "about 160", "maybe 1".
fn amount(uses: f64) -> String {
    if uses < 0.75 {
        "maybe 1".into()
    } else if (uses - uses.round()).abs() < 0.05 {
        format!("{}", uses.round())
    } else {
        format!("about {:.0}", uses)
    }
}

/// What a line does, as an instruction: "Spam Shrieking Essence of Contempt (about 9 tries)",
/// "Remove the crafted mods at the bench".
fn doing(line: &Line, cx: &Context) -> Vec<Piece> {
    let what = instruction(line, cx);
    let gives = if line.always.is_empty() || line.op.starts_with("buy:") {
        Vec::new()
    } else {
        [vec![words("gives ")], line.always.clone()].concat()
    };
    let drops: Vec<String> = line.dropped.iter().map(|(key, _)| (cx.bench)(key).unwrap_or_else(|| key.clone())).collect();
    let removes = if drops.is_empty() { Vec::new() } else { [vec![words("removes the crafted ")], crafted(&drops)].concat() };
    match (gives.is_empty(), removes.is_empty()) {
        (true, true) => what,
        (false, false) => [what, vec![words(" — it always ")], gives, vec![words(" and ")], removes].concat(),
        _ => [what, vec![words(" — it always ")], gives, removes].concat(),
    }
}

/// Currencies that add or take a random mod once: you slam them.
const SLAMS: [&str; 6] = ["exalted", "eldritch-exalted", "veiled-exalted", "regal", "annulment", "eldritch-annulment"];

fn instruction(line: &Line, cx: &Context) -> Vec<Piece> {
    let (kind, rest) = line.op.split_once(':').unwrap_or((&line.op, ""));
    let op = |lead: &str| vec![words(lead), Piece::Op(line.op.clone())];
    match (kind, rest) {
        ("buy", "base") => vec![words(format!("Buy {} {} of item level {} or higher", article(cx.base), cx.base, cx.item_level))],
        ("buy", part) => vec![words(format!("Take one of the {part} items made above"))],
        ("unveil", _) => [vec![words("Unveil it at Jun and keep ")], unveiled(rest, cx)].concat(),
        ("bench", "remove-crafted") => vec![words("Remove the crafted mods at the bench")],
        ("bench", _) => {
            let text = (cx.bench)(rest).unwrap_or_else(|| rest.to_string());
            vec![words("Bench-craft "), Piece::Mod { short: text.clone(), full: text, role: "bench" }]
        }
        ("bench-over", _) => {
            let text = (cx.bench)(rest).unwrap_or_else(|| rest.to_string());
            vec![words("Bench-craft "), Piece::Mod { short: text.clone(), full: text, role: "bench" }, words(" over the crafted mod")]
        }
        ("beast", "wild-bristle-matron") if line.branches.len() == 1 => {
            [op("Use a "), vec![words(": only one side has room for its lock, so it is sure to fit")]].concat()
        }
        ("beast", "wild-bristle-matron") => [op("Use a "), vec![words(", which locks its prefixes or its suffixes, either one")]].concat(),
        _ if line.augments > 0.0 && line.spam() => [
            op("Spam "),
            vec![words(format!(" (about {}), with an ", count(line.each)))],
            vec![Piece::Op("augmentation".into())],
            vec![words(format!(" whenever a prefix or suffix is open (about {})", count(line.augments)))],
        ]
        .concat(),
        _ if line.spam() => [op("Spam "), vec![words(format!(" (about {} tries)", count(line.each)))]].concat(),
        _ if SLAMS.contains(&line.op.as_str()) => op(&format!("Slam {} ", article(&noun(&line.op, cx)))),
        _ => op(&format!("Use {} ", article(&noun(&line.op, cx)))),
    }
}

/// A result as a row under its instruction: how often (when there is a choice), what it did,
/// and where it sends the item.
struct Row {
    share: Option<f64>,
    what: Vec<Piece>,
    /// where it goes: more than one where results read alike, for the item's card to tell
    next: Vec<Next>,
}

/// A line's results as rows. A spammed currency's results that change nothing are the spamming
/// itself, so its other results are shares of those that change something.
fn rows(line: &Line) -> Vec<Row> {
    let spam = line.spam();
    let of = if spam { (1.0 - line.stays()).max(f64::MIN_POSITIVE) } else { 1.0 };
    let mut shown: Vec<&Branch> = line.branches.iter().filter(|branch| !(spam && branch.next == Next::Same)).collect();
    if shown.iter().all(|branch| branch.next == Next::Same) {
        shown.clear();
    }
    let choice = shown.len() > 1;
    let certain = !choice && shown.first().is_some_and(|branch| branch.share >= CERTAIN);
    let mut out: Vec<Row> = shown
        .into_iter()
        .map(|branch| {
            let what = match (&branch.next, branch.what.is_empty()) {
                (Next::Same, _) => vec![words("if nothing you need changes")],
                _ if certain => vec![words("then")],
                (_, true) if !choice => vec![words("then")],
                (Next::Done, true) => vec![words("if that finishes it")],
                (_, true) => vec![words("if it gets nothing you need")],
                (_, false) => [vec![words("if it ")], branch.what.clone()].concat(),
            };
            Row { share: choice.then_some(branch.share / of), what, next: vec![branch.next] }
        })
        .collect();
    out.sort_by(|a, b| b.share.unwrap_or(1.0).total_cmp(&a.share.unwrap_or(1.0)));
    let mut merged: Vec<Row> = Vec::new();
    for row in out {
        match merged.iter_mut().find(|known| known.what == row.what) {
            Some(known) => {
                known.share = known.share.zip(row.share).map(|(a, b)| a + b);
                known.next.extend(row.next);
            }
            None => merged.push(row),
        }
    }
    merged
}

/// Where a row sends the item: "step 4", "step 4 or step 9, whichever matches it".
fn going(next: &[Next], line: &Line) -> Vec<Piece> {
    match next {
        [one] => going_one(*one, line),
        many => {
            let ways: Vec<Vec<Piece>> = many.iter().map(|&next| going_one(next, line)).collect();
            let ways = ways.into_iter().reduce(|a, b| [a, vec![words(" or ")], b].concat()).unwrap_or_default();
            [ways, vec![words(", whichever matches it")]].concat()
        }
    }
}

fn going_one(next: Next, line: &Line) -> Vec<Piece> {
    match next {
        Next::Same if line.spam() => vec![words("again")],
        Next::Same => vec![words("stay on this step")],
        Next::Step(to) => vec![Piece::Step(to)],
        Next::Done => vec![words("done ✔")],
        Next::StartOver => vec![words("start over at "), Piece::Step(0)],
    }
}

fn percent(share: f64) -> String {
    if share * 100.0 < 0.5 { "<1%".into() } else { format!("{:.0}%", share * 100.0) }
}

/// How a step's line follows the one before: in turn after one that leaves what you need alone
/// (a lock), else instead of it, on the items it suits.
fn lead(lines: &[Line], l: usize) -> Option<Vec<Piece>> {
    let previous = lines[..l].last()?;
    let line = &lines[l];
    let share = percent(line.share);
    Some(match () {
        _ if previous.exits().next().is_none() => vec![words("Then:")],
        _ if line.when.is_empty() => vec![words(format!("Or, depending on its other mods ({share} of the time):"))],
        _ => [vec![words("Or, when it ")], line.when.clone(), vec![words(format!(" ({share} of the time):"))]].concat(),
    })
}

/// What the item has, in few words: "life + int (fractured)".
fn has(have: &Have) -> String {
    if have.kind == FlowKind::Buy {
        return "nothing yet".into();
    }
    let none = have.mods.is_empty().then(|| format!("{}, none of the mods yet", have.rarity.as_deref().unwrap_or("new")));
    none.into_iter()
        .chain(
            have.mods.iter().map(|held| if held.fractured { format!("{} (fractured)", held.name.short) } else { held.name.short.clone() }),
        )
        .chain(have.crafted.iter().map(|(text, _)| format!("crafted {text}")))
        .chain(have.veiled.then(|| "a veiled mod".to_string()))
        .chain(have.eldritch.as_deref().map(|side| format!("{} dominant", dominant(Some(side)))))
        .collect::<Vec<_>>()
        .join(" + ")
}

fn plain(pieces: &[Piece], cx: &Context) -> String {
    pieces
        .iter()
        .map(|piece| match piece {
            Piece::Words(text) => text.clone(),
            Piece::Mod { short, .. } => short.clone(),
            Piece::Op(op) => noun(op, cx),
            Piece::Step(to) => format!("step {}", to + 1),
        })
        .collect()
}

/// The recipe to read; returns an operation clicked.
pub fn ui(ui: &mut Ui, summary: &Summary, cx: &Context, divine: f64) -> Option<String> {
    let mut clicked = None;
    let id = ui.scope_id().with("recipe jump");
    let jump = ui.data_mut(|d| d.remove_temp::<(usize, usize)>(id));
    let book = book(ui, summary, cx);
    if let Some(finish) = book.last() {
        let makes = [vec![words(format!("Makes one {} with ", cx.base))], finish.goal.clone(), vec![words(".")]].concat();
        if let Some(op) = sentence(ui, &makes, cx, None, false) {
            clicked = Some(op);
        }
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Costs about").weak());
            ui.label(money(Some(summary.cost), divine)).on_hover_text("expected cost per item made, everything included");
            if book.len() > 1 {
                ui.label(RichText::new(format!("· in {} parts", book.len())).weak());
            }
        });
        ui.add_space(6.0);
    }
    for (c, chapter) in book.iter().enumerate() {
        let page = Page { chapter, c, count: book.len(), id, jump };
        if let Some(op) = chapter_ui(ui, &page, cx, divine) {
            clicked = Some(op);
        }
    }
    if !summary.unpriced.is_empty() {
        let names: Vec<String> = summary.unpriced.iter().map(|op| noun(op, cx)).collect();
        ui.label(RichText::new(format!("Left out, no price: {}", names.join(", "))).small().weak());
    }
    clicked
}

/// A chapter being drawn: which of how many, and the step asked to be scrolled to, (chapter,
/// step), with where the ask is kept.
struct Page<'a> {
    chapter: &'a Chapter,
    c: usize,
    count: usize,
    id: egui::Id,
    jump: Option<(usize, usize)>,
}

/// The colour of an item's name at a rarity, as the game shows it.
fn rarity_colour(ui: &Ui, rarity: Option<&str>) -> egui::Color32 {
    let dark = ui.visuals().dark_mode;
    match rarity {
        Some("magic") => theme::magic(dark),
        Some("rare") => theme::rare(dark),
        _ => ui.visuals().strong_text_color(),
    }
}

/// What the item has at a step, as a small item card: its rarity and base, then each side's
/// mods you need, and a line for each other slot, which holds some other mod or is open.
fn card(ui: &mut Ui, have: &Have, base: &str) {
    let colour = rarity_colour(ui, have.rarity.as_deref());
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, colour.gamma_multiply(0.45)))
        .fill(ui.visuals().extreme_bg_color)
        .corner_radius(4.0)
        .inner_margin(egui::Margin::symmetric(8, 5))
        .show(ui, |ui| ui.vertical(|ui| card_lines(ui, have, base, colour)));
}

fn card_lines(ui: &mut Ui, have: &Have, base: &str, colour: egui::Color32) {
    let dark = ui.visuals().dark_mode;
    ui.set_width(CARD_WIDTH);
    ui.spacing_mut().item_spacing.y = 1.0;
    let rarity = if have.kind == FlowKind::Buy { "normal" } else { have.rarity.as_deref().unwrap_or("normal") };
    ui.vertical_centered(|ui| ui.label(RichText::new(format!("{} {base}", title_case(rarity))).color(colour)));
    if have.slots == [0, 0] {
        ui.label(RichText::new("no mods").small().weak());
        return;
    }
    let mut sides: [Vec<&Held>; 2] = [Vec::new(), Vec::new()];
    let used = |side: &Vec<&Held>| side.iter().map(|held| held.count as usize).sum::<usize>();
    for held in &have.mods {
        let side = match held.side.as_str() {
            "prefix" => 0,
            "suffix" => 1,
            _ if have.slots[0].saturating_sub(used(&sides[0])) >= have.slots[1].saturating_sub(used(&sides[1])) => 0,
            _ => 1,
        };
        sides[side].push(held);
    }
    for (side, name) in ["prefixes", "suffixes"].into_iter().enumerate() {
        ui.label(RichText::new(name).small().weak());
        for held in &sides[side] {
            let role = if held.fractured { "fractured" } else { "target" };
            let times = if held.count > 1 { format!("{}× ", held.count) } else { String::new() };
            let text = RichText::new(format!("{times}{}", held.name.full)).small().color(theme::role(role, dark));
            let response = ui.add(egui::Label::new(text).truncate());
            if held.fractured {
                response.on_hover_text("fractured: no currency can change it");
            }
        }
        let mut shown = used(&sides[side]);
        for (text, _) in have.crafted.iter().filter(|(_, on)| on == ["prefix", "suffix"][side]) {
            ui.add(egui::Label::new(RichText::new(text).small().color(theme::role("bench", dark))).truncate());
            shown += 1;
        }
        match have.filled[side] {
            Some(filled) => {
                let filled = (filled as usize).min(have.slots[side]);
                for _ in shown..filled {
                    ui.label(RichText::new("· another mod").small().italics().weak());
                }
                for _ in shown.max(filled)..have.slots[side] {
                    ui.label(RichText::new("· open").small().weak());
                }
            }
            None => {
                for _ in shown..have.slots[side] {
                    ui.label(RichText::new("· another mod, or open").small().italics().weak());
                }
            }
        }
    }
    if have.veiled {
        ui.label(RichText::new("a veiled mod, to unveil").small().weak());
    }
    if let Some(side) = have.eldritch.as_deref() {
        ui.label(RichText::new(format!("{} implicit dominant: Eldritch currency works on {side}es", dominant(Some(side)))).small().weak());
    }
}

/// Pieces as wrapped text: mods in their colours with the full text on hover, currencies to
/// click, and steps that show what the item has there on hover and scroll there on click
/// (given the page). Returns a currency clicked.
fn sentence(ui: &mut Ui, pieces: &[Piece], cx: &Context, page: Option<&Page>, strong: bool) -> Option<String> {
    let dark = ui.visuals().dark_mode;
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for piece in pieces {
            match piece {
                Piece::Words(text) => {
                    let text = RichText::new(text);
                    ui.label(if strong { text.strong() } else { text });
                }
                Piece::Mod { short, full, role } => {
                    let response = ui.label(RichText::new(short).color(theme::role(role, dark)));
                    if short != full {
                        response.on_hover_text(full);
                    }
                }
                Piece::Op(op) => {
                    let name = RichText::new(noun(op, cx)).color(theme::accent(dark));
                    let response = ui.add(egui::Label::new(name).sense(egui::Sense::click()));
                    if response.on_hover_text("click: list every craft using it").clicked() {
                        clicked = Some(op.clone());
                    }
                }
                Piece::Step(to) => {
                    let link = RichText::new(format!("step {}", to + 1)).strong().underline();
                    let response = ui.add(egui::Label::new(link).sense(egui::Sense::click()));
                    if let Some(page) = page {
                        let step = &page.chapter.steps[*to];
                        let response = response.on_hover_ui(|ui| {
                            sentence(ui, &step.heading, cx, None, true);
                            card(ui, &step.have, cx.base);
                        });
                        if response.clicked() {
                            ui.data_mut(|d| d.insert_temp(page.id, (page.c, *to)));
                        }
                    }
                }
            }
        }
    });
    clicked
}

fn chapter_ui(ui: &mut Ui, page: &Page, cx: &Context, divine: f64) -> Option<String> {
    let dark = ui.visuals().dark_mode;
    let chapter = page.chapter;
    let mut clicked = None;
    if page.count > 1 {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(&chapter.title).size(15.0).strong());
            ui.label(money(Some(chapter.cost), divine));
        });
        let makes = [vec![words("Makes it with ")], chapter.goal.clone(), vec![words(".")]].concat();
        if let Some(op) = sentence(ui, &makes, cx, None, false) {
            clicked = Some(op);
        }
    }
    ui.label(RichText::new("Ingredients").strong().color(theme::accent(dark)));
    egui::Grid::new((page.id, page.c, "ingredients")).num_columns(3).spacing([12.0, 2.0]).show(ui, |ui| {
        for (op, uses, chaos) in &chapter.ingredients {
            ui.label(RichText::new(amount(*uses)).monospace());
            if ui
                .add(egui::Label::new(noun(op, cx)).sense(egui::Sense::click()))
                .on_hover_text("click: list every craft using it")
                .clicked()
            {
                clicked = Some(op.clone());
            }
            ui.label(RichText::new(if *chaos > 0.0 { view::money(Some(*chaos), divine) } else { String::new() }).weak());
            ui.end_row();
        }
    });
    ui.add_space(6.0);
    ui.label(RichText::new("Method").strong().color(theme::accent(dark)));
    let rare = chapter.listed();
    for s in 0..rare {
        if s == chapter.route {
            ui.add_space(6.0);
            ui.label(RichText::new("If it goes another way").strong().color(theme::accent(dark)));
            ui.label(RichText::new("The method's steps send you here when a result is off the usual route.").small().weak());
        }
        if let Some(op) = numbered(ui, s, page, cx) {
            clicked = Some(op);
        }
    }
    if rare < chapter.steps.len() {
        ui.add_space(6.0);
        if let Some(op) = finder(ui, page, rare, cx) {
            clicked = Some(op);
        }
    }
    ui.add_space(10.0);
    clicked
}

/// A step with its number, its card beside it where there is room, and scrolled to when asked;
/// or just the room it took last time while it is out of sight, as laying out every step each
/// frame is most of a frame's work. Returns a currency clicked.
fn numbered(ui: &mut Ui, s: usize, page: &Page, cx: &Context) -> Option<String> {
    let width = ui.available_width();
    let id = page.id.with(("step height", std::ptr::from_ref(page.chapter) as usize, s, width.round() as i32));
    let spacing = ui.spacing().item_spacing.y;
    if let Some(height) = ui.data(|d| d.get_temp::<f32>(id))
        && page.jump != Some((page.c, s))
        && !ui.is_rect_visible(egui::Rect::from_min_size(ui.cursor().min, egui::vec2(width, height)))
    {
        ui.allocate_space(egui::vec2(width, (height - spacing).max(0.0)));
        return None;
    }
    let top = ui.cursor().top();
    let clicked = step_shown(ui, s, page, cx);
    let height = ui.cursor().top() - top;
    ui.data_mut(|d| d.insert_temp(id, height));
    clicked
}

/// The steps too rare to list, found by the item in hand: tick the needs it has (twice for
/// fractured) to see its step and the likeliest way on from there. A link to a rare step picks
/// its needs.
fn finder(ui: &mut Ui, page: &Page, rare: usize, cx: &Context) -> Option<String> {
    let dark = ui.visuals().dark_mode;
    let chapter = page.chapter;
    let id = page.id.with(("find", page.c));
    let mut picked: Vec<u8> = ui.data(|d| d.get_temp(id)).unwrap_or_else(|| vec![0; chapter.needs.len()]);
    if let Some((c, s)) = page.jump
        && c == page.c
        && s >= rare
    {
        picked = vec![0; chapter.needs.len()];
        for &(need, fractured) in &chapter.steps[s].needs {
            picked[need] = if fractured { 2 } else { 1 };
        }
    }
    ui.label(RichText::new("Find your item").strong().color(theme::accent(dark)));
    ui.label(
        RichText::new(format!(
            "{} rarer situations are not listed. Tick what your item has (twice: fractured) to see its step.",
            chapter.steps.len() - rare
        ))
        .small()
        .weak(),
    );
    ui.horizontal_wrapped(|ui| {
        for (need, name) in chapter.needs.iter().enumerate() {
            let (mark, role) = match picked[need] {
                1 => ("✔ ", "target"),
                2 => ("✔ ", "fractured"),
                _ => ("", ""),
            };
            let label = RichText::new(format!("{mark}{}", name.short));
            let label = if role.is_empty() { label.weak() } else { label.color(theme::role(role, dark)) };
            let hint = if picked[need] == 2 { format!("{} (fractured)", name.full) } else { name.full.clone() };
            if ui.add(egui::Button::new(label).selected(picked[need] > 0)).on_hover_text(hint).clicked() {
                picked[need] = (picked[need] + 1) % 3;
            }
        }
    });
    ui.data_mut(|d| d.insert_temp(id, picked.clone()));
    let holds = |step: &Step| {
        let mut ticked: Vec<(usize, bool)> =
            (0..picked.len()).filter(|&need| picked[need] > 0).map(|need| (need, picked[need] == 2)).collect();
        let mut has = step.needs.clone();
        ticked.sort();
        has.sort();
        ticked == has
    };
    let mut matches: Vec<usize> =
        (0..chapter.steps.len()).filter(|&s| holds(&chapter.steps[s]) && chapter.steps[s].have.kind == FlowKind::Work).collect();
    matches.sort_by(|a, b| chapter.steps[*b].visits.total_cmp(&chapter.steps[*a].visits));
    let mut clicked = None;
    if matches.is_empty() {
        ui.label(RichText::new("No step has exactly these.").small().italics().weak());
    }
    for &s in matches.iter().take(FOUND) {
        ui.add_space(4.0);
        if s < rare {
            let said = [vec![words("That is ")], vec![Piece::Step(s)], vec![words(", above.")]].concat();
            if let Some(op) = sentence(ui, &said, cx, Some(page), false) {
                clicked = Some(op);
            }
            continue;
        }
        if let Some(op) = numbered(ui, s, page, cx) {
            clicked = Some(op);
        }
        let mut way = vec![words("Usually then:")];
        let mut at = s;
        for _ in 0..WAY {
            match chapter.steps[at].usual {
                Some(Next::Step(next)) if next != s && !way.contains(&Piece::Step(next)) => {
                    way.extend([words(" ⏵ "), Piece::Step(next)]);
                    if next < rare {
                        way.push(words(", back in the method"));
                        break;
                    }
                    at = next;
                }
                Some(Next::Done) => {
                    way.push(words(" ⏵ done ✔"));
                    break;
                }
                Some(Next::StartOver) => {
                    way.push(words(" ⏵ start over"));
                    break;
                }
                _ => break,
            }
        }
        if let Some(op) = sentence(ui, &way, cx, Some(page), false) {
            clicked = Some(op);
        }
    }
    if matches.len() > FOUND {
        ui.label(RichText::new(format!("and {} more with these, differing in other mods", matches.len() - FOUND)).small().weak());
    }
    clicked
}

fn step_shown(ui: &mut Ui, s: usize, page: &Page, cx: &Context) -> Option<String> {
    let dark = ui.visuals().dark_mode;
    let step = &page.chapter.steps[s];
    let wide = ui.available_width() >= CARD_WIDTH * 2.4;
    let header = ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{:>2}.", s + 1)).strong().monospace().color(theme::accent(dark)));
        let heading: Vec<Piece> =
            step.heading.iter().map(|piece| if let Piece::Words(text) = piece { words(text.clone()) } else { piece.clone() }).collect();
        sentence(ui, &heading, cx, None, true);
        if step.visits < RARE_STEP && step.have.kind != FlowKind::Buy {
            ui.label(RichText::new("  rare").small().weak());
        }
    });
    let rule = header.response.rect;
    let right = ui.max_rect().right();
    ui.painter().hline(rule.left()..=right, rule.bottom() + 2.0, egui::Stroke::new(1.0, theme::accent(dark).gamma_multiply(0.35)));
    ui.add_space(6.0);
    let response = ui.horizontal_top(|ui| {
        ui.add_space(NUMBER_WIDTH);
        let text_width = if wide { ui.available_width() - CARD_WIDTH - 32.0 } else { ui.available_width() };
        let clicked = ui
            .vertical(|ui| {
                ui.set_min_width(text_width);
                ui.set_max_width(text_width);
                if !wide {
                    card(ui, &step.have, cx.base);
                }
                step_ui(ui, step, page, cx)
            })
            .inner;
        if wide {
            ui.with_layout(egui::Layout::right_to_left(Align::Min), |ui| card(ui, &step.have, cx.base));
        }
        clicked
    });
    if page.jump == Some((page.c, s)) {
        header.response.scroll_to_me(Some(Align::TOP));
    }
    ui.add_space(12.0);
    response.inner
}

/// A step: its heading, then each currency with its results a row each. Returns a currency
/// clicked.
fn step_ui(ui: &mut Ui, step: &Step, page: &Page, cx: &Context) -> Option<String> {
    let mut clicked = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    for (l, line) in step.lines.iter().enumerate() {
        if let Some(lead) = lead(&step.lines, l) {
            ui.add_space(4.0);
            if let Some(op) = sentence(ui, &lead, cx, Some(page), false) {
                clicked = Some(op);
            }
        }
        if let Some(op) = sentence(ui, &doing(line, cx), cx, Some(page), false) {
            clicked = Some(op);
        }
        for row in rows(line) {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 0.0;
                ui.add_space(12.0);
                let share = RichText::new(row.share.map_or(String::new(), percent)).small().monospace().weak();
                ui.add_sized([OUTCOME_SHARE_WIDTH, ui.spacing().interact_size.y], egui::Label::new(share));
                let said = [row.what, vec![words("  ⏵ ")], going(&row.next, line)].concat();
                if let Some(op) = sentence(ui, &said, cx, Some(page), false) {
                    clicked = Some(op);
                }
            });
        }
    }
    if step.others >= RARE_USE {
        ui.label(RichText::new("now and then something else is used here").small().weak());
    }
    clicked
}

/// The recipe as plain text, to paste somewhere.
pub fn text(summary: &Summary, cx: &Context) -> String {
    let book = chapters(summary, cx);
    let mut out = String::new();
    if let Some(finish) = book.last() {
        out += &format!("Makes one {} with {}.\nCosts about {:.0}c.\n\n", cx.base, plain(&finish.goal, cx), summary.cost);
    }
    for chapter in &book {
        if book.len() > 1 {
            out += &format!("{} (about {:.0}c)\nMakes it with {}.\n\n", chapter.title, chapter.cost, plain(&chapter.goal, cx));
        }
        out += "Ingredients\n";
        for (op, uses, _) in &chapter.ingredients {
            out += &format!("  {} {}\n", amount(*uses), noun(op, cx));
        }
        out += "\nMethod\n";
        let listed = chapter.listed();
        for (s, step) in chapter.steps.iter().enumerate() {
            if s == chapter.route && s < listed {
                out += "\nIf it goes another way\n";
            }
            if s == listed {
                out += "\nRarer situations (what the item has: what to do, and where it usually goes)\n";
            }
            if s >= listed {
                let usual = step.usual.map_or(String::new(), |next| match next {
                    Next::Step(to) => format!(" → step {}", to + 1),
                    Next::Done => " → done".into(),
                    Next::StartOver => " → start over".into(),
                    Next::Same => String::new(),
                });
                let what = step.lines.first().map_or(String::new(), |line| plain(&doing(line, cx), cx));
                out += &format!("{:>2}. [{}] {}: {what}{usual}\n", s + 1, has(&step.have), plain(&step.heading, cx));
                continue;
            }
            out += &format!("{:>2}. {}  [has: {}]\n", s + 1, plain(&step.heading, cx), has(&step.have));
            for (l, line) in step.lines.iter().enumerate() {
                if let Some(lead) = lead(&step.lines, l) {
                    out += &format!("    {}\n", plain(&lead, cx));
                }
                out += &format!("    {}\n", plain(&doing(line, cx), cx));
                for row in rows(line) {
                    let share = row.share.map_or(String::new(), percent);
                    out += &format!("      {share:>4}  {} → {}\n", plain(&row.what, cx), plain(&going(&row.next, line), cx));
                }
            }
        }
        out += "\n";
    }
    out
}

/// A Kalandralang operation name, or the operation as a comment where there is none.
fn kalandra(op: &str, cx: &Context) -> String {
    let (kind, rest) = op.split_once(':').unwrap_or((op, ""));
    let named = match kind {
        "transmutation" => "transmute",
        "alteration" => "alt",
        "augmentation" => "aug",
        "regal" => "regal",
        "alchemy" => "alch",
        "chaos" => "chaos",
        "exalted" => "exalt",
        "annulment" => "annul",
        "scouring" => "scour",
        "fracturing" => "fracture",
        "eldritch-chaos" => "eldritch_chaos",
        "eldritch-exalted" => "eldritch_exalt",
        "eldritch-annulment" => "eldritch_annul",
        "buy" => return format!("buy \"{}\" ilvl {}", cx.base, cx.item_level),
        "essence" => {
            let (tier, kind) = rest.split_once('-').unwrap_or(("", rest));
            return format!("{tier}_essence_of_{}", kind.replace('-', "_"));
        }
        "harvest" => return format!("harvest_{}", rest.replace('-', "_")),
        "bench" if rest == "remove-crafted" => "remove_crafted_mods",
        "bench" => return format!("craft \"{rest}\""),
        "bench-over" => return format!("craft_over \"{rest}\""),
        _ => return format!("# {}", noun(op, cx)),
    };
    named.into()
}

/// The recipe as a Kalandralang-style script: a label per step, the step's main currency
/// repeated while nothing changes, then a jump for each result. Conditions name mods by their
/// text, and a step's other currencies are comments, so it may need edits to run.
pub fn script(summary: &Summary, cx: &Context) -> String {
    let mut out = String::new();
    for (c, chapter) in chapters(summary, cx).iter().enumerate() {
        out += &format!("# {}: {} (about {:.0}c)\n", chapter.title, plain(&chapter.goal, cx), chapter.cost);
        let label = |s: usize| format!(".p{}s{}", c + 1, s + 1);
        for (s, step) in chapter.steps.iter().enumerate() {
            out += &format!("{}:  # {} (has: {})\n", label(s), plain(&step.heading, cx), has(&step.have));
            let Some((main, others)) = step.lines.split_first() else { continue };
            let op = kalandra(&main.op, cx);
            out += &match () {
                _ if main.augments > 0.0 => format!("  repeat ({op}; if open_prefix or open_suffix then aug) until changed\n"),
                _ if main.spam() => format!("  repeat {op} until changed\n"),
                _ => format!("  {op}\n"),
            };
            for branch in main.exits() {
                let target = match branch.next {
                    Next::Step(to) => format!("goto {}", label(to)),
                    Next::Done => "stop".into(),
                    Next::StartOver | Next::Same => format!("goto {}", label(0)),
                };
                if branch.what.is_empty() {
                    out += &format!("  {target}\n");
                } else {
                    out += &format!("  if \"{}\" then {target}  # {}\n", plain(&branch.what, cx), percent(branch.share));
                }
            }
            for other in others {
                out += &format!("  # with other mods, instead: {}\n", kalandra(&other.op, cx).trim_start_matches("# "));
            }
            if main.stays() > 0.0 {
                out += &format!("  goto {}\n", label(s));
            }
        }
        out += "\n";
    }
    out
}
