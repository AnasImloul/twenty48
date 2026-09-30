//! A desktop UI for the twenty48 engine and its expectimax agent.
//!
//! Run with `cargo run --release -p twenty48-gui`. Arrow keys or WASD play by
//! hand; the agent plays from the panel on the right, where its depth and
//! probability floor can be changed while it runs.

#![forbid(unsafe_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod render;
mod theme;
mod worker;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([900.0, 700.0])
            .with_min_inner_size([620.0, 520.0])
            .with_title("twenty48"),
        ..Default::default()
    };

    eframe::run_native(
        "twenty48",
        options,
        Box::new(|cc| Ok(Box::new(app::App::new(&cc.egui_ctx)))),
    )
}
