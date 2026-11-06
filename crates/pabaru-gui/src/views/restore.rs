//! "Restore" tab: dropping scans, progress, quality map, password, restore.

use egui::{ComboBox, RichText, TextEdit, Ui, vec2};

use super::{Action, preview, two_columns};
use crate::i18n::{self, tr};
use crate::model::{DestChoice, RestoreModel, Task};
use crate::theme::Palette;
use crate::widgets::{
    FileIcon, card, card_title, chip, drop_zone, file_icon, hint, primary_button, progress_bar,
    secondary_button,
};

pub fn show(ui: &mut Ui, m: &mut RestoreModel, hovering: bool, actions: &mut Vec<Action>) {
    if m.scans.is_empty() {
        empty_state(ui, hovering, actions);
        return;
    }
    two_columns(
        ui,
        0.42,
        &mut (m, actions),
        |ui, (m, actions)| {
            scans_card(ui, m, hovering, actions);
            status_card(ui, m);
        },
        |ui, (m, actions)| {
            quality_card(ui, m);
            restore_card(ui, m, actions);
        },
    );
}

fn empty_state(ui: &mut Ui, hovering: bool, actions: &mut Vec<Action>) {
    let p = Palette::of(ui);
    ui.add_space(8.0);
    ui.label(
        RichText::new(tr("Restore data from scans", "Obnovte data ze skenů"))
            .size(28.0)
            .strong(),
    );
    ui.label(
        RichText::new(tr(
            "Scan the pages in greyscale (at least 600 dpi) and drop them here. Order and rotation do not matter.",
            "Naskenujte stránky v odstínech šedi (aspoň 600 dpi) a přetáhněte je sem. Na pořadí ani natočení nezáleží.",
        ))
        .color(p.muted)
        .size(15.0),
    );
    ui.add_space(18.0);
    if drop_zone(
        ui,
        "⬇",
        tr(
            "Drop scanned pages here",
            "Přetáhněte sem naskenované stránky",
        ),
        tr(
            "PNG, JPEG, TIFF, BMP or PDF from the scanner",
            "PNG, JPEG, TIFF, BMP nebo PDF ze skeneru",
        ),
        hovering,
        240.0,
    )
    .clicked()
    {
        actions.push(Action::PickScans);
    }
    ui.add_space(12.0);
    if secondary_button(ui, tr("Choose scans…", "Vybrat skeny…")).clicked() {
        actions.push(Action::PickScans);
    }
}

fn scans_card(ui: &mut Ui, m: &mut RestoreModel, hovering: bool, actions: &mut Vec<Action>) {
    card(ui, |ui| {
        let p = Palette::of(ui);
        card_title(
            ui,
            tr("Scans", "Skeny"),
            Some(tr(
                "You can drop more pages onto the window at any time.",
                "Další stránky můžete kdykoli přetáhnout do okna.",
            )),
        );
        for scan in &m.scans {
            ui.horizontal(|ui| {
                file_icon(ui, FileIcon::Image);
                ui.label(RichText::new(scan.file_name()).strong());
                ui.with_layout(
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| match &scan.state {
                        Task::Running | Task::Idle => {
                            ui.spinner();
                        }
                        Task::Done(pages) => {
                            for s in pages.iter().rev() {
                                let total = s.ok + s.failed;
                                let text = match s.page_index {
                                    Some(i) => format!(
                                        "{} {} · {}/{}",
                                        tr("p.", "str."),
                                        i + 1,
                                        s.ok,
                                        total
                                    ),
                                    None => {
                                        format!("{}/{} {}", s.ok, total, tr("blocks", "bloků"))
                                    }
                                };
                                let fg = if s.failed == 0 { p.success } else { p.warning };
                                chip(ui, &text, fg, p.surface_alt);
                            }
                        }
                        Task::Failed(e) => {
                            chip(ui, tr("unreadable", "nečitelné"), p.danger, p.surface_alt);
                            ui.label(RichText::new("ℹ").color(p.danger))
                                .on_hover_text(e);
                        }
                    },
                );
            });
        }
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if secondary_button(ui, tr("+ Add scans…", "+ Přidat skeny…")).clicked() {
                actions.push(Action::PickScans);
            }
            if ui
                .add(
                    egui::Button::new(RichText::new(tr("Clear", "Vyčistit")).color(p.muted))
                        .frame(false),
                )
                .clicked()
            {
                m.clear();
            }
        });
        if hovering {
            hint(
                ui,
                "⬇",
                tr(
                    "Release the scans – they will be added to the restore.",
                    "Pusťte skeny – přidají se k obnově.",
                ),
                p.accent,
            );
        }
    });
    ui.add_space(14.0);
}

