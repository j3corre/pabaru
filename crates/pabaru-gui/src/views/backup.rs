//! "Back up" tab: dropping data, layout proposals, encryption, export.

use egui::{
    Align, ComboBox, CornerRadius, Frame, Grid, Layout, Margin, RichText, Sense, Stroke, TextEdit,
    Ui, vec2,
};
use pabaru_core::{LayoutProposal, Printer};

use super::{Action, preview, two_columns};
use crate::i18n::{self, decimal, printer_label, profile_description, profile_label, tr};
use crate::model::{BackupModel, PaperChoice, SCANNER_DPIS, Task, file_name, human_size};
use crate::theme::{CARD_RADIUS, Palette};
use crate::widgets::{
    FileIcon, card, card_title, chip, drop_zone, file_icon, hint, primary_button, secondary_button,
    stat,
};

/// The word "page" in the right grammatical form for `n` (Czech has three forms).
pub fn pages_word(n: u32) -> &'static str {
    match (i18n::lang(), n) {
        (i18n::Lang::En, 1) => "page",
        (i18n::Lang::En, _) => "pages",
        (i18n::Lang::Cs, 1) => "stránka",
        (i18n::Lang::Cs, 2..=4) => "stránky",
        (i18n::Lang::Cs, _) => "stránek",
    }
}

pub fn show(ui: &mut Ui, m: &mut BackupModel, hovering: bool, actions: &mut Vec<Action>) {
    if m.inputs.is_empty() {
        empty_state(ui, hovering, actions);
        return;
    }
    two_columns(
        ui,
        0.42,
        &mut (m, actions),
        |ui, (m, actions)| {
            inputs_card(ui, m, hovering, actions);
            devices_card(ui, m);
            password_card(ui, m);
        },
        |ui, (m, actions)| {
            proposals_section(ui, m);
            export_card(ui, m, actions);
        },
    );
}

/// Start screen without data.
fn empty_state(ui: &mut Ui, hovering: bool, actions: &mut Vec<Action>) {
    let p = Palette::of(ui);
    ui.add_space(8.0);
    ui.label(
        RichText::new(tr("Back up your data to paper", "Zálohujte data na papír"))
            .size(28.0)
            .strong(),
    );
    ui.label(
        RichText::new(tr(
            "Drop files or whole folders. We will propose a page layout for your printer and scanner.",
            "Přetáhněte soubory nebo celé složky. Navrhneme rozložení stránek podle vaší tiskárny a skeneru.",
        ))
        .color(p.muted)
        .size(15.0),
    );
    ui.add_space(18.0);
    if drop_zone(
        ui,
        "⬇",
        tr(
            "Drop files or folders here",
            "Přetáhněte sem soubory nebo složky",
        ),
        tr(
            "or click to choose files",
            "nebo klikněte a vyberte soubory",
        ),
        hovering,
        240.0,
    )
    .clicked()
    {
        actions.push(Action::PickInputs { folders: false });
    }
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if secondary_button(ui, tr("Choose files…", "Vybrat soubory…")).clicked() {
            actions.push(Action::PickInputs { folders: false });
        }
        if secondary_button(ui, tr("Choose folder…", "Vybrat složku…")).clicked() {
            actions.push(Action::PickInputs { folders: true });
        }
    });
    ui.add_space(28.0);
    feature_tiles(ui);
}

