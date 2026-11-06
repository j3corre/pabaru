//! Page geometry: paper size, print resolution and number of blocks.

use serde::{Deserialize, Serialize};

use crate::{CELL_DOTS, FRAME_EXTRA, QUIET};

/// Paper size.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Paper {
    A4,
    Letter,
    /// Custom size in millimetres (width, height).
    Custom {
        width_mm: f32,
        height_mm: f32,
    },
}

impl Paper {
    /// Size in millimetres (width, height).
    pub fn size_mm(self) -> (f32, f32) {
        match self {
            Paper::A4 => (210.0, 297.0),
            Paper::Letter => (215.9, 279.4),
            Paper::Custom {
                width_mm,
                height_mm,
            } => (width_mm, height_mm),
        }
    }
}

/// Print parameters chosen by the user or by the optimizer.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PrintParams {
    /// Physical printer resolution (e.g. 600).
    pub printer_dpi: u32,
    /// How many printer pixels make up one dot (pitch). 600 dpi / 2 = 300 dpi dots.
    pub px_per_dot: u32,
    /// Size of a black dot as a percentage of the pitch (PaperBack recommends 70 %).
    /// A smaller dot "bleeds" less into its neighbours.
    pub dot_fill_pct: u8,
    pub paper: Paper,
    /// Paper margin (printers usually cannot print up to the edge).
    pub margin_mm: f32,
    /// Height of the strip for the human-readable header above the grid.
    pub header_mm: f32,
}

impl Default for PrintParams {
    fn default() -> Self {
        Self {
            printer_dpi: 600,
            px_per_dot: 3,
            dot_fill_pct: 70,
            paper: Paper::A4,
            margin_mm: 10.0,
            header_mm: 12.0,
        }
    }
}

impl PrintParams {
    /// Dot density (dots per inch).
    pub fn dot_dpi(&self) -> f32 {
        self.printer_dpi as f32 / self.px_per_dot as f32
    }

    /// Computes the page geometry for these parameters.
    pub fn geometry(&self) -> PageGeometry {
        PageGeometry::compute(self)
    }
}

/// Resulting geometry of a single page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageGeometry {
    /// Number of block columns.
    pub cols: u32,
    /// Number of block rows.
    pub rows: u32,
    /// Size of the whole page in printer pixels.
    pub page_w_px: u32,
    pub page_h_px: u32,
    /// Top-left corner of the data area (including the white margin) in pixels.
    pub region_x_px: u32,
    pub region_y_px: u32,
    /// Size of the data area in dots (including the white margin).
    pub region_w_dots: u32,
    pub region_h_dots: u32,
    pub px_per_dot: u32,
}

/// How many dots a grid with `n` cells takes in one direction, including frame and margin.
pub fn region_dots(n: u32) -> u32 {
    n * CELL_DOTS + 1 + 2 * (FRAME_EXTRA + QUIET)
}

/// How many cells fit into `avail` dots.
fn cells_fitting(avail: u32) -> u32 {
    let fixed = 1 + 2 * (FRAME_EXTRA + QUIET);
    avail.saturating_sub(fixed) / CELL_DOTS
}

impl PageGeometry {
    fn compute(p: &PrintParams) -> Self {
        let (w_mm, h_mm) = p.paper.size_mm();
        let mm_to_px = |mm: f32| (mm / 25.4 * p.printer_dpi as f32).floor() as u32;
        let page_w_px = mm_to_px(w_mm);
        let page_h_px = mm_to_px(h_mm);
        let margin_px = mm_to_px(p.margin_mm);
        let header_px = mm_to_px(p.header_mm);

        // Available area in dots.
        let avail_w = page_w_px.saturating_sub(2 * margin_px) / p.px_per_dot;
        let avail_h = page_h_px.saturating_sub(2 * margin_px + header_px) / p.px_per_dot;
        let cols = cells_fitting(avail_w);
        let rows = cells_fitting(avail_h);
        let region_w_dots = region_dots(cols);
        let region_h_dots = region_dots(rows);

        // Center the data area horizontally; vertically it starts below the header.
        let region_w_px = region_w_dots * p.px_per_dot;
        let region_x_px = (page_w_px.saturating_sub(region_w_px)) / 2;
        let region_y_px = margin_px + header_px;

        Self {
            cols,
            rows,
            page_w_px,
            page_h_px,
            region_x_px,
            region_y_px,
            region_w_dots,
            region_h_dots,
            px_per_dot: p.px_per_dot,
        }
    }

    /// Number of blocks (cells) on a page.
    pub fn cells(&self) -> u32 {
        self.cols * self.rows
    }

    /// Raw page capacity in bytes of payload (without parity and superblocks).
    pub fn raw_capacity_bytes(&self) -> u64 {
        self.cells() as u64 * pabaru_format::PAYLOAD_LEN as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_600dpi_200dpi_dots() {
        let g = PrintParams::default().geometry();
        // A4 without margins is 190 mm ≈ 1496 dots at 200 dpi -> 36 columns.
        assert_eq!(g.cols, 36);
        assert!(g.rows > 40);
        // The data area must fit on the paper.
        assert!(g.region_x_px + g.region_w_dots * g.px_per_dot <= g.page_w_px);
        assert!(g.region_y_px + g.region_h_dots * g.px_per_dot <= g.page_h_px);
    }

    #[test]
    fn higher_density_more_blocks() {
        let mut p = PrintParams::default();
        let a = p.geometry().cells();
        p.px_per_dot = 2;
        let b = p.geometry().cells();
        assert!(b > a * 2);
    }

    #[test]
    fn small_paper_does_not_panic() {
        let p = PrintParams {
            paper: Paper::Custom {
                width_mm: 20.0,
                height_mm: 20.0,
            },
            ..Default::default()
        };
        assert_eq!(p.geometry().cells(), 0);
    }
}
