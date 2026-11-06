//! Launches the Pabaru desktop application.

// Do not show a console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use pabaru_gui::i18n::{self, tr};

fn main() -> eframe::Result {
    // Pick the UI language before any text is created (the window title too).
    i18n::set_lang(i18n::detect());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(tr(
                "Pabaru – data backup to paper",
                "Pabaru – záloha dat na papír",
            ))
            .with_inner_size([1180.0, 800.0])
            .with_min_inner_size([760.0, 560.0])
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
                    .unwrap_or_default(),
            ),
        ..Default::default()
    };
    eframe::run_native(
        "Pabaru",
        options,
        Box::new(|cc| Ok(Box::new(pabaru_gui::PabaruApp::new(cc)))),
    )
}
