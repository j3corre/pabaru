//! Rendering of the individual screens.
//!
//! Views only read and modify the model; anything with side effects (dialogs,
//! background jobs) is returned as an [`Action`] and performed by `app.rs`.

pub mod backup;
pub mod preview;
pub mod restore;

use egui::{Align, Layout, Ui, vec2};

/// A request from the UI that the application performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Open a dialog to pick files (`folders = true` → folders).
    PickInputs { folders: bool },
    /// Prepare (pack) the data again after the inputs changed.
    Prepare,
    /// Export the backup (PDF, or PNG into a folder).
    Export { png: bool },
    /// Pick scans.
    PickScans,
    /// Pick the restore destination folder.
    PickRestoreFolder,
    /// Start the restore.
    Restore,
}

/// Two-column layout; in a narrow window the columns are stacked.
/// `ratio` is the share of width taken by the left column.
/// `state` is passed to both columns in turn (left one first).
pub fn two_columns<S>(
    ui: &mut Ui,
    ratio: f32,
    state: &mut S,
    left: impl FnOnce(&mut Ui, &mut S),
    right: impl FnOnce(&mut Ui, &mut S),
) {
    let gap = 20.0;
    let width = ui.available_width();
    if width < 860.0 {
        left(ui, state);
        ui.add_space(14.0);
        right(ui, state);
        return;
    }
    let left_w = (width - gap) * ratio;
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        ui.allocate_ui_with_layout(vec2(left_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(left_w);
            left(ui, state);
        });
        let right_w = ui.available_width();
        ui.allocate_ui_with_layout(vec2(right_w, 0.0), Layout::top_down(Align::Min), |ui| {
            ui.set_width(right_w);
            right(ui, state);
        });
    });
}
