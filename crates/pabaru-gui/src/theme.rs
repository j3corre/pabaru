//! Visual style of the app – a clean, modern look with light and dark variants.
//!
//! Colours live in one place ([`Palette`]); the rest of the GUI reads them via
//! [`Palette::of`], so the theme follows the system automatically.

use egui::{
    Color32, CornerRadius, FontFamily, FontId, Shadow, Stroke, TextStyle, Theme, ThemePreference,
    Visuals, vec2,
};

/// Colour palette of one theme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    /// Window background.
    pub bg: Color32,
    /// Card surface.
    pub surface: Color32,
    /// Slightly different surface (inputs, chip backgrounds).
    pub surface_alt: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    /// Main accent colour (indigo).
    pub accent: Color32,
    /// Soft accent variant (background of selected items).
    pub accent_soft: Color32,
    /// Text on the accent colour.
    pub on_accent: Color32,
    pub success: Color32,
    pub warning: Color32,
    pub danger: Color32,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        bg: Color32::from_rgb(0xF5, 0xF6, 0xF8),
        surface: Color32::from_rgb(0xFF, 0xFF, 0xFF),
        surface_alt: Color32::from_rgb(0xF1, 0xF2, 0xF5),
        border: Color32::from_rgb(0xE3, 0xE5, 0xEA),
        text: Color32::from_rgb(0x17, 0x19, 0x1F),
        muted: Color32::from_rgb(0x6B, 0x71, 0x7D),
        accent: Color32::from_rgb(0x4F, 0x46, 0xE5),
        accent_soft: Color32::from_rgb(0xEE, 0xEF, 0xFE),
        on_accent: Color32::WHITE,
        success: Color32::from_rgb(0x16, 0xA3, 0x4A),
        warning: Color32::from_rgb(0xD9, 0x77, 0x06),
        danger: Color32::from_rgb(0xDC, 0x26, 0x26),
    };

    pub const DARK: Palette = Palette {
        bg: Color32::from_rgb(0x10, 0x12, 0x17),
        surface: Color32::from_rgb(0x19, 0x1C, 0x23),
        surface_alt: Color32::from_rgb(0x22, 0x25, 0x2E),
        border: Color32::from_rgb(0x2C, 0x30, 0x3A),
        text: Color32::from_rgb(0xE9, 0xEB, 0xEF),
        muted: Color32::from_rgb(0x9A, 0xA0, 0xAC),
        accent: Color32::from_rgb(0x81, 0x8C, 0xF8),
        accent_soft: Color32::from_rgb(0x25, 0x28, 0x4D),
        on_accent: Color32::from_rgb(0x10, 0x12, 0x17),
        success: Color32::from_rgb(0x22, 0xC5, 0x5E),
        warning: Color32::from_rgb(0xF5, 0x9E, 0x0B),
        danger: Color32::from_rgb(0xF8, 0x71, 0x71),
    };

    /// Palette for the current theme of the given `Ui`.
    pub fn of(ui: &egui::Ui) -> Palette {
        if ui.visuals().dark_mode {
            Self::DARK
        } else {
            Self::LIGHT
        }
    }
}

/// Corner radius of cards.
pub const CARD_RADIUS: u8 = 14;
/// Corner radius of controls.
pub const WIDGET_RADIUS: u8 = 8;

/// Builds `Visuals` from a palette.
fn visuals(p: &Palette, dark: bool) -> Visuals {
    let mut v = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.surface;
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 90 } else { 28 }),
    };
    v.popup_shadow = v.window_shadow;
    v.menu_corner_radius = CornerRadius::same(WIDGET_RADIUS);
    v.extreme_bg_color = p.surface_alt;
    v.faint_bg_color = p.surface_alt;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.accent;
    v.selection.bg_fill = p.accent_soft;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.warn_fg_color = p.warning;
    v.error_fg_color = p.danger;

    let r = CornerRadius::same(WIDGET_RADIUS);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.surface;
    w.noninteractive.weak_bg_fill = p.surface;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.noninteractive.corner_radius = r;
    for (state, fill, stroke) in [
        (&mut w.inactive, p.surface_alt, p.border),
        (&mut w.hovered, p.accent_soft, p.accent),
        (&mut w.active, p.accent_soft, p.accent),
        (&mut w.open, p.accent_soft, p.accent),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, stroke);
        state.fg_stroke = Stroke::new(1.5, p.text);
        state.corner_radius = r;
        state.expansion = 0.0;
    }
    v
}

/// Installs the style into the context (both themes + typography and spacing).
pub fn install(ctx: &egui::Context) {
    ctx.set_visuals_of(Theme::Light, visuals(&Palette::LIGHT, false));
    ctx.set_visuals_of(Theme::Dark, visuals(&Palette::DARK, true));
    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (
                TextStyle::Heading,
                FontId::new(22.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Small,
                FontId::new(12.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
        style.spacing.item_spacing = vec2(8.0, 8.0);
        style.spacing.button_padding = vec2(14.0, 7.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.combo_width = 180.0;
        // An app, not a document: labels are not selectable (clicks belong to cards).
        style.interaction.selectable_labels = false;
    });
    ctx.set_theme(ThemePreference::System);
}
