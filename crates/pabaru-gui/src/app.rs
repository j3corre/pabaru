//! Main application: top bar, tabs, drag & drop, handling of actions and jobs.

use std::path::PathBuf;
use std::sync::Arc;

use egui::{
    Align, CentralPanel, CornerRadius, Frame, Layout, Margin, Panel, RichText, ScrollArea, Stroke,
    ThemePreference, Ui, vec2,
};

use crate::dialogs::{Dialogs, NativeDialogs};
use crate::i18n::{self, tr};
use crate::model::{BackupModel, RestoreModel, Tab, Task};
use crate::theme::{self, Palette};
use crate::views::{self, Action};
use crate::widgets::segmented;
use crate::worker::{Job, JobResult, RestoreTarget, Worker};

/// Maximum content width (content is centred on wide monitors).
const MAX_CONTENT_WIDTH: f32 = 1240.0;

/// The whole application.
pub struct PabaruApp {
    pub tab: Tab,
    pub backup: BackupModel,
    pub restore: RestoreModel,
    worker: Worker,
    dialogs: Box<dyn Dialogs>,
    theme: ThemePreference,
}

impl PabaruApp {
    /// Creates the app from eframe (native dialogs, installed style).
    ///
    /// Also detects the UI language from the OS locale (`PABARU_LANG` overrides).
    /// Tests use [`Self::with_dialogs`] and set the language themselves.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        i18n::set_lang(i18n::detect());
        Self::with_dialogs(&cc.egui_ctx, Box::new(NativeDialogs))
    }

    /// Creates the app with custom dialogs (tests).
    pub fn with_dialogs(ctx: &egui::Context, dialogs: Box<dyn Dialogs>) -> Self {
        theme::install(ctx);
        let repaint_ctx = ctx.clone();
        Self {
            tab: Tab::Backup,
            backup: BackupModel::default(),
            restore: RestoreModel::default(),
            worker: Worker::new(Arc::new(move || repaint_ctx.request_repaint())),
            dialogs,
            theme: ThemePreference::System,
        }
    }

    /// Number of background jobs still running.
    pub fn pending_jobs(&self) -> usize {
        self.worker.pending()
    }

    /// Handles dropped files according to the active tab.
    pub fn drop_paths(&mut self, paths: Vec<PathBuf>) {
        match self.tab {
            Tab::Backup => {
                if self.backup.add_inputs(paths) {
                    self.handle(Action::Prepare);
                }
            }
            Tab::Restore => {
                for path in self.restore.add_scans(paths) {
                    self.worker.submit(Job::DecodeScan { path });
                }
            }
        }
    }

    /// Performs an action requested by the UI.
    pub fn handle(&mut self, action: Action) {
        match action {
            Action::PickInputs { folders } => {
                let paths = if folders {
                    self.dialogs.pick_folders()
                } else {
                    self.dialogs.pick_files()
                };
                if !paths.is_empty() {
                    self.tab = Tab::Backup;
                    self.drop_paths(paths);
                }
            }
            Action::Prepare => {
                if !self.backup.inputs.is_empty() {
                    self.worker.submit(Job::Prepare {
                        generation: self.backup.generation,
                        inputs: self.backup.inputs.clone(),
                    });
                }
            }
            Action::Export { png } => self.start_export(png),
            Action::PickScans => {
                let paths = self.dialogs.pick_scans();
                if !paths.is_empty() {
                    self.tab = Tab::Restore;
                    self.drop_paths(paths);
                }
            }
            Action::PickRestoreFolder => {
                if let Some(dir) = self
                    .dialogs
                    .pick_folder(tr("Where to restore the files", "Kam obnovit soubory"))
                {
                    self.restore.folder = Some(dir);
                }
            }
            Action::Restore => {
                if self.restore.restore_blocker().is_some() {
                    return;
                }
                let Some(dir) = self.restore.target_dir() else {
                    return;
                };
                let target = RestoreTarget::Folder(dir);
                self.restore.restore = Task::Running;
                self.worker.submit(Job::Restore {
                    decoder: Box::new(self.restore.decoder.clone()),
                    password: self.restore.password_opt(),
                    target,
                });
            }
        }
    }

    fn start_export(&mut self, png: bool) {
        if self.backup.export_blocker().is_some() {
            return;
        }
        let (Some(prepared), Some(opts)) = (
            self.backup.prepared.done().cloned(),
            self.backup.encode_options(),
        ) else {
            return;
        };
        let path = if png {
            self.dialogs
                .pick_folder(tr("Folder for PNG pages", "Složka pro stránky PNG"))
        } else {
            self.dialogs.save_pdf(&self.backup.suggested_file_name())
        };
        let Some(path) = path else { return };
        self.backup.export = Task::Running;
        self.worker.submit(Job::Export {
            prepared,
            opts: Box::new(opts),
            path,
            png,
        });
    }

    /// Collects results of finished jobs.
    pub fn process_results(&mut self) {
        for result in self.worker.poll() {
            match result {
                JobResult::Prepared { generation, result } => {
                    self.backup.on_prepared(generation, result)
                }
                JobResult::Exported(r) => {
                    self.backup.export = match r {
                        Ok(s) => Task::Done(s),
                        Err(e) => Task::Failed(e),
                    }
                }
                JobResult::ScanDecoded { path, result } => {
                    self.restore.on_scan_decoded(&path, result)
                }
                JobResult::Restored(r) => {
                    self.restore.restore = match r {
                        Ok(s) => Task::Done(s),
                        Err(e) => Task::Failed(e),
                    }
                }
            }
        }
    }

    fn top_bar(&mut self, ui: &mut Ui) {
        let p = Palette::of(ui);
        Panel::top("top_bar")
            .frame(
                Frame::new()
                    .fill(p.surface)
                    .inner_margin(Margin::symmetric(24, 12))
                    .stroke(Stroke::new(1.0, p.border)),
            )
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // Logo: rounded square with a "grid".
                    let (rect, _) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(rect, CornerRadius::same(8), p.accent);
                    for i in 0..3 {
                        for j in 0..3 {
                            if (i + j) % 2 == 0 {
                                let r = egui::Rect::from_min_size(
                                    rect.min + vec2(7.0 + i as f32 * 6.0, 7.0 + j as f32 * 6.0),
                                    vec2(4.0, 4.0),
                                );
                                ui.painter()
                                    .rect_filled(r, CornerRadius::same(1), p.on_accent);
                            }
                        }
                    }
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new("Pabaru").size(17.0).strong());
                        ui.label(
                            RichText::new(tr("Data backup to paper", "Záloha dat na papír"))
                                .small()
                                .color(p.muted),
                        );
                    });
                    ui.add_space(24.0);
                    segmented(
                        ui,
                        &mut self.tab,
                        &[
                            (Tab::Backup, tr("Back up", "Zálohovat")),
                            (Tab::Restore, tr("Restore", "Obnovit")),
                        ],
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if crate::widgets::theme_toggle(ui) {
                            // Switch to the theme opposite to the one shown now.
                            let next = if ui.visuals().dark_mode {
                                ThemePreference::Light
                            } else {
                                ThemePreference::Dark
                            };
                            self.theme = next;
                            ui.ctx().set_theme(next);
                        }
                    });
                });
            });
    }
}

