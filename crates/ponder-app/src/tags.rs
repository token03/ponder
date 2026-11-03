//! Tag search: chips for skills, ascendancies, mods, cluster enchants, items, bases and
//! currencies, picked from fuzzy completions of what is typed. A craft must match a tag of every
//! kind chosen; tags of one kind are alternatives.

use std::collections::BTreeMap;

use egui::{Key, RichText, Ui};
use ponder::pipeline::Estimate;
use serde::{Deserialize, Serialize};

use crate::theme;
use crate::widgets::operation;

/// Completions shown at most.
const SHOWN: usize = 12;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Kind {
    Skill,
    Ascendancy,
    Mod,
    Enchant,
    Item,
    Base,
    Uses,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Skill => "skill",
            Kind::Ascendancy => "ascendancy",
            Kind::Mod => "mod",
            Kind::Enchant => "enchant",
            Kind::Item => "item",
            Kind::Base => "base",
            Kind::Uses => "uses",
        }
    }

    pub fn colour(self, dark: bool) -> egui::Color32 {
        match self {
            Kind::Skill => theme::info(dark),
            Kind::Ascendancy => theme::accent(dark),
            Kind::Mod | Kind::Enchant => theme::role("target", dark),
            Kind::Item | Kind::Base => theme::rare(dark),
            Kind::Uses => theme::good(dark),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Tag {
    pub kind: Kind,
    pub value: String,
}

impl Tag {
    pub fn new(kind: Kind, value: impl Into<String>) -> Tag {
        Tag { kind, value: value.into() }
    }

    fn shown(&self) -> String {
        match self.kind {
            Kind::Uses => operation(&self.value),
            _ => self.value.clone(),
        }
    }
}

fn named(kind: Kind, list: &[(String, usize)]) -> impl Iterator<Item = Tag> + '_ {
    list.iter().map(move |(value, _)| Tag::new(kind, value.clone()))
}

/// Every tag a craft carries.
fn tags(e: &Estimate) -> impl Iterator<Item = Tag> + '_ {
    let mut parts = e.title.split(" · ");
    let item = parts.next().unwrap_or(&e.title).to_string();
    let enchant = (e.kind == "cluster").then(|| parts.next()).flatten().map(|enchant| Tag::new(Kind::Enchant, enchant));
    let uses =
        e.summary.iter().flat_map(|summary| &summary.stages).flat_map(|stage| &stage.uses).map(|(name, _)| Tag::new(Kind::Uses, name));
    named(Kind::Skill, &e.skills)
        .chain(named(Kind::Ascendancy, &e.classes))
        .chain(e.targets.iter().map(|target| Tag::new(Kind::Mod, target.clone())))
        .chain(enchant)
        .chain([Tag::new(Kind::Item, item), Tag::new(Kind::Base, e.base.clone())])
        .chain(named(Kind::Base, &e.worn_bases))
        .chain(uses)
}

/// Whether a craft has a tag of every kind chosen.
pub fn admits(chosen: &[Tag], e: &Estimate) -> bool {
    if chosen.is_empty() {
        return true;
    }
    let has: Vec<Tag> = tags(e).collect();
    let mut kinds: Vec<Kind> = chosen.iter().map(|tag| tag.kind).collect();
    kinds.dedup();
    kinds.iter().all(|&kind| chosen.iter().filter(|tag| tag.kind == kind).any(|tag| has.contains(tag)))
}

/// How well `query` matches `text`, fzf-style: each word's letters in order, scoring words that
/// start where a word of the text starts or run on unbroken; `None` if a word doesn't match.
pub fn fuzzy(query: &str, text: &str) -> Option<i32> {
    let text: Vec<char> = text.to_lowercase().chars().collect();
    query.split_whitespace().map(|word| word_score(&word.to_lowercase().chars().collect::<Vec<_>>(), &text)).sum()
}

fn word_score(word: &[char], text: &[char]) -> Option<i32> {
    let starts = |i: usize| i == 0 || !text[i - 1].is_alphanumeric();
    let unbroken = (0..text.len()).find(|&i| text[i..].starts_with(word));
    if let Some(at) = unbroken {
        return Some(10 * word.len() as i32 + if starts(at) { 15 } else { 0 });
    }
    let mut score = 0;
    let mut last: Option<usize> = None;
    // Scattered letters match almost anything; each should start a word or run on.
    let mut from = 0;
    for &c in word {
        let i = from + text[from..].iter().position(|&t| t == c)?;
        score += 1 + if starts(i) { 6 } else { 0 } + if last == Some(i.wrapping_sub(1)) { 4 } else { 0 };
        score -= last.map_or(0, |last| (i - last - 1).min(4) as i32);
        last = Some(i);
        from = i + 1;
    }
    (score >= 4 * word.len() as i32).then_some(score)
}