/// Three tiles with the main features.
fn feature_tiles(ui: &mut Ui) {
    let tiles = [
        (
            "🛡",
            tr("Resistant to damage", "Odolné proti poškození"),
            tr(
                "Two-level error correction reads even crumpled or stained paper.",
                "Dvouúrovňová oprava chyb přečte i pomačkaný nebo politý papír.",
            ),
        ),
        (
            "🔒",
            tr("Password encryption", "Šifrování heslem"),
            tr(
                "Argon2id + XChaCha20-Poly1305. The password is never printed.",
                "Argon2id + XChaCha20-Poly1305. Heslo se nikdy netiskne.",
            ),
        ),
        (
            "📴",
            tr("No cloud, no dependencies", "Bez cloudu a bez závislostí"),
            tr(
                "Paper lasts for decades and needs neither power nor an account.",
                "Papír vydrží desítky let a nepotřebuje elektřinu ani účet.",
            ),
        ),
    ];
    let gap = 14.0;
    let w = ((ui.available_width() - 2.0 * gap) / 3.0).max(180.0);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        for (icon, title, text) in tiles {
            ui.allocate_ui_with_layout(vec2(w, 0.0), Layout::top_down(Align::Min), |ui| {
                card(ui, |ui| {
                    let p = Palette::of(ui);
                    ui.label(RichText::new(icon).size(22.0).color(p.accent));
                    ui.label(RichText::new(title).strong());
                    ui.label(RichText::new(text).color(p.muted).small());
                });
            });
        }
    });
}

fn inputs_card(ui: &mut Ui, m: &mut BackupModel, hovering: bool, actions: &mut Vec<Action>) {
    card(ui, |ui| {
        let p = Palette::of(ui);
        card_title(
            ui,
            tr("Data to back up", "Data k zálohování"),
            Some(tr(
                "You can drop more files and folders onto the window at any time.",
                "Další soubory a složky můžete kdykoli přetáhnout do okna.",
            )),
        );
        let mut remove = None;
        for (i, path) in m.inputs.iter().enumerate() {
            ui.horizontal(|ui| {
                file_icon(
                    ui,
                    if path.is_dir() {
                        FileIcon::Folder
                    } else {
                        FileIcon::File
                    },
                );
                ui.vertical(|ui| {
                    ui.label(RichText::new(file_name(path)).strong());
                    ui.label(
                        RichText::new(path.display().to_string())
                            .color(p.muted)
                            .small(),
                    );
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let label = format!("{} {}", tr("Remove", "Odebrat"), file_name(path));
                    let resp = ui
                        .add(egui::Button::new(RichText::new("×").color(p.muted)).frame(false))
                        .on_hover_text(tr("Remove", "Odebrat"));
                    resp.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, &label)
                    });
                    if resp.clicked() {
                        remove = Some(i);
                    }
                });
            });
        }
        if let Some(i) = remove
            && m.remove_input(i)
        {
            actions.push(Action::Prepare);
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if secondary_button(ui, tr("+ Files…", "+ Soubory…")).clicked() {
                actions.push(Action::PickInputs { folders: false });
            }
            if secondary_button(ui, tr("+ Folder…", "+ Složka…")).clicked() {
                actions.push(Action::PickInputs { folders: true });
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
                    "Release the files – they will be added to the backup.",
                    "Pusťte soubory – přidají se k záloze.",
                ),
                p.accent,
            );
        }

        ui.separator();
        match &m.prepared {
            Task::Running => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(tr(
                            "Packing and compressing data…",
                            "Balím a komprimuji data…",
                        ))
                        .color(p.muted),
                    );
                });
            }
            Task::Done(prep) => {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 28.0;
                    stat(ui, tr("files", "souborů"), &prep.file_count().to_string());
                    stat(ui, tr("size", "velikost"), &human_size(prep.raw_bytes()));
                    stat(
                        ui,
                        tr("compressed", "po kompresi"),
                        &human_size(prep.packed_len()),
                    );
                });
            }
            Task::Failed(e) => hint(ui, "⚠", e, p.danger),
            Task::Idle => {}
        }
    });
    ui.add_space(14.0);
}

