//! Notefy — a minimal Obsidian-inspired Markdown notes app (egui/eframe).
//! See AGENTS.md for the module map.

mod app;
mod config;
mod live;
mod semantic;
mod theme;
mod vault;

use app::Notefy;
use config::load_vault_config;
use eframe::egui;
use std::path::PathBuf;
pub(crate) use theme::icon;

fn main() -> eframe::Result {
    let vault = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .or_else(load_vault_config)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
            PathBuf::from(home).join("Notefy")
        });
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1100.0, 720.0])
            .with_min_inner_size([600.0, 400.0])
            .with_title("Notefy"),
        ..Default::default()
    };
    eframe::run_native(
        "Notefy",
        options,
        Box::new(|cc| Ok(Box::new(Notefy::new(cc, vault)))),
    )
}
