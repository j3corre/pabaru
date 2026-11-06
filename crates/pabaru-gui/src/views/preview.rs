//! Painted previews: page schematic and scan quality map.

use egui::{Color32, CornerRadius, Rect, Sense, Ui, pos2, vec2};
use pabaru_core::{CELL_DOTS, FRAME_EXTRA, PageScan, PrintParams, QUIET};

use crate::theme::Palette;

/// Schematic of the printed page: paper, header and block grid to scale.
pub fn page_preview(ui: &mut Ui, params: &PrintParams, width: f32) {
    let p = Palette::of(ui);
    let g = params.geometry();
    let (w_mm, h_mm) = params.paper.size_mm();
    let height = width * h_mm / w_mm;
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let painter = ui.painter();

    // Paper with a shadow.
    painter.rect_filled(
        rect.translate(vec2(0.0, 3.0)),
        CornerRadius::same(4),
        Color32::from_black_alpha(25),
    );
    painter.rect_filled(rect, CornerRadius::same(4), Color32::WHITE);
    crate::widgets::outline(ui, rect, 4, p.border);

    let scale = width / g.page_w_px as f32;
    let px = |x: u32, y: u32| {
        pos2(
            rect.left() + x as f32 * scale,
            rect.top() + y as f32 * scale,
        )
    };

    // Header – three "lines of text".
    let margin = params.margin_mm / 25.4 * params.printer_dpi as f32;
    for (i, frac) in [0.55f32, 0.75, 0.35].iter().enumerate() {
        let y = margin + i as f32 * 0.016 * g.page_h_px as f32;
        let start = px(margin as u32, y as u32);
        let r = Rect::from_min_size(start, vec2(width * 0.8 * frac, 1.6));
        painter.rect_filled(r, CornerRadius::ZERO, Color32::from_gray(185));
    }

    // Data area with the grid.
    let ppd = g.px_per_dot;
    let x0 = g.region_x_px + QUIET * ppd;
    let y0 = g.region_y_px + QUIET * ppd;
    // Outer frame size in dots (see `pabaru-layout`).
    let frame_dots = |n: u32| (n * CELL_DOTS + 1 + 2 * FRAME_EXTRA) * ppd;
    let grid = Rect::from_min_max(
        px(x0, y0),
        px(x0 + frame_dots(g.cols), y0 + frame_dots(g.rows)),
    );
    painter.rect_filled(
        grid,
        CornerRadius::ZERO,
        Color32::from_rgb(0xE6, 0xE7, 0xEE),
    );
    let line = Color32::from_rgb(0x9A, 0x9C, 0xB0);
    let step_x = grid.width() / g.cols.max(1) as f32;
    let step_y = grid.height() / g.rows.max(1) as f32;
    // With a dense grid draw only every n-th line so the preview is not a grey blob.
    let every = ((3.0 / step_x.min(step_y)).ceil() as u32).max(1);
    for c in (0..=g.cols).step_by(every as usize) {
        let x = grid.left() + c as f32 * step_x;
        painter.line_segment([pos2(x, grid.top()), pos2(x, grid.bottom())], (0.5, line));
    }
    for r in (0..=g.rows).step_by(every as usize) {
        let y = grid.top() + r as f32 * step_y;
        painter.line_segment([pos2(grid.left(), y), pos2(grid.right(), y)], (0.5, line));
    }
    crate::widgets::outline(ui, grid, 0, Color32::from_gray(40));
}

/// Cell colour by the number of corrected bytes (`None` = unreadable).
pub fn quality_color(p: &Palette, corrected: Option<usize>) -> Color32 {
    match corrected {
        None => p.danger,
        Some(0) => p.success,
        Some(n) => {
            // Smooth green → orange transition by the share of corrections (max. 30 bytes).
            let t = (n as f32 / 30.0).clamp(0.0, 1.0);
            lerp(p.success, p.warning, t)
        }
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

/// Quality map: each block as a coloured square.
pub fn quality_map(ui: &mut Ui, scan: &PageScan, max_width: f32) {
    let p = Palette::of(ui);
    let cols = scan.cols.max(1) as f32;
    let rows = scan.rows.max(1) as f32;
    let cell = (max_width / cols).clamp(2.0, 14.0);
    let (rect, _) = ui.allocate_exact_size(vec2(cell * cols, cell * rows), Sense::hover());
    let painter = ui.painter();
    let gap = if cell >= 5.0 { 1.0 } else { 0.0 };
    let radius = CornerRadius::same(if cell >= 8.0 { 2 } else { 0 });
    for c in &scan.cells {
        let min = pos2(
            rect.left() + c.col as f32 * cell,
            rect.top() + c.row as f32 * cell,
        );
        let r = Rect::from_min_size(min, vec2(cell - gap, cell - gap));
        let color = quality_color(&p, c.block.as_ref().map(|b| b.corrected_bytes));
        painter.rect_filled(r, radius, color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quality_colors() {
        let p = Palette::LIGHT;
        assert_eq!(quality_color(&p, None), p.danger);
        assert_eq!(quality_color(&p, Some(0)), p.success);
        assert_eq!(quality_color(&p, Some(30)), p.warning);
    }
}