fn devices_card(ui: &mut Ui, m: &mut BackupModel) {
    card(ui, |ui| {
        card_title(
            ui,
            tr("Print and scan", "Tisk a sken"),
            Some(tr(
                "We pick a dot density your devices can reliably read.",
                "Podle zařízení zvolíme hustotu bodů, kterou spolehlivě přečtete.",
            )),
        );
        let mut changed = false;
        Grid::new("devices")
            .num_columns(2)
            .spacing(vec2(16.0, 10.0))
            .show(ui, |ui| {
                ui.label(tr("Printer", "Tiskárna"));
                ComboBox::from_id_salt("printer")
                    .selected_text(printer_label(m.printer))
                    .show_ui(ui, |ui| {
                        for pr in Printer::ALL {
                            changed |= ui
                                .selectable_value(&mut m.printer, pr, printer_label(pr))
                                .changed();
                        }
                    });
                ui.end_row();

                ui.label(tr("Scanner", "Skener"));
                ComboBox::from_id_salt("scanner")
                    .selected_text(format!("{} dpi", m.scanner_dpi))
                    .show_ui(ui, |ui| {
                        for dpi in SCANNER_DPIS {
                            changed |= ui
                                .selectable_value(&mut m.scanner_dpi, dpi, format!("{dpi} dpi"))
                                .changed();
                        }
                    });
                ui.end_row();

                ui.label(tr("Paper", "Papír"));
                ComboBox::from_id_salt("paper")
                    .selected_text(m.paper.label())
                    .show_ui(ui, |ui| {
                        for pc in [PaperChoice::A4, PaperChoice::Letter] {
                            changed |= ui.selectable_value(&mut m.paper, pc, pc.label()).changed();
                        }
                    });
                ui.end_row();
            });
        if changed {
            m.recompute();
        }
    });
    ui.add_space(14.0);
}

fn password_card(ui: &mut Ui, m: &mut BackupModel) {
    card(ui, |ui| {
        let p = Palette::of(ui);
        card_title(ui, tr("Password protection", "Ochrana heslem"), None);
        if ui
            .checkbox(
                &mut m.encrypt,
                tr(
                    "Encrypt the backup with a password",
                    "Zašifrovat zálohu heslem",
                ),
            )
            .changed()
        {
            m.recompute();
        }
        if m.encrypt {
            ui.add_space(4.0);
            ui.add(
                TextEdit::singleline(&mut m.password)
                    .password(true)
                    .hint_text(tr("Password", "Heslo"))
                    .desired_width(f32::INFINITY),
            );
            ui.add(
                TextEdit::singleline(&mut m.password_again)
                    .password(true)
                    .hint_text(tr("Password again", "Heslo znovu"))
                    .desired_width(f32::INFINITY),
            );
            if let Some(problem) = m.password_problem() {
                hint(ui, "⚠", problem, p.danger);
            } else if let Some(h) = m.password_hint() {
                hint(ui, "ℹ", h, p.warning);
            }
            hint(
                ui,
                "🔒",
                tr(
                    "The password is not printed on paper. Without it the backup cannot be restored.",
                    "Heslo se na papír netiskne. Bez něj zálohu nelze obnovit.",
                ),
                p.muted,
            );
        } else {
            ui.label(
                RichText::new(tr(
                    "Without encryption anyone holding the paper can read the backup.",
                    "Bez šifrování může zálohu přečíst kdokoli, kdo má papír.",
                ))
                .color(p.muted)
                .small(),
            );
        }
    });
}

