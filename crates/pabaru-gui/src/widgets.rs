//! Reusable UI elements (cards, drop zone, buttons, chips…).

use egui::{
    Align2, Button, Color32, CornerRadius, FontId, Frame, Margin, Response, RichText, Sense, Shape,
    Stroke, StrokeKind, Ui, pos2, vec2,
};

use crate::i18n::tr;
use crate::theme::{CARD_RADIUS, Palette, WIDGET_RADIUS};

/// Card: white surface with a subtle border and rounded corners.
///
/// Content is always laid out top to bottom – otherwise `Frame` would inherit
/// the parent's layout (e.g. horizontal) and texts would not wrap.
pub fn card<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let p = Palette::of(ui);
    Frame::new()
        .fill(p.surface)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(CARD_RADIUS))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(add_contents).inner
        })
        .inner
}

/// Card title with an optional subtitle.
pub fn card_title(ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    let p = Palette::of(ui);
    ui.label(RichText::new(title).size(16.0).strong());
    if let Some(s) = subtitle {
        ui.label(RichText::new(s).color(p.muted).small());
    }
    ui.add_space(6.0);
}

/// Primary (accent) button. When disabled it has a muted fill with readable text.
pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    let p = Palette::of(ui);
    let (fill, fg) = if enabled {
        (p.accent, p.on_accent)
    } else {
        (p.surface_alt, p.muted)
    };
    let button = Button::new(RichText::new(text).color(fg).strong())
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(WIDGET_RADIUS))
        .min_size(vec2(0.0, 38.0));
    // Custom disabled colours – without egui's default "greying out".
    ui.scope(|ui| {
        ui.style_mut().visuals.disabled_alpha = 1.0;
        ui.add_enabled(enabled, button)
    })
    .inner
}

/// Kind of file icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileIcon {
    Folder,
    File,
    Image,
}

/// Small painted icon (sharp at any resolution, no emoji font needed).
pub fn file_icon(ui: &mut Ui, kind: FileIcon) {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(9), p.accent_soft);
    let o = rect.min;
    let r = |x0: f32, y0: f32, x1: f32, y1: f32| {
        egui::Rect::from_min_max(o + vec2(x0, y0), o + vec2(x1, y1))
    };
    let stroke = Stroke::new(1.6, p.accent);
    match kind {
        FileIcon::Folder => {
            painter.rect_filled(r(8.0, 10.0, 16.0, 14.0), CornerRadius::same(2), p.accent);
            painter.rect_filled(r(8.0, 12.5, 26.0, 25.0), CornerRadius::same(3), p.accent);
        }
        FileIcon::File => {
            painter.rect_stroke(
                r(10.5, 8.0, 23.5, 26.0),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Middle,
            );
            for y in [14.0, 18.0, 22.0] {
                painter.line_segment(
                    [o + vec2(13.5, y), o + vec2(20.5, y)],
                    Stroke::new(1.2, p.accent),
                );
            }
        }
        FileIcon::Image => {
            painter.rect_stroke(
                r(8.0, 10.0, 26.0, 24.0),
                CornerRadius::same(2),
                stroke,
                StrokeKind::Middle,
            );
            painter.circle_filled(o + vec2(13.0, 14.5), 1.8, p.accent);
            painter.line_segment([o + vec2(9.0, 23.0), o + vec2(15.5, 17.0)], stroke);
            painter.line_segment([o + vec2(15.5, 17.0), o + vec2(19.0, 20.5)], stroke);
            painter.line_segment([o + vec2(19.0, 20.5), o + vec2(25.0, 15.5)], stroke);
        }
    }
}

/// Light/dark theme toggle: a painted sun or moon. Returns `true` when clicked.
pub fn theme_toggle(ui: &mut Ui) -> bool {
    let p = Palette::of(ui);
    let dark = ui.visuals().dark_mode;
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    let resp = resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if dark {
            tr("Switch to light theme", "Přepnout na světlý motiv")
        } else {
            tr("Switch to dark theme", "Přepnout na tmavý motiv")
        });
    resp.widget_info(|| {
        egui::WidgetInfo::labeled(
            egui::Role::Button,
            true,
            tr("Toggle theme", "Přepnout motiv"),
        )
    });
    let painter = ui.painter();
    if resp.hovered() {
        painter.rect_filled(rect, CornerRadius::same(9), p.surface_alt);
    }
    let c = rect.center();
    if dark {
        // Moon: a circle covered by a circle in the background colour.
        painter.circle_filled(c, 8.0, p.text);
        let bg = if resp.hovered() {
            p.surface_alt
        } else {
            p.surface
        };
        painter.circle_filled(c + vec2(4.0, -3.0), 7.0, bg);
    } else {
        // Sun: a circle and rays.
        painter.circle_filled(c, 4.5, p.text);
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let d = vec2(a.cos(), a.sin());
            painter.line_segment([c + d * 7.5, c + d * 10.5], Stroke::new(1.6, p.text));
        }
    }
    resp.clicked()
}