impl eframe::App for PabaruApp {
    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.process_results();

        // Drag & drop: files/folders dropped onto the window.
        let (dropped, hovering) = ui.ctx().input(|i| {
            (
                i.raw
                    .dropped_files
                    .iter()
                    .map(|f| f.path().to_path_buf())
                    .collect::<Vec<_>>(),
                !i.raw.hovered_files.is_empty(),
            )
        });
        if !dropped.is_empty() {
            self.drop_paths(dropped);
        }

        self.top_bar(ui);

        let p = Palette::of(ui);
        let mut actions = Vec::new();
        CentralPanel::no_frame().show(ui, |ui| {
            Frame::new()
                .fill(p.bg)
                .inner_margin(Margin::symmetric(24, 20))
                .show(ui, |ui| {
                    ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            // Centre the content with a maximum width.
                            let w = ui.available_width();
                            let content = w.min(MAX_CONTENT_WIDTH);
                            ui.horizontal_top(|ui| {
                                ui.add_space(((w - content) / 2.0).max(0.0));
                                ui.vertical(|ui| {
                                    ui.set_width(content);
                                    match self.tab {
                                        Tab::Backup => views::backup::show(
                                            ui,
                                            &mut self.backup,
                                            hovering,
                                            &mut actions,
                                        ),
                                        Tab::Restore => views::restore::show(
                                            ui,
                                            &mut self.restore,
                                            hovering,
                                            &mut actions,
                                        ),
                                    }
                                    ui.add_space(24.0);
                                });
                            });
                        });
                });
        });

        for action in actions {
            self.handle(action);
        }
        // Keep spinners animating while jobs are running.
        if self.pending_jobs() > 0 {
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(100));
        }
    }
}
