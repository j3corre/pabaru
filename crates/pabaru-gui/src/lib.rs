//! # pabaru-gui
//!
//! Pabaru desktop interface (egui/eframe).
//!
//! Layers:
//! * [`model`] – state and logic without an egui dependency (unit tested),
//! * [`worker`] – long-running jobs in background threads,
//! * `views` + `widgets` + `theme` – rendering,
//! * [`app`] – glue: drag & drop, actions, job results,
//! * [`dialogs`] – system dialogs behind a trait (replaced in tests),
//! * [`i18n`] – English/Czech texts, language picked from the OS locale.

pub mod app;
pub mod dialogs;
pub mod i18n;
pub mod model;
mod theme;
mod views;
mod widgets;
pub mod worker;

pub use app::PabaruApp;
pub use dialogs::{Dialogs, NativeDialogs};
