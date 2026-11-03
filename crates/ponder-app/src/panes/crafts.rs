//! Every craft of the run, one row each: sort by any column, click to select.

use egui::{RichText, Ui};
use egui_extras::{Column, TableBuilder};
use ponder::pipeline::{Depth, Estimate};

use crate::state::State;
use crate::theme;
use crate::view::Col;
use crate::widgets::{empty, money, signed};

/// How far along an estimate's plan is.
enum Status {
    Unsolvable,
    Waiting,
    Provisional,
    Solved,
    Deep,
    Failed,
}

impl Status {
    fn of(state: &State, estimate: &Estimate) -> Status {
        if estimate.failure().is_some() {
            return Status::Failed;
        }
        match &estimate.summary {
            None if estimate.candidate.is_some() => Status::Waiting,
            None => Status::Unsolvable,
            Some(_) if state.quick.contains(&estimate.key) => Status::Provisional,
            Some(summary) if summary.depth == Depth::Deep => Status::Deep,
            Some(_) => Status::Solved,
        }
    }

    fn hint(&self, estimate: &Estimate) -> String {
        match self {
            Status::Unsolvable => "not solved".into(),
            Status::Waiting => "waiting to be solved".into(),
            Status::Provisional => "provisional: a quick plan, or one solved under older prices, until this one is solved".into(),
            Status::Solved => "solved".into(),
            Status::Deep => "solved with every mechanic tried".into(),
            Status::Failed => estimate.failure().unwrap_or_default().into(),
        }
    }

    /// A dot: filled when solved, a ring while not; a star when every mechanic was tried.
    fn paint(&self, ui: &mut Ui) -> egui::Response {
        let dark = ui.visuals().dark_mode;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
        let painter = ui.painter();
        let centre = rect.center();
        match self {
            Status::Unsolvable => {}
            Status::Waiting => {
                painter.circle_stroke(centre, 3.0, egui::Stroke::new(1.0, ui.visuals().weak_text_color()));
            }
            Status::Provisional => {
                painter.circle_stroke(centre, 4.0, egui::Stroke::new(1.5, theme::info(dark)));
            }
            Status::Solved => {
                painter.circle_filled(centre, 4.0, theme::good(dark));
            }
            Status::Failed => {
                painter.circle_filled(centre, 4.0, theme::bad(dark));
            }
            Status::Deep => {
                painter.text(centre, egui::Align2::CENTER_CENTER, "★", egui::FontId::proportional(13.0), theme::accent(dark));
            }
        }
        response
    }
}

pub fn ui(ui: &mut Ui, state: &mut State) {
    if state.data.is_none() {
        empty(ui, "⚒", "Scanning the ladder…");
        return;
    }
    if state.rows.is_empty() {
        empty(ui, "🔍", "No matches");
        return;
    }
    let reveal = std::mem::take(&mut state.reveal);
    let (clicked, sort) = egui::ScrollArea::horizontal().auto_shrink(false).show(ui, |ui| table(ui, state, reveal)).inner;
    if let Some(col) = sort {
        state.saved.sort.click(col);
        state.refilter();
    }
    if let Some((index, twice)) = clicked {
        state.select(index);
        if twice {
            state.pin = Some(index);
        }
    }
}

/// The table; returns the row clicked (and whether twice) and the header clicked.
fn table(ui: &mut Ui, state: &State, reveal: bool) -> (Option<(usize, bool)>, Option<Col>) {
    let mut clicked = None;
    let mut sort = None;
    let estimates = state.estimates();
    let divine = state.divine();
    let dark = ui.visuals().dark_mode;
    let mut table = TableBuilder::new(ui)
        .striped(true)
        .resizable(true)
        .sense(egui::Sense::click())
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(Column::exact(18.0))
        .columns(Column::auto().at_least(28.0), 1)
        .column(Column::auto().at_least(150.0).clip(true))
        .columns(Column::auto().at_least(54.0), Col::ALL.len() - 2);
    if reveal && let Some(row) = state.selected.and_then(|selected| state.rows.iter().position(|&row| row == selected)) {
        table = table.scroll_to_row(row, None);
    }
    table
        .header(26.0, |mut header| {
            header.col(|_| {});
            for col in Col::ALL {
                header.col(|ui| {
                    let active = state.saved.sort.col == col;
                    let arrow = if !active {
                        ""
                    } else if state.saved.sort.descending {
                        " ⏷"
                    } else {
                        " ⏶"
                    };
                    let label = RichText::new(format!("{}{arrow}", col.label())).strong();
                    let label = if active { label.color(theme::accent(dark)) } else { label };
                    if ui.add(egui::Button::new(label).frame(false)).on_hover_text(col.tooltip()).clicked() {
                        sort = Some(col);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(26.0, state.rows.len(), |mut row| {
                let index = state.rows[row.index()];
                let estimate = &estimates[index];
                row.set_selected(state.selected == Some(index));
                row.col(|ui| {
                    let status = Status::of(state, estimate);
                    status.paint(ui).on_hover_text(status.hint(estimate));
                });
                for col in Col::ALL {
                    row.col(|ui| match col {
                        Col::Rank => {
                            ui.label(RichText::new(state.ranks.get(index).copied().unwrap_or_default().to_string()).weak());
                        }
                        Col::Item => {
                            ui.label(RichText::new(&estimate.title).strong());
                            ui.label(RichText::new(&estimate.base).small().weak());
                        }
                        Col::Mods => {
                            ui.label(estimate.targets.len().to_string()).on_hover_text(estimate.targets.join("\n"));
                        }
                        Col::Demand => {
                            ui.label(RichText::new(estimate.demand.to_string()).monospace());
                        }
                        Col::Margin => {
                            ui.label(signed(estimate.margin(), divine, dark));
                        }
                        _ => {
                            ui.label(money(col.value(estimate), divine));
                        }
                    });
                }
                let response = row.response();
                if response.double_clicked() {
                    clicked = Some((index, true));
                } else if response.clicked() {
                    clicked = Some((index, false));
                }
            })
        });
    (clicked, sort)
}