fn proposals_section(ui: &mut Ui, m: &mut BackupModel) {
    let p = Palette::of(ui);
    ui.label(
        RichText::new(tr("Recommended layout", "Doporučené rozložení"))
            .size(18.0)
            .strong(),
    );
    ui.label(
        RichText::new(tr(
            "Choose a profile based on how long and where the backup will be stored.",
            "Vyberte profil podle toho, jak dlouho a kde bude záloha uložena.",
        ))
        .color(p.muted),
    );
    ui.add_space(8.0);

    if m.prepared.is_running() {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(tr("Computing proposals…", "Počítám návrhy…")).color(p.muted),
                );
            });
        });
        ui.add_space(14.0);
        return;
    }
    if m.proposals.is_empty() {
        card(ui, |ui| {
            hint(
                ui,
                "⚠",
                tr(
                    "No profile fits. Choose a scanner with a higher resolution or back up less data.",
                    "Žádný profil nevyhovuje. Zvolte skener s vyšším rozlišením nebo zálohujte méně dat.",
                ),
                p.warning,
            );
        });
        ui.add_space(14.0);
        return;
    }

    let gap = 12.0;
    let n = m.proposals.len() as f32;
    let narrow = ui.available_width() < 560.0;
    let w = if narrow {
        ui.available_width()
    } else {
        (ui.available_width() - gap * (n - 1.0)) / n
    };
    let mut clicked = None;
    let layout = if narrow {
        Layout::top_down(Align::Min)
    } else {
        Layout::left_to_right(Align::Min)
    };
    ui.with_layout(layout, |ui| {
        ui.spacing_mut().item_spacing = vec2(gap, gap);
        for prop in &m.proposals {
            if proposal_card(ui, prop, prop.profile == m.selected, w) {
                clicked = Some(prop.profile);
            }
        }
    });
    if let Some(profile) = clicked {
        m.selected = profile;
        m.export = Task::Idle;
    }
    ui.add_space(14.0);
}

/// Card of one proposal. Returns `true` when clicked.
fn proposal_card(ui: &mut Ui, prop: &LayoutProposal, selected: bool, width: f32) -> bool {
    let p = Palette::of(ui);
    let (fill, stroke) = if selected {
        (p.accent_soft, Stroke::new(2.0, p.accent))
    } else {
        (p.surface, Stroke::new(1.0, p.border))
    };
    // The whole card is one clickable element: `scope_builder` with `Sense::click()`
    // registers a container that receives clicks anywhere inside it.
    // Top-down layout – otherwise the card would inherit the parent's horizontal row.
    let builder = egui::UiBuilder::new()
        .sense(Sense::click())
        .layout(Layout::top_down(Align::Min));
    let scope = ui.scope_builder(builder, |ui| {
        Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(CornerRadius::same(CARD_RADIUS))
            .inner_margin(Margin::same(16))
            .show(ui, |ui| {
                ui.set_width(width - 36.0);
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(profile_label(prop.profile))
                            .size(15.0)
                            .strong(),
                    );
                    if selected {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new("✔").color(p.accent).strong());
                        });
                    }
                });
                ui.label(
                    RichText::new(profile_description(prop.profile))
                        .color(p.muted)
                        .small(),
                );
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(prop.pages.to_string()).size(30.0).strong());
                    ui.label(RichText::new(pages_word(prop.pages)).color(p.muted));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(6.0, 6.0);
                    chip(
                        ui,
                        &format!("{} {} mm", tr("dot", "bod"), decimal(prop.dot_mm as f64, 1)),
                        p.text,
                        p.surface_alt,
                    );
                    chip(
                        ui,
                        &format!("{} ≥ {} dpi", tr("scan", "sken"), prop.min_scan_dpi),
                        p.text,
                        p.surface_alt,
                    );
                    chip(
                        ui,
                        &format!(
                            "{} {:.0} %",
                            tr("tolerance", "odolnost"),
                            prop.loss_tolerance() * 100.0
                        ),
                        p.text,
                        p.surface_alt,
                    );
                    if prop.survives_page_loss() {
                        chip(
                            ui,
                            tr("survives losing a page", "přežije ztrátu stránky"),
                            p.success,
                            p.surface_alt,
                        );
                    }
                });
            });
    });
    let label = format!(
        "{} {}",
        tr("Profile", "Profil"),
        profile_label(prop.profile)
    );
    let resp = scope
        .response
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    resp.widget_info(|| {
        egui::WidgetInfo::selected(egui::Role::RadioButton, true, selected, &label)
    });
    resp.clicked()
}

