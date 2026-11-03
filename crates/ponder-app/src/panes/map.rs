//! The crafts shown, by what they cost to make and how many wear them: the cheap, popular
//! corner is where to look. Click a point to select it.

use egui::Ui;
use egui_plot::{GridMark, HoverPosition, Plot, PlotPoints, Points};

use crate::state::State;
use crate::widgets::empty;
use crate::{theme, view};

pub fn ui(ui: &mut Ui, state: &mut State) {
    let estimates = state.estimates();
    let shown: Vec<(usize, [f64; 2])> = state
        .rows
        .iter()
        .filter_map(|&index| {
            let estimate = &estimates[index];
            let cost = estimate.craft_cost().filter(|cost| *cost > 0.0)?;
            Some((index, [cost.log10(), estimate.demand as f64]))
        })
        .collect();
    if shown.is_empty() {
        empty(ui, "📈", "Nothing solved yet");
        return;
    }
    let dark = ui.visuals().dark_mode;
    let divine = state.divine();
    let colours = [theme::good(dark), theme::bad(dark), theme::info(dark).gamma_multiply(0.75)];
    let group = |index: usize| match estimates[index].margin() {
        Some(margin) if margin > 0.0 => 0,
        Some(_) => 1,
        None => 2,
    };
    let groups: Vec<Vec<(usize, [f64; 2])>> =
        (0..colours.len()).map(|g| shown.iter().copied().filter(|(index, _)| group(*index) == g).collect()).collect();
    let titles: Vec<Vec<String>> = groups
        .iter()
        .map(|members| {
            members
                .iter()
                .map(|&(index, _)| {
                    let estimate = &estimates[index];
                    format!("{}\n{} · {} wear it", estimate.title, view::money(estimate.craft_cost(), divine), estimate.demand)
                })
                .collect()
        })
        .collect();
    let selected = state.selected.and_then(|selected| shown.iter().find(|(index, _)| *index == selected)).map(|(_, point)| *point);
    ui.horizontal(|ui| {
        for (colour, text) in colours.iter().zip(["sells above cost", "below cost", "no market price"]) {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 4.0, *colour);
            ui.label(egui::RichText::new(text).small().weak());
        }
    });
    let response = Plot::new("market map")
        .x_axis_label("craft cost")
        .y_axis_label("characters wearing it")
        .x_axis_formatter(move |mark: GridMark, _| view::money(Some(10f64.powf(mark.value)), divine))
        .label_formatter(move |hover: &HoverPosition<'_>| match hover {
            HoverPosition::NearDataPoint { index, plot_name, .. } => {
                let group: usize = plot_name.strip_prefix("group ")?.parse().ok()?;
                titles.get(group)?.get(*index).cloned()
            }
            HoverPosition::Elsewhere { .. } => None,
        })
        .show(ui, |plot| {
            for (g, members) in groups.iter().enumerate() {
                if !members.is_empty() {
                    let points: Vec<[f64; 2]> = members.iter().map(|(_, point)| *point).collect();
                    plot.points(Points::new(format!("group {g}"), PlotPoints::from(points)).radius(3.5).color(colours[g]));
                }
            }
            if let Some(point) = selected {
                plot.points(Points::new("selected", vec![point]).radius(7.0).filled(false).color(theme::accent(dark)));
            }
        });
    if response.response.clicked()
        && let Some(pointer) = response.response.interact_pointer_pos()
    {
        let nearest = shown
            .iter()
            .map(|(index, [x, y])| (*index, response.transform.position_from_point(&egui_plot::PlotPoint::new(*x, *y)).distance(pointer)))
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((index, distance)) = nearest
            && distance < 12.0
        {
            state.select(index);
            state.reveal = true;
        }
    }
}