fn status_card(ui: &mut Ui, m: &RestoreModel) {
    card(ui, |ui| {
        let p = Palette::of(ui);
        card_title(ui, tr("Restore status", "Stav obnovy"), None);
        let Some(pr) = m.progress() else {
            ui.label(
                RichText::new(if m.busy() {
                    tr(
                        "Looking for a backup on the scans…",
                        "Hledám zálohu na skenech…",
                    )
                } else {
                    tr(
                        "No Pabaru backup has been recognised on the scans yet.",
                        "Na skenech zatím nebyla rozpoznána žádná záloha Pabaru.",
                    )
                })
                .color(p.muted),
            );
            return;
        };
        ui.horizontal(|ui| {
            // The name is empty unless the user gave an explicit label.
            let name = if pr.name.is_empty() {
                tr("Pabaru backup", "Záloha Pabaru")
            } else {
                pr.name.as_str()
            };
            ui.label(RichText::new(name).size(16.0).strong());
            if pr.encrypted {
                chip(
                    ui,
                    tr("🔒 encrypted", "🔒 šifrováno"),
                    p.accent,
                    p.accent_soft,
                );
            }
        });
        ui.add_space(6.0);
        let fraction = pr.have as f32 / pr.need.max(1) as f32;
        let color = if pr.can_restore() {
            p.success
        } else {
            p.accent
        };
        progress_bar(ui, fraction, color);
        ui.add_space(4.0);
        let (have, total, need) = (pr.have.min(pr.total), pr.total, pr.need);
        ui.label(match i18n::lang() {
            i18n::Lang::En => {
                format!("Read {have} of {total} blocks; {need} are enough to restore.")
            }
            i18n::Lang::Cs => format!("Přečteno {have} bloků z {total}; k obnově stačí {need}."),
        });
        ui.label(
            RichText::new(format!(
                "{} {} {} {}",
                tr("Scanned pages:", "Naskenované stránky:"),
                pr.pages_seen.len(),
                tr("of", "z"),
                pr.page_count
            ))
            .color(p.muted),
        );
        if pr.can_restore() {
            let missing = pr.missing_pages();
            if missing.is_empty() {
                hint(
                    ui,
                    "✔",
                    tr("Enough data to restore.", "Dost dat pro obnovu."),
                    p.success,
                );
            } else {
                hint(
                    ui,
                    "✔",
                    tr(
                        "Enough data to restore – error correction will rebuild the missing pages.",
                        "Dost dat pro obnovu – chybějící stránky dopočítá oprava chyb.",
                    ),
                    p.success,
                );
            }
        }
    });
}

fn quality_card(ui: &mut Ui, m: &mut RestoreModel) {
    if m.pages.is_empty() {
        return;
    }
    card(ui, |ui| {
        let p = Palette::of(ui);
        card_title(
            ui,
            tr("Quality map", "Mapa kvality"),
            Some(tr(
                "Each square is one data block on the scanned page.",
                "Každý čtvereček je jeden blok dat na naskenované stránce.",
            )),
        );
        if m.selected_page >= m.pages.len() {
            m.selected_page = 0;
        }
        if m.pages.len() > 1 {
            ComboBox::from_id_salt("page")
                .selected_text(m.pages[m.selected_page].0.clone())
                .show_ui(ui, |ui| {
                    for (i, (label, _)) in m.pages.iter().enumerate() {
                        ui.selectable_value(&mut m.selected_page, i, label);
                    }
                });
        }
        let width = ui.available_width().min(520.0);
        preview::quality_map(ui, &m.pages[m.selected_page].1, width);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
            chip(
                ui,
                tr("■ no errors", "■ bez chyb"),
                p.success,
                p.surface_alt,
            );
            chip(
                ui,
                tr("■ corrected", "■ opraveno"),
                p.warning,
                p.surface_alt,
            );
            chip(
                ui,
                tr("■ unreadable", "■ nečitelné"),
                p.danger,
                p.surface_alt,
            );
        });
    });
    ui.add_space(14.0);
}

fn restore_card(ui: &mut Ui, m: &mut RestoreModel, actions: &mut Vec<Action>) {
    card(ui, |ui| {
        let p = Palette::of(ui);
        card_title(ui, tr("File restore", "Obnova souborů"), None);

        if m.needs_password() {
            ui.label(tr("Backup password", "Heslo zálohy"));
            ui.add(
                TextEdit::singleline(&mut m.password)
                    .password(true)
                    .hint_text(tr("Enter the password", "Zadejte heslo"))
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(6.0);
        }

        ui.label(tr("Restore to", "Kam obnovit"));
        ui.radio_value(
            &mut m.dest,
            DestChoice::NextToScans,
            tr("Next to the scans", "Vedle skenů"),
        );
        ui.radio_value(
            &mut m.dest,
            DestChoice::Folder,
            tr("Into a chosen folder", "Do vybrané složky"),
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let text = m
                .target_dir()
                .map(|f| f.display().to_string())
                .unwrap_or_else(|| tr("No folder selected", "Složka není vybrána").into());
            ui.label(RichText::new(text).color(p.muted));
            if m.dest == DestChoice::Folder
                && secondary_button(ui, tr("Choose…", "Vybrat…")).clicked()
            {
                actions.push(Action::PickRestoreFolder);
            }
        });

        ui.add_space(10.0);
        let blocker = m.restore_blocker();
        if primary_button(
            ui,
            tr("Restore files", "Obnovit soubory"),
            blocker.is_none(),
        )
        .clicked()
        {
            actions.push(Action::Restore);
        }
        match &m.restore {
            Task::Running => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(tr(
                            "Restoring and verifying files…",
                            "Obnovuji a ověřuji soubory…",
                        ))
                        .color(p.muted),
                    );
                });
            }
            Task::Done(s) => {
                hint(
                    ui,
                    "✔",
                    &match i18n::lang() {
                        i18n::Lang::En => format!(
                            "Restored {} files to: {}. All checksums match.",
                            s.files, s.target
                        ),
                        i18n::Lang::Cs => format!(
                            "Obnoveno {} souborů do: {}. Všechny kontrolní součty sedí.",
                            s.files, s.target
                        ),
                    },
                    p.success,
                );
                if s.recovered_blocks > 0 {
                    hint(
                        ui,
                        "ℹ",
                        &format!(
                            "{} {}",
                            s.recovered_blocks,
                            tr(
                                "missing blocks were rebuilt from parity.",
                                "chybějících bloků bylo dopočítáno z parity."
                            )
                        ),
                        p.muted,
                    );
                }
            }
            Task::Failed(e) => hint(ui, "⚠", e, p.danger),
            Task::Idle => {
                if let Some(b) = blocker {
                    hint(ui, "ℹ", &b, p.muted);
                }
            }
        }
    });
}
