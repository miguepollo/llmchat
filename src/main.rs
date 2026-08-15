#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api;
mod app;
mod config;
mod files;
mod markdown;
mod types;

use eframe::egui;

fn main() -> eframe::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("no se pudo crear el runtime de tokio");
    let handle = runtime.handle().clone();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1000.0, 700.0])
            .with_min_inner_size([640.0, 480.0])
            .with_title("LLMchat"),
        ..Default::default()
    };

    eframe::run_native(
        "llmchat",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, handle)))),
    )
}
