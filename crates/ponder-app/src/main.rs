//! `ponder-app`: the desktop app. Runs the ladder pipeline in the background and lays its
//! results out in tiles you can drag around: crafts, item goals and policy operations.

// No console window behind the app on Windows (release builds; debug keeps it for logs).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod filter;
mod jobs;
mod panes;
mod state;
mod view;
mod widgets;

use std::path::Path;

/// Local data and caches are relative to the project directory:
/// start there when launched from inside it.
fn find_project_dir() {
    let Ok(cwd) = std::env::current_dir() else {
        return;
    };
    if let Some(dir) = cwd.ancestors().find(|d| Path::new(d).join("data").is_dir() || Path::new(d).join(".cache").is_dir()) {
        let _ = std::env::set_current_dir(dir);
    }
}

/// Without a console a panic would vanish: also write it to `ponder-crash.log`.
fn log_panics() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = std::fs::write("ponder-crash.log", format!("{info}\n{}", std::backtrace::Backtrace::force_capture()));
        default(info);
    }));
}

fn main() -> eframe::Result {
    log_panics();
    if let Some(dir) = std::env::args().nth(1) {
        std::env::set_current_dir(&dir).unwrap_or_else(|e| panic!("can't open {dir}: {e}"));
    } else {
        find_project_dir();
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("ponder")
            .with_inner_size([1500.0, 900.0])
            .with_min_inner_size([900.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native("ponder", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
