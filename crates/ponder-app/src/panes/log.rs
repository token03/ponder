//! What is running, and what happened.

use egui::{RichText, Ui};

use crate::state::State;

pub fn ui(ui: &mut Ui, state: &mut State) {
    for (name, duration) in state.jobs.running() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label(RichText::new(name).strong());
            ui.label(RichText::new(format!("{:.0}s", duration.as_secs_f64())).monospace().weak());
        });
    }
    let height = ui.text_style_height(&egui::TextStyle::Body);
    egui::ScrollArea::vertical().auto_shrink(false).stick_to_bottom(true).show_rows(ui, height, state.log.len(), |ui, shown| {
        for (time, message) in &state.log[shown] {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{:>6.1}s", time.duration_since(state.started).as_secs_f64())).monospace().weak());
                ui.add(egui::Label::new(message).truncate()).on_hover_text(message);
            });
        }
    });
}
