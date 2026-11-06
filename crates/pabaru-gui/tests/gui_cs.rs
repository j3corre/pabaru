//! GUI test of the Czech localisation.
//!
//! The UI language is a process-wide setting, so the Czech checks live in
//! their own test binary and never race with the English tests in `gui.rs`.

use std::path::PathBuf;

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use pabaru_gui::i18n::{Lang, set_lang};
use pabaru_gui::model::Tab;
use pabaru_gui::{Dialogs, PabaruApp};

/// Dialogs that never return anything (no dialog is opened in this test).
struct NoDialogs;

impl Dialogs for NoDialogs {
    fn pick_files(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn pick_folders(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn pick_scans(&self) -> Vec<PathBuf> {
        Vec::new()
    }
    fn pick_folder(&self, _title: &str) -> Option<PathBuf> {
        None
    }
    fn save_pdf(&self, _name: &str) -> Option<PathBuf> {
        None
    }
}

#[test]
fn czech_labels() {
    set_lang(Lang::Cs);
    let mut h: Harness<'static, PabaruApp> = Harness::builder()
        .with_size(egui::vec2(1240.0, 900.0))
        .build_eframe(|cc| PabaruApp::with_dialogs(&cc.egui_ctx, Box::new(NoDialogs)));
    h.run_steps(2);

    // Start screen of the "Back up" tab.
    h.get_by_label("Zálohovat");
    h.get_by_label("Přetáhněte sem soubory nebo složky");
    h.get_by_label("Vybrat složku…");

    // Switch to the "Restore" tab.
    h.get_by_label("Obnovit").click();
    h.run_steps(2);
    assert_eq!(h.state().tab, Tab::Restore);
    h.get_by_label("Přetáhněte sem naskenované stránky");
    h.get_by_label("Vybrat skeny…");

    // Czech uses a decimal comma.
    assert_eq!(pabaru_gui::model::human_size(1536), "1,5 KB");
}
