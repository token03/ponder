//! What the tables show, without drawing anything: filters, sort orders and number formats.

use std::cmp::Ordering;

use ponder::pipeline::{Estimate, cmp_priority};
use serde::{Deserialize, Serialize};

/// Chaos as render.py prints it: divines above one divine, whole chaos above 10.
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

pub fn percent(p: f64) -> String {
    if p >= 0.1 { format!("{:.0}%", p * 100.0) } else if p >= 0.001 { format!("{:.1}%", p * 100.0) } else { "<0.1%".into() }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Kind {
    #[default]
    All,
    Gear,
    Cluster,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::All, Kind::Gear, Kind::Cluster];

    pub fn label(self) -> &'static str {
        match self {
            Kind::All => "All",
            Kind::Gear => "Gear",
            Kind::Cluster => "Clusters",
        }
    }

    fn admits(self, e: &Estimate) -> bool {
        match self {
            Kind::All => true,
            Kind::Gear => e.kind == "gear",
            Kind::Cluster => e.kind == "cluster",
        }
    }
}

/// Which rows the crafts table shows.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Filters {
    pub kind: Kind,
    /// words that must all appear in the title, base, mods or skills
    pub text: String,
    /// one base group (an estimate's title for gear), or every group when empty
    pub group: String,
    pub min_demand: usize,
    /// cheapest way to make it, in chaos; 0 is no limit
    pub max_cost: f64,
    pub only_market: bool,
    pub only_fractures: bool,
    pub only_recomb: bool,
    pub only_sales: bool,
    pub hide_errors: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Filters {
            kind: Kind::All,
            text: String::new(),
            group: String::new(),
            min_demand: 0,
            max_cost: 0.0,
            only_market: false,
            only_fractures: false,
            only_recomb: false,
            only_sales: false,
            hide_errors: true,
        }
    }
}

impl Filters {
    pub fn is_default(&self) -> bool {
        *self == Filters::default()
    }

    pub fn admits(&self, e: &Estimate) -> bool {
        if !self.kind.admits(e) || e.demand < self.min_demand {
            return false;
        }
        if self.hide_errors && e.error.is_some() {
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
        if self.only_fractures && e.best_fracture().and_then(|f| f.total()).is_none() {
            return false;
        }
        if self.only_recomb && e.recomb_total().is_none() {
            return false;
        }
        if self.only_sales && e.smaller_products.is_empty() {
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
    Fractured,
    Recomb,
    Cheapest,
    Market,
    Margin,
    Hole,
    Demand,
    Mods,
}

impl Col {
    pub const ALL: [Col; 11] =
        [Col::Rank, Col::Item, Col::Craft, Col::Fractured, Col::Recomb, Col::Cheapest, Col::Market, Col::Margin, Col::Hole, Col::Demand, Col::Mods];

    pub fn label(self) -> &'static str {
        match self {
            Col::Rank => "#",
            Col::Item => "Item",
            Col::Craft => "Craft",
            Col::Fractured => "Fractured",
            Col::Recomb => "Recomb",
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
            Col::Rank => "best first: known margins, then the dearest crafts",
            Col::Item => "the base group (every base that rolls the same mods)",
            Col::Craft => "expected cost from a fresh base: currency, bench crafts and clicking time",
            Col::Fractured => "cheapest total starting from a base with one target already fractured",
            Col::Recomb => "cheapest recombinator tree of clean donors",
            Col::Cheapest => "the cheapest of crafting, fractured and recombinator",
            Col::Market => "what the finished item sells for on trade",
            Col::Margin => "market price minus the cheapest way to make it",
            Col::Hole => "demand the market doesn't meet: wearers per listing, weighted by margin",
            Col::Demand => "ladder characters wearing this",
            Col::Mods => "the mods it needs",
        }
    }

    /// Whether a click on the header sorts high to low first.
    pub fn descending_first(self) -> bool {
        !matches!(self, Col::Rank | Col::Item | Col::Cheapest | Col::Craft | Col::Fractured | Col::Recomb | Col::Mods)
    }

    pub fn value(self, e: &Estimate) -> Option<f64> {
        match self {
            Col::Craft => e.total(),
            Col::Fractured => e.best_fracture().and_then(|f| f.total()),
            Col::Recomb => e.recomb_total(),
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
            Col::Rank => cmp_priority(a, b),
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
                .then_with(|| cmp_priority(a, b));
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
    idx.sort_by(|&a, &b| cmp_priority(&estimates[a], &estimates[b]));
    let mut rank = vec![0; estimates.len()];
    for (r, i) in idx.into_iter().enumerate() {
        rank[i] = r + 1;
    }
    rank
}

/// The base groups (gear) and enchants (clusters) present, for the group filter.
pub fn groups(estimates: &[Estimate]) -> Vec<String> {
    let mut g: Vec<String> = estimates.iter().map(|e| e.title.clone()).collect();
    g.sort();
    g.dedup();
    g
}

#[cfg(test)]
mod tests {
    use super::*;

    fn est(kind: &str, title: &str, cost: f64, demand: usize) -> Estimate {
        let mut e = Estimate::new(kind, title, vec!["Life".into()]);
        e.base = "Vermillion Ring".into();
        e.demand = demand;
        e.plan = Some(ponder_core::Plan { cost, ..Default::default() });
        e
    }

    #[test]
    fn money_matches_render_py() {
        assert_eq!(money(None, 300.0), "–");
        assert_eq!(money(Some(4.25), 300.0), "4.2c");
        assert_eq!(money(Some(1234.0), 3000.0), "1,234c");
        assert_eq!(money(Some(450.0), 300.0), "1.5d");
        assert_eq!(money(Some(60000.0), 300.0), "200d");
        assert_eq!(money(Some(-15.0), 300.0), "-15c");
    }

    #[test]
    fn filters_and_sorts() {
        let es = vec![est("gear", "Ring", 900.0, 40), est("cluster", "Medium cluster", 50.0, 10), est("gear", "Amulet", 20.0, 90)];
        let all = Filters { hide_errors: false, ..Filters::default() };
        assert_eq!(visible(&es, &all, Sort { col: Col::Cheapest, descending: false }), vec![2, 1, 0]);
        assert_eq!(visible(&es, &all, Sort { col: Col::Demand, descending: true }), vec![2, 0, 1]);
        let gear = Filters { kind: Kind::Gear, ..all.clone() };
        assert_eq!(visible(&es, &gear, Sort::default()).len(), 2);
        let cheap = Filters { max_cost: 100.0, ..all.clone() };
        assert_eq!(visible(&es, &cheap, Sort { col: Col::Item, descending: false }), vec![2, 1]);
        let text = Filters { text: "vermillion RING".into(), ..all.clone() };
        assert_eq!(visible(&es, &text, Sort::default()).len(), 3);
        let group = Filters { group: "Ring".into(), ..all };
        assert_eq!(visible(&es, &group, Sort::default()), vec![0]);
        assert_eq!(groups(&es), vec!["Amulet", "Medium cluster", "Ring"]);
    }

    #[test]
    fn missing_values_sort_last_both_ways() {
        let mut es = vec![est("gear", "A", 10.0, 1), est("gear", "B", 20.0, 1)];
        es[0].market = Some((100.0, 3));
        for descending in [false, true] {
            assert_eq!(visible(&es, &Filters::default(), Sort { col: Col::Market, descending })[0], 0);
        }
        let mut s = Sort::default();
        s.click(Col::Margin);
        assert!(s.descending);
        s.click(Col::Margin);
        assert!(!s.descending);
    }
}
