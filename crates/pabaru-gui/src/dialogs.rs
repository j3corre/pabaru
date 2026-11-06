//! System dialogs for picking files.
//!
//! They sit behind the [`Dialogs`] trait so the GUI can be tested without
//! opening real windows (tests supply their own implementation with fixed paths).

use std::path::PathBuf;

use crate::i18n::tr;

/// Dialogs the application needs.
pub trait Dialogs {
    /// Pick files to back up.
    fn pick_files(&self) -> Vec<PathBuf>;
    /// Pick folders to back up.
    fn pick_folders(&self) -> Vec<PathBuf>;
    /// Pick files containing scans.
    fn pick_scans(&self) -> Vec<PathBuf>;
    /// Pick a single folder (restore target, PNG export).
    fn pick_folder(&self, title: &str) -> Option<PathBuf>;
    /// Save a PDF.
    fn save_pdf(&self, suggested_name: &str) -> Option<PathBuf>;
}

/// Native dialogs (Windows, macOS, Linux) via the `rfd` crate.
pub struct NativeDialogs;

impl Dialogs for NativeDialogs {
    fn pick_files(&self) -> Vec<PathBuf> {
        rfd::FileDialog::new()
            .set_title(tr(
                "Select files to back up",
                "Vyberte soubory k zálohování",
            ))
            .pick_files()
            .unwrap_or_default()
    }

    fn pick_folders(&self) -> Vec<PathBuf> {
        rfd::FileDialog::new()
            .set_title(tr(
                "Select folders to back up",
                "Vyberte složky k zálohování",
            ))
            .pick_folders()
            .unwrap_or_default()
    }

    fn pick_scans(&self) -> Vec<PathBuf> {
        rfd::FileDialog::new()
            .set_title(tr("Select scanned pages", "Vyberte naskenované stránky"))
            .add_filter(
                tr("Scans", "Skeny"),
                &["png", "jpg", "jpeg", "tif", "tiff", "bmp", "pdf"],
            )
            .pick_files()
            .unwrap_or_default()
    }

    fn pick_folder(&self, title: &str) -> Option<PathBuf> {
        rfd::FileDialog::new().set_title(title).pick_folder()
    }

    fn save_pdf(&self, suggested_name: &str) -> Option<PathBuf> {
        rfd::FileDialog::new()
            .set_title(tr("Save backup as PDF", "Uložit zálohu jako PDF"))
            .add_filter("PDF", &["pdf"])
            .set_file_name(suggested_name)
            .save_file()
    }
}
