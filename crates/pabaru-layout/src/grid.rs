//! Dot map of a page's data area.

use pabaru_format::{BLOCK_DOTS, BLOCK_LEN, dots};

use crate::geometry::region_dots;
use crate::{CELL_DOTS, FRAME_EXTRA, QUIET};

/// Two-dimensional array of dots; `true` = black dot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DotGrid {
    pub width: u32,
    pub height: u32,
    /// Dots in row order.
    pub dots: Vec<bool>,
    /// The dot belongs to a grid line or the frame (printed as a full square
    /// so that the lines are continuous).
    pub is_line: Vec<bool>,
}

/// Offset of the first grid line from the edge of the data area.
pub const GRID_OFFSET: u32 = QUIET + FRAME_EXTRA;

/// Top-left dot of a cell's data area (column `c`, row `r`).
pub fn cell_origin(c: u32, r: u32) -> (u32, u32) {
    (
        GRID_OFFSET + c * CELL_DOTS + 1,
        GRID_OFFSET + r * CELL_DOTS + 1,
    )
}

impl DotGrid {
    /// Builds the dot map for a `cols × rows` grid with the given blocks
    /// (in row order, length `cols * rows`).
    pub fn build(cols: u32, rows: u32, blocks: &[[u8; BLOCK_LEN]]) -> Self {
        assert_eq!(
            blocks.len(),
            (cols * rows) as usize,
            "number of blocks does not match the grid"
        );
        let width = region_dots(cols);
        let height = region_dots(rows);
        let mut g = Self {
            width,
            height,
            dots: vec![false; (width * height) as usize],
            is_line: vec![false; (width * height) as usize],
        };

        // 1) Thick outer frame.
        let (x0, y0) = (QUIET, QUIET);
        let (x1, y1) = (width - QUIET, height - QUIET); // exclusive
        let t = FRAME_EXTRA + 1;
        for y in y0..y1 {
            for x in x0..x1 {
                let inside = x >= x0 + t && x < x1 - t && y >= y0 + t && y < y1 - t;
                if !inside {
                    g.set_line(x, y);
                }
            }
        }

        // 2) Inner grid lines.
        let grid_end_x = GRID_OFFSET + cols * CELL_DOTS;
        let grid_end_y = GRID_OFFSET + rows * CELL_DOTS;
        for c in 0..=cols {
            let x = GRID_OFFSET + c * CELL_DOTS;
            for y in GRID_OFFSET..=grid_end_y {
                g.set_line(x, y);
            }
        }
        for r in 0..=rows {
            let y = GRID_OFFSET + r * CELL_DOTS;
            for x in GRID_OFFSET..=grid_end_x {
                g.set_line(x, y);
            }
        }

        // 3) Block data.
        for r in 0..rows {
            for c in 0..cols {
                let cw = &blocks[(r * cols + c) as usize];
                let (ox, oy) = cell_origin(c, r);
                for dy in 0..BLOCK_DOTS {
                    for dx in 0..BLOCK_DOTS {
                        if dots::dot_at(cw, dx, dy) {
                            let i = g.idx(ox + dx as u32, oy + dy as u32);
                            g.dots[i] = true;
                        }
                    }
                }
            }
        }
        g
    }

    #[inline]
    fn idx(&self, x: u32, y: u32) -> usize {
        (y * self.width + x) as usize
    }

    fn set_line(&mut self, x: u32, y: u32) {
        let i = self.idx(x, y);
        self.dots[i] = true;
        self.is_line[i] = true;
    }

    /// Is the dot black?
    pub fn get(&self, x: u32, y: u32) -> bool {
        self.dots[self.idx(x, y)]
    }

    /// Does the dot belong to a grid line?
    pub fn line(&self, x: u32, y: u32) -> bool {
        self.is_line[self.idx(x, y)]
    }

    /// Like [`DotGrid::line`], but allows coordinates outside the grid (returns `false`).
    pub fn line_at(&self, x: i64, y: i64) -> bool {
        x >= 0
            && y >= 0
            && x < self.width as i64
            && y < self.height as i64
            && self.line(x as u32, y as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_read_back() {
        let blocks: Vec<[u8; BLOCK_LEN]> = (0..6)
            .map(|i| {
                let mut b = [0u8; BLOCK_LEN];
                for (j, v) in b.iter_mut().enumerate() {
                    *v = (i * 13 + j * 7) as u8;
                }
                b
            })
            .collect();
        let g = DotGrid::build(3, 2, &blocks);
        for r in 0..2 {
            for c in 0..3 {
                let (ox, oy) = cell_origin(c, r);
                let cw = dots::codeword_from_dots(|x, y| g.get(ox + x as u32, oy + y as u32));
                assert_eq!(cw, blocks[(r * 3 + c) as usize]);
            }
        }
    }

    #[test]
    fn frame_and_margin() {
        let g = DotGrid::build(1, 1, &[[0u8; BLOCK_LEN]]);
        assert!(!g.get(0, 0), "white margin");
        assert!(g.get(QUIET, QUIET), "frame corner");
        assert!(g.get(GRID_OFFSET, GRID_OFFSET + 10), "grid line");
        assert!(!g.get(GRID_OFFSET + 5, GRID_OFFSET + 5), "empty data");
    }
}
