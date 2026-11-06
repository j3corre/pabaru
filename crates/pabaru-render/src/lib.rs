//! # pabaru-render
//!
//! Renders pages into a 1-bit bitmap at printer resolution and exports them
//! to PNG or PDF.
//!
//! * [`Bitmap`] – black-and-white image, 1 bit per pixel (1 = white, 0 = black,
//!   the same as in PDF and PNG, so the data can be written without conversion).
//! * [`rasterize`] – turns a dot map ([`DotGrid`]) into printer pixels.
//! * [`png::write_png`] – PNG with the DPI stored.
//! * [`pdf::write_pdf`] – multi-page PDF, image at an exact 1:1 scale.

mod bitmap;
pub mod pdf;
pub mod png;

pub use bitmap::Bitmap;

use pabaru_layout::{DotGrid, PageGeometry};
use thiserror::Error;

/// Export errors.
#[derive(Debug, Error)]
pub enum RenderError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("PNG error: {0}")]
    Png(#[from] ::png::EncodingError),
}

/// Renders a page's data area into a full-page bitmap.
///
/// * Data dots are drawn smaller according to `dot_fill_pct` so that the
///   toner/ink does not merge with neighbouring dots.
/// * Grid lines have the same thickness and position as data dots, they are
///   just continuous along the line. Thanks to that, the centre of a line lies
///   exactly on the centre of a row of dots, and the decoder can locate the
///   dots from the lines (important when a dot cannot be centred in pixels,
///   e.g. 2 px within a 3 px pitch).
/// * Line crossings and the thick frame are drawn as full squares.
pub fn rasterize(geom: &PageGeometry, grid: &DotGrid, dot_fill_pct: u8) -> Bitmap {
    let mut bmp = Bitmap::new_white(geom.page_w_px, geom.page_h_px);
    let ppd = geom.px_per_dot;
    // Data dot size in pixels (at least 1 pixel) and the padding to centre it.
    let size = ((ppd as f32 * dot_fill_pct as f32 / 100.0).round() as u32).clamp(1, ppd);
    let pad = (ppd - size) / 2;

    for y in 0..grid.height {
        for x in 0..grid.width {
            if !grid.get(x, y) {
                continue;
            }
            let px = geom.region_x_px + x * ppd;
            let py = geom.region_y_px + y * ppd;
            if grid.line(x, y) {
                let (xi, yi) = (x as i64, y as i64);
                // Line direction from the neighbouring line dots.
                let vertical = grid.line_at(xi, yi - 1) || grid.line_at(xi, yi + 1);
                let horizontal = grid.line_at(xi - 1, yi) || grid.line_at(xi + 1, yi);
                match (vertical, horizontal) {
                    (true, false) => bmp.fill_rect(px + pad, py, size, ppd),
                    (false, true) => bmp.fill_rect(px, py + pad, ppd, size),
                    _ => bmp.fill_rect(px, py, ppd, ppd),
                }
            } else {
                bmp.fill_rect(px + pad, py + pad, size, size);
            }
        }
    }
    bmp
}

#[cfg(test)]
mod tests {
    use super::*;
    use pabaru_format::{BLOCK_LEN, dots};
    use pabaru_layout::grid::cell_origin;
    use pabaru_layout::{Paper, PrintParams};

    /// Ideal "scan": reads the blocks directly from the bitmap's dot centres.
    /// Checks that rasterization and geometry fit together.
    #[test]
    fn ideal_read_from_bitmap() {
        let params = PrintParams {
            paper: Paper::Custom {
                width_mm: 60.0,
                height_mm: 60.0,
            },
            margin_mm: 3.0,
            header_mm: 3.0,
            ..Default::default()
        };
        let geom = params.geometry();
        assert!(geom.cells() >= 4);
        let blocks: Vec<[u8; BLOCK_LEN]> = (0..geom.cells())
            .map(|i| {
                let mut b = [0u8; BLOCK_LEN];
                for (j, v) in b.iter_mut().enumerate() {
                    *v = (i as usize * 17 + j * 5) as u8;
                }
                b
            })
            .collect();
        let grid = DotGrid::build(geom.cols, geom.rows, &blocks);
        let bmp = rasterize(&geom, &grid, params.dot_fill_pct);

        let ppd = geom.px_per_dot;
        for r in 0..geom.rows {
            for c in 0..geom.cols {
                let (ox, oy) = cell_origin(c, r);
                let cw = dots::codeword_from_dots(|x, y| {
                    let px = geom.region_x_px + (ox + x as u32) * ppd + ppd / 2;
                    let py = geom.region_y_px + (oy + y as u32) * ppd + ppd / 2;
                    bmp.is_black(px, py)
                });
                assert_eq!(cw, blocks[(r * geom.cols + c) as usize]);
            }
        }
    }
}
