//! The scan as a funnel: characters read, the rare items they wear, the mod combinations found,
//! the crafts costed and how they came out; then why crafts fail and what kinds there are.

use std::collections::BTreeMap;

use egui::{RichText, Ui};

use crate::state::State;
use crate::tags::{Kind, Tag};
use crate::theme;
use crate::widgets::{bars, empty, section, stat};

pub fn ui(ui: &mut Ui, state: &mut State) {
    let Some(data) = &state.data else {
        empty(ui, "⛃", "Scan the ladder first");
        return;
    };
    let report = &data.report;
    let dark = ui.visuals().dark_mode;
    let estimates = &report.estimates;
    let solved = estimates.iter().filter(|e| e.summary.is_some()).count();
    let failed = estimates.iter().filter(|e| e.failure().is_some()).count();
    let picked = egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let steps = [
                (report.characters, "characters", "ladder characters read (poe.ninja)"),
                (report.items, "rare items", "rare gear and jewels they wear"),
                (
                    report.archetypes + report.cluster_archetypes,
                    "combos",
                    "mod combinations worn together, merged across bases sharing a mod pool",
                ),
                (estimates.len(), "crafts", "the combinations costed: the most worn, the top of each base group and niche ones"),
                (solved, "solved", "with a plan"),
            ];
            for (i, (count, name, hint)) in steps.into_iter().enumerate() {
                if i > 0 {
                    ui.label(RichText::new("⏵").size(18.0).weak());
                }
                stat(ui, name, RichText::new(crate::view::count(count as f64)).strong(), hint);
            }
            if failed > 0 {
                ui.label(RichText::new(format!("⚠ {failed} failed")).color(theme::bad(dark)));
            }
        });
        let mut kinds = BTreeMap::<String, usize>::new();
        for estimate in estimates {
            *kinds.entry(estimate.title.split(" · ").next().unwrap_or(&estimate.title).to_string()).or_default() += 1;
        }
        section(ui, "Crafts by item");
        let kinds = sorted(kinds);
        let picked = bars(ui, &kinds, estimates.len(), theme::info(dark), true).map(|i| kinds[i].0.clone());
        let mut reasons = BTreeMap::<String, usize>::new();
        for error in estimates.iter().filter_map(|e| e.failure()) {
            *reasons.entry(reason(error)).or_default() += 1;
        }
        if !reasons.is_empty() {
            section(ui, "Why crafts fail");
            bars(ui, &sorted(reasons), failed, theme::bad(dark), false);
        }
        picked
    });
    if let Some(item) = picked.inner {
        state.saved.filters.add(Tag::new(Kind::Item, item));
        state.refilter();
    }
}

fn sorted(counts: BTreeMap<String, usize>) -> Vec<(String, usize)> {
    let mut rows: Vec<_> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    rows
}

/// An error without its particulars, so alike ones count together.
fn reason(error: &str) -> String {
    let general = error.split(';').next().unwrap_or(error);
    let general: String = general.chars().take_while(|c| *c != '[' && *c != '"').collect();
    general.split_whitespace().filter(|word| !word.starts_with(|c: char| c.is_ascii_digit())).collect::<Vec<_>>().join(" ")
}