/// The tags matching `query` best, each with how many crafts carry it; more common ones first
/// when equally good.
fn complete(estimates: &[Estimate], query: &str, chosen: &[Tag]) -> Vec<(Tag, usize)> {
    let mut counts = BTreeMap::<Tag, usize>::new();
    for e in estimates {
        let mut own: Vec<Tag> = tags(e).collect();
        own.sort();
        own.dedup();
        for tag in own {
            *counts.entry(tag).or_default() += 1;
        }
    }
    let mut scored: Vec<(i32, Tag, usize)> = counts
        .into_iter()
        .filter(|(tag, _)| !chosen.contains(tag))
        .filter_map(|(tag, n)| Some((fuzzy(query, &tag.shown())?, tag, n)))
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(b.2.cmp(&a.2)));
    scored.into_iter().take(SHOWN).map(|(_, tag, n)| (tag, n)).collect()
}

pub fn chip(ui: &mut Ui, tag: &Tag) -> egui::Response {
    let dark = ui.visuals().dark_mode;
    let colour = tag.kind.colour(dark);
    let text = RichText::new(format!("{} {} ×", tag.kind.name(), tag.shown())).small().color(colour);
    ui.add(egui::Button::new(text).fill(colour.gamma_multiply(0.12)).corner_radius(10.0).small()).on_hover_text(format!(
        "{}: {}\nclick to remove",
        tag.kind.name(),
        tag.value
    ))
}

/// The search box: chips for the chosen tags, then a field whose completions add more. Typed
/// words that aren't picked still filter as plain text. Returns whether the filter changed.
pub fn search(ui: &mut Ui, estimates: &[Estimate], chosen: &mut Vec<Tag>, text: &mut String, focus: bool) -> bool {
    let mut changed = false;
    let mut removed = None;
    for (i, tag) in chosen.iter().enumerate() {
        if chip(ui, tag).clicked() {
            removed = Some(i);
        }
    }
    let id = ui.make_persistent_id("tag search");
    let field = ui.add(
        egui::TextEdit::singleline(text)
            .id(id)
            .hint_text("🔍 skill, ascendancy, mod, base, currency… (Ctrl+F)")
            .desired_width(ui.available_width().clamp(160.0, 320.0)),
    );
    if focus {
        field.request_focus();
    }
    changed |= field.changed();
    let typing = field.has_focus() && !text.trim().is_empty();
    if field.has_focus() && text.is_empty() && ui.input(|i| i.key_pressed(Key::Backspace)) && !chosen.is_empty() {
        removed = Some(chosen.len() - 1);
    }
    if let Some(i) = removed {
        chosen.remove(i);
        changed = true;
    }
    let cursor_id = id.with("cursor");
    let mut cursor = ui.data(|d| d.get_temp::<usize>(cursor_id)).unwrap_or(0);
    if field.changed() {
        cursor = 0;
    }
    if typing {
        let options = complete(estimates, text, chosen);
        let (down, up, enter) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                i.key_pressed(Key::Enter),
            )
        });
        if down {
            cursor = (cursor + 1).min(options.len().saturating_sub(1));
        }
        if up {
            cursor = cursor.saturating_sub(1);
        }
        let mut picked = None;
        egui::Area::new(id.with("popup")).order(egui::Order::Foreground).fixed_pos(field.rect.left_bottom() + egui::vec2(0.0, 4.0)).show(
            ui.ctx(),
            |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(420.0);
                    if options.is_empty() {
                        ui.label(RichText::new("no tag matches; filtering by the words").small().weak());
                    }
                    for (i, (tag, n)) in options.iter().enumerate() {
                        let dark = ui.visuals().dark_mode;
                        let row = ui.horizontal(|ui| {
                            ui.label(RichText::new(tag.kind.name()).small().color(tag.kind.colour(dark)));
                            ui.add(egui::Label::new(tag.shown()).truncate().selectable(false));
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.label(RichText::new(n.to_string()).small().weak());
                            });
                        });
                        let response = ui.interact(row.response.rect, id.with(("option", i)), egui::Sense::click());
                        if i == cursor || response.hovered() {
                            ui.painter().rect_filled(
                                row.response.rect.expand(2.0),
                                3.0,
                                ui.visuals().selection.bg_fill.gamma_multiply(0.4),
                            );
                        }
                        if response.clicked() {
                            picked = Some(i);
                        }
                    }
                    ui.label(RichText::new("⏶⏷ choose · Enter add · Backspace on empty removes").small().weak());
                });
            },
        );
        if enter && !options.is_empty() {
            picked = Some(cursor.min(options.len() - 1));
        }
        if let Some(i) = picked {
            chosen.push(options[i].0.clone());
            text.clear();
            changed = true;
            cursor = 0;
            field.request_focus();
        }
    }
    ui.data_mut(|d| d.insert_temp(cursor_id, cursor));
    changed
}
