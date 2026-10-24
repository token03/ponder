//! `ponder-app`: the desktop app. Runs the ladder pipeline in the background and lays its
//! results out in tiles you can drag around: the crafts table, the selected item, its recipe,
//! fractured bases, product tree, recombinator tree, finishing a pasted item.

mod app;
mod jobs;
mod panes;
mod state;
mod view;
mod widgets;

use std::path::Path;

/// The caches (`.cache/ladder`, `.cache/ninja`, ...) are relative to the project directory:
/// start there when launched from inside it.
fn find_project_dir() {
    let Ok(cwd) = std::env::current_dir() else { return };
    if let Some(dir) = cwd.ancestors().find(|d| Path::new(d).join(".cache").is_dir()) {
        let _ = std::env::set_current_dir(dir);
    }
}

fn main() -> eframe::Result {
    if let Some(dir) = std::env::args().nth(1) {
        std::env::set_current_dir(&dir).unwrap_or_else(|e| panic!("can't open {dir}: {e}"));
    } else {
        find_project_dir();
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("ponder").with_inner_size([1500.0, 900.0]).with_min_inner_size([900.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native("ponder", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
