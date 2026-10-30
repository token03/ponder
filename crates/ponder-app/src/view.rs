//! What the tables show, without drawing anything: filters, sort orders and number formats.

use std::cmp::Ordering;

use ponder::pipeline::{Estimate, cmp_ladder};
use serde::{Deserialize, Serialize};

use crate::filter::ItemSelection;

/// Chaos for display: divines above one divine, whole chaos above 10.
pub fn money(x: Option<f64>, divine: f64) -> String {
    let Some(x) = x else { return "–".into() };
    if divine > 0.0 && x.abs() >= divine {
        let d = x / divine;
        return if d.abs() >= 100.0 { format!("{}d", thousands(d.round())) } else { format!("{d:.1}d") };
    }
    if x.abs() < 10.0 { format!("{x:.1}c") } else { format!("{}c", thousands(x.round())) }
}

fn thousands(x: f64) -> String {
    let digits = format!("{:.0}", x.abs());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if x < 0.0 { format!("-{out}") } else { out }
}

pub fn hours(h: f64) -> String {
    if h < 1.0 { format!("{:.0} min", h * 60.0) } else { format!("{h:.1} h") }
}

/// Which rows the crafts table shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    pub items: ItemSelection,
    /// words that must all appear in the title, base, mods or skills
    pub text: String,
    /// one base group (an estimate's title for gear), or every group when empty
    pub group: String,
    pub min_demand: usize,
    /// cheapest way to make it, in chaos; 0 is no limit
    pub max_cost: f64,
    pub only_market: bool,
    pub hide_errors: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Filters {
            items: ItemSelection::default(),
            text: String::new(),
            group: String::new(),
            min_demand: 0,
            max_cost: 0.0,
            only_market: false,
            hide_errors: true,
        }
    }
}

impl Filters {
    pub fn is_default(&self) -> bool {
        *self == Filters::default()
    }

    pub fn admits(&self, e: &Estimate) -> bool {
        if e.demand < self.min_demand {
            return false;
        }
        if self.hide_errors && e.failure().is_some() {
            return false;
        }
        if !self.group.is_empty() && e.title != self.group {
            return false;
        }
        if self.max_cost > 0.0 && e.cheapest().is_none_or(|c| c > self.max_cost) {
            return false;
        }
        if self.only_market && e.market.is_none() {
            return false;
        }
        let words: Vec<String> = self.text.split_whitespace().map(str::to_lowercase).collect();
        if words.is_empty() {
            return true;
        }
        let hay = haystack(e);
        words.iter().all(|w| hay.contains(w.as_str()))
    }
}

fn haystack(e: &Estimate) -> String {
    let mut s = format!("{} {} {}", e.title, e.base, e.targets.join(" "));
    for r in &e.rows {
        s.push(' ');
        s.push_str(&r.text);
    }
    for k in &e.skills {
        s.push(' ');
        s.push_str(k);
    }
    s.to_lowercase()
}

/// The crafts table's columns, which are also its sort keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Col {
    Rank,
    Item,
    Craft,
    Cheapest,
    Market,
    Margin,
    Hole,
    Demand,
    Mods,
}

impl Col {
    pub const ALL: [Col; 9] =
        [Col::Rank, Col::Item, Col::Demand, Col::Cheapest, Col::Market, Col::Margin, Col::Hole, Col::Craft, Col::Mods];

    pub fn label(self) -> &'static str {
        match self {
            Col::Rank => "#",
            Col::Item => "Item",
            Col::Craft => "Craft",
            Col::Cheapest => "Cheapest",
            Col::Market => "Market",
            Col::Margin => "Margin",
            Col::Hole => "Hole",
            Col::Demand => "Demand",
            Col::Mods => "Targets",
        }
    }

    pub fn tooltip(self) -> &'static str {
        match self {
            Col::Rank => "best first: market holes, then what most ladder characters wear",
            Col::Item => "the base group (every base that rolls the same mods)",
            Col::Craft => "expected cost from a fresh base: currency, bench crafts and clicking time",
            Col::Cheapest => "crafting and expected base acquisition; unknown until the base is quoted",
            Col::Market => "what the finished item sells for on trade",
            Col::Margin => "market price minus the cheapest way to make it",
            Col::Hole => "demand the market doesn't meet: wearers per listing, weighted by margin",
            Col::Demand => "ladder characters wearing this",
            Col::Mods => "the mods it needs",
        }
    }

    /// Whether a click on the header sorts high to low first.
    pub fn descending_first(self) -> bool {
        !matches!(self, Col::Rank | Col::Item | Col::Cheapest | Col::Craft | Col::Mods)
    }

    pub fn value(self, e: &Estimate) -> Option<f64> {
        match self {
            Col::Craft => e.craft_cost(),
            Col::Cheapest => e.cheapest(),
            Col::Market => e.market.map(|m| m.0),
            Col::Margin => e.margin(),
            Col::Hole => e.hole(),
            Col::Demand => Some(e.demand as f64),
            Col::Rank | Col::Item | Col::Mods => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Sort {
    pub col: Col,
    pub descending: bool,
}

impl Default for Sort {
    fn default() -> Self {
        Sort { col: Col::Rank, descending: false }
    }
}

impl Sort {
    /// A click on a header: flip the direction, or sort by that column.
    pub fn click(&mut self, col: Col) {
        if self.col == col {
            self.descending = !self.descending;
        } else {
            *self = Sort { col, descending: col.descending_first() };
        }
    }

    fn cmp(&self, a: &Estimate, b: &Estimate) -> Ordering {
        let ord = match self.col {
            Col::Rank => cmp_ladder(a, b),
            Col::Item => a.title.cmp(&b.title).then_with(|| a.targets.cmp(&b.targets)),
            Col::Mods => a.targets.len().cmp(&b.targets.len()).then_with(|| a.targets.cmp(&b.targets)),
            col => {
                // missing values always go last, whichever way the column sorts
                return match (col.value(a), col.value(b)) {
                    (Some(x), Some(y)) => {
                        let o = x.total_cmp(&y);
                        if self.descending { o.reverse() } else { o }
                    }
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                }
                .then_with(|| cmp_ladder(a, b));
            }
        };
        if self.descending { ord.reverse() } else { ord }
    }
}

/// Indices of the estimates the table shows, in display order.
pub fn visible(estimates: &[Estimate], filters: &Filters, sort: Sort) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..estimates.len()).filter(|&i| filters.admits(&estimates[i])).collect();
    idx.sort_by(|&a, &b| sort.cmp(&estimates[a], &estimates[b]));
    idx
}

/// Each estimate's place in the pipeline's own ranking (1 = best).
pub fn ranks(estimates: &[Estimate]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..estimates.len()).collect();
    idx.sort_by(|&a, &b| cmp_ladder(&estimates[a], &estimates[b]));
    let mut rank = vec![0; estimates.len()];
    for (r, i) in idx.into_iter().enumerate() {
        rank[i] = r + 1;
    }
    rank
}