/// Secondary (outlined) button.
pub fn secondary_button(ui: &mut Ui, text: &str) -> Response {
    let p = Palette::of(ui);
    ui.add(
        Button::new(RichText::new(text).color(p.text))
            .fill(p.surface)
            .stroke(Stroke::new(1.0, p.border))
            .corner_radius(CornerRadius::same(WIDGET_RADIUS))
            .min_size(vec2(0.0, 34.0)),
    )
}

/// Small coloured tag ("chip").
///
/// Painted as a single element of known size – so `horizontal_wrapped` can
/// move it to the next line as a whole (a container such as `Frame` could
/// not wrap and the text would break letter by letter).
pub fn chip(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), FontId::proportional(12.0), fg);
    let size = galley.size() + vec2(18.0, 7.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same(255), bg);
    painter.galley(rect.center() - galley.size() / 2.0, galley, fg);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
}

/// Statistic: a large value with a label below.
pub fn stat(ui: &mut Ui, label: &str, value: &str) {
    let p = Palette::of(ui);
    ui.vertical(|ui| {
        ui.label(RichText::new(value).size(20.0).strong());
        ui.label(RichText::new(label).color(p.muted).small());
    });
}

/// Drop zone for dragged files. A click can be used to open a dialog.
///
/// `hovering` = the user is dragging files over the window (highlighted).
pub fn drop_zone(
    ui: &mut Ui,
    icon: &str,
    title: &str,
    subtitle: &str,
    hovering: bool,
    height: f32,
) -> Response {
    let p = Palette::of(ui);
    let (rect, response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
    let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    let active = hovering || response.hovered();
    let painter = ui.painter_at(rect.expand(2.0));

    painter.rect_filled(
        rect,
        CornerRadius::same(CARD_RADIUS),
        if active { p.accent_soft } else { p.surface },
    );
    // Dashed border (rectangle with bevelled corners as a polyline).
    let stroke = Stroke::new(1.5, if active { p.accent } else { p.border });
    let r = CARD_RADIUS as f32;
    let (l, t, rr, b) = (rect.left(), rect.top(), rect.right(), rect.bottom());
    let path = [
        pos2(l + r, t),
        pos2(rr - r, t),
        pos2(rr, t + r),
        pos2(rr, b - r),
        pos2(rr - r, b),
        pos2(l + r, b),
        pos2(l, b - r),
        pos2(l, t + r),
        pos2(l + r, t),
    ];
    painter.extend(Shape::dashed_line(&path, stroke, 7.0, 5.0));

    // Icon in a circle and texts.
    let c = rect.center();
    painter.circle_filled(c - vec2(0.0, 30.0), 24.0, p.accent_soft);
    painter.text(
        c - vec2(0.0, 30.0),
        Align2::CENTER_CENTER,
        icon,
        FontId::proportional(22.0),
        p.accent,
    );
    painter.text(
        c + vec2(0.0, 12.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(17.0),
        p.text,
    );
    painter.text(
        c + vec2(0.0, 36.0),
        Align2::CENTER_CENTER,
        subtitle,
        FontId::proportional(13.0),
        p.muted,
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, title));
    response
}

/// A row with an icon and text in a given colour (hint, status).
pub fn hint(ui: &mut Ui, icon: &str, text: &str, color: Color32) {
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(icon).color(color));
        ui.label(RichText::new(text).color(color));
    });
}

/// Segmented control (e.g. tabs). Returns `true` if the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let p = Palette::of(ui);
    let mut changed = false;
    Frame::new()
        .fill(p.surface_alt)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (opt, label) in options {
                    let selected = *value == *opt;
                    let text = RichText::new(*label).color(if selected { p.text } else { p.muted });
                    let text = if selected { text.strong() } else { text };
                    let resp = ui.add(
                        Button::new(text)
                            .fill(if selected {
                                p.surface
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(if selected {
                                Stroke::new(1.0, p.border)
                            } else {
                                Stroke::NONE
                            })
                            .corner_radius(CornerRadius::same(8))
                            .min_size(vec2(110.0, 30.0)),
                    );
                    if resp.clicked() && !selected {
                        *value = *opt;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// Thin rounded progress bar.
pub fn progress_bar(ui: &mut Ui, fraction: f32, color: Color32) {
    let p = Palette::of(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 8.0), Sense::hover());
    ui.painter()
        .rect_filled(rect, CornerRadius::same(4), p.surface_alt);
    let mut fill = rect;
    fill.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    ui.painter().rect_filled(fill, CornerRadius::same(4), color);
}

/// Rectangle outline (helper for previews).
pub fn outline(ui: &Ui, rect: egui::Rect, radius: u8, color: Color32) {
    ui.painter().rect_stroke(
        rect,
        CornerRadius::same(radius),
        Stroke::new(1.0, color),
        StrokeKind::Inside,
    );
}