fn export_card(ui: &mut Ui, m: &mut BackupModel, actions: &mut Vec<Action>) {
    let Some(prop) = m.selected_proposal().copied() else {
        return;
    };
    card(ui, |ui| {
        let p = Palette::of(ui);
        card_title(ui, tr("Preview and export", "Náhled a export"), None);
        ui.horizontal_top(|ui| {
            preview::page_preview(ui, &prop.params, 150.0);
            ui.add_space(18.0);
            ui.vertical(|ui| {
                let g = prop.params.geometry();
                Grid::new("details")
                    .num_columns(2)
                    .spacing(vec2(14.0, 6.0))
                    .show(ui, |ui| {
                        let mut row = |k: &str, v: String| {
                            ui.label(RichText::new(k).color(p.muted));
                            ui.label(RichText::new(v).strong());
                            ui.end_row();
                        };
                        row(
                            tr("Pages", "Stránek"),
                            format!(
                                "{} ({})",
                                prop.pages,
                                profile_label(prop.profile).to_lowercase()
                            ),
                        );
                        row(
                            tr("Grid", "Mřížka"),
                            format!("{} × {} {}", g.cols, g.rows, tr("blocks", "bloků")),
                        );
                        row(
                            tr("Dot density", "Hustota bodů"),
                            format!(
                                "{} dpi ({} {} mm, {} {} %)",
                                prop.dot_dpi,
                                tr("dot", "bod"),
                                decimal(prop.dot_mm as f64, 2),
                                tr("fill", "výplň"),
                                prop.params.dot_fill_pct
                            ),
                        );
                        row(
                            tr("Page capacity", "Kapacita stránky"),
                            human_size(prop.bytes_per_page),
                        );
                        row(
                            tr("Blocks", "Bloky"),
                            format!(
                                "{} {} + {} {}",
                                prop.data_shards,
                                tr("data", "datových"),
                                prop.parity_shards,
                                tr("parity", "paritních")
                            ),
                        );
                        row(
                            tr("Loss of a whole page", "Ztráta celé stránky"),
                            if prop.survives_page_loss() {
                                tr("survives", "přežije").into()
                            } else {
                                tr("does not survive", "nepřežije").into()
                            },
                        );
                        row(
                            tr("Scan at", "Skenujte"),
                            format!(
                                "≥ {} dpi, {}",
                                prop.min_scan_dpi,
                                tr("greyscale", "odstíny šedi")
                            ),
                        );
                    });
            });
        });
        ui.add_space(10.0);
        let blocker = m.export_blocker();
        ui.horizontal(|ui| {
            if primary_button(ui, tr("Save PDF…", "Uložit PDF…"), blocker.is_none()).clicked()
            {
                actions.push(Action::Export { png: false });
            }
            if ui
                .add_enabled(
                    blocker.is_none(),
                    egui::Button::new(tr("Save as PNG…", "Uložit jako PNG…"))
                        .min_size(vec2(0.0, 38.0)),
                )
                .clicked()
            {
                actions.push(Action::Export { png: true });
            }
        });
        match &m.export {
            Task::Running => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(
                        RichText::new(tr("Creating pages…", "Vytvářím stránky…")).color(p.muted),
                    );
                });
            }
            Task::Done(s) => hint(
                ui,
                "✔",
                &format!(
                    "{} {} {}: {}",
                    tr("Saved", "Uloženo"),
                    s.pages,
                    pages_word(s.pages as u32),
                    s.path.display()
                ),
                p.success,
            ),
            Task::Failed(e) => hint(ui, "⚠", e, p.danger),
            Task::Idle => {
                if let Some(b) = blocker {
                    hint(ui, "ℹ", b, p.muted);
                }
            }
        }
        hint(
            ui,
            "ℹ",
            tr(
                "Print at 100 % scale (no \"fit to page\"), ideally on a laser printer.",
                "Tiskněte v měřítku 100 % (bez „přizpůsobit stránce“), ideálně laserovou tiskárnou.",
            ),
            p.muted,
        );
    });
}
