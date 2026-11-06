//! Detecting the number of grid columns and rows and building an exact model.
//!
//! On a scan we know neither the resolution nor the dot size. We do know,
//! however, where the frame corners are. So we rectify the frame area into a
//! unit square and compute a "profile": for every vertical line, the fraction
//! of dark pixels. Solid grid lines are dark almost along their whole length
//! (≈100 %), whereas data – thanks to whitening – is only about half dark.
//! The profile peaks are therefore the grid lines, and their spacing gives
//! the number of columns. The same goes for rows.

use pabaru_layout::{CELL_DOTS, FRAME_EXTRA, QUIET};

use crate::frame::Frame;
use crate::gray::Gray;
use crate::homography::Homography;

/// Grid model: the number of cells and the transform from page dot
/// coordinates (the same as in `pabaru_layout::DotGrid`) to scan pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridModel {
    pub cols: u32,
    pub rows: u32,
    pub dots_to_px: Homography,
}

/// Finds the grid model inside the frame.
pub fn detect_grid(img: &Gray, frame: &Frame) -> Option<GridModel> {
    let c = frame.corners;
    let unit = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    let to_img = Homography::from_points(unit, [c.tl, c.tr, c.br, c.bl])?;

    let dist = |a: (f64, f64), b: (f64, f64)| ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt();
    let width_px = dist(c.tl, c.tr).max(dist(c.bl, c.br));
    let height_px = dist(c.tl, c.bl).max(dist(c.tr, c.br));

    // Darkness from 0.0 (white) to 1.0 (black).
    let dark = |u: f64, v: f64| {
        let (x, y) = to_img.apply(u, v);
        1.0 - img.sample(x, y) / 255.0
    };

    // Profile along x (vertical lines) and along y (horizontal lines).
    let cols = count_cells(&profile((width_px * 2.0) as usize, dark))?;
    let rows = count_cells(&profile((height_px * 2.0) as usize, |t, s| dark(s, t)))?;

    // Exact model: the outer frame corners in dot coordinates.
    let q = QUIET as f64;
    let wf = (cols * CELL_DOTS + 1 + 2 * FRAME_EXTRA) as f64;
    let hf = (rows * CELL_DOTS + 1 + 2 * FRAME_EXTRA) as f64;
    let dots_to_px = Homography::from_points(
        [(q, q), (q + wf, q), (q + wf, q + hf), (q, q + hf)],
        [c.tl, c.tr, c.br, c.bl],
    )?;
    Some(GridModel {
        cols,
        rows,
        dots_to_px,
    })
}

/// Computes the profile: for `n` positions `t ∈ (0, 1)`, the mean darkness
/// along the perpendicular direction `s` (we take the middle 70 % to avoid
/// the frame).
fn profile(n: usize, dark: impl Fn(f64, f64) -> f64) -> Vec<f64> {
    const SAMPLES: usize = 96;
    (0..n.max(16))
        .map(|i| {
            let t = (i as f64 + 0.5) / n.max(16) as f64;
            let sum: f64 = (0..SAMPLES)
                .map(|j| dark(t, 0.15 + 0.7 * (j as f64 + 0.5) / SAMPLES as f64))
                .sum();
            sum / SAMPLES as f64
        })
        .collect()
}

/// Threshold for lines in the profile: halfway between the median (typical
/// data darkness) and the 98.5th percentile (lines take up about 1/41 of the
/// profile, so this percentile lies on the lines). Independent of scan
/// contrast and brightness.
fn line_threshold(profile: &[f64]) -> f64 {
    let mut sorted = profile.to_vec();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let high = sorted[((sorted.len() as f64 * 0.985) as usize).min(sorted.len() - 1)];
    (median + high) / 2.0
}

/// Determines the number of cells from the profile: finds the centres of the
/// "lines" (contiguous runs above the threshold), takes the median of their
/// spacing and derives the cell count from it.
fn count_cells(profile: &[f64]) -> Option<u32> {
    let limit = line_threshold(profile);
    let mut centers = Vec::new();
    let mut start: Option<usize> = None;
    for (i, &p) in profile.iter().enumerate() {
        match (p > limit, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                centers.push((s + i - 1) as f64 / 2.0);
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        centers.push((s + profile.len() - 1) as f64 / 2.0);
    }
    if centers.len() < 2 {
        return None;
    }
    // Noise can split one line into several short runs right next to each
    // other. Grid lines, however, are ~40 dots apart, while fragments of one
    // line are only a fraction of a dot apart. So we merge centres closer
    // than 1/8 of the largest gap.
    let max_gap = centers
        .windows(2)
        .map(|w| w[1] - w[0])
        .fold(0.0f64, f64::max);
    let mut groups: Vec<(f64, usize)> = vec![(centers[0], 1)]; // (sum, count)
    let mut last = centers[0];
    for &c in &centers[1..] {
        if c - last < max_gap / 8.0 {
            let g = groups.last_mut().expect("at least one group");
            g.0 += c;
            g.1 += 1;
        } else {
            groups.push((c, 1));
        }
        last = c;
    }
    let centers: Vec<f64> = groups.iter().map(|(s, n)| s / *n as f64).collect();
    if centers.len() < 2 {
        return None;
    }
    let mut gaps: Vec<f64> = centers.windows(2).map(|w| w[1] - w[0]).collect();
    gaps.sort_by(f64::total_cmp);
    let median = gaps[gaps.len() / 2];
    if median <= 0.0 {
        return None;
    }
    let span = centers.last()? - centers.first()?;
    let cells = (span / median).round() as u32;
    (cells > 0).then_some(cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_count_from_profile() {
        // Synthetic profile: lines every 20 samples, noise around 0.5 between them.
        let p: Vec<f64> = (0..205)
            .map(|i| if i % 20 < 2 { 1.0 } else { 0.5 })
            .collect();
        assert_eq!(count_cells(&p), Some(10));
    }

    #[test]
    fn split_lines_are_merged() {
        // Every line broken by noise into two runs.
        let p: Vec<f64> = (0..205)
            .map(|i| if i % 20 == 0 || i % 20 == 2 { 1.0 } else { 0.5 })
            .collect();
        assert_eq!(count_cells(&p), Some(10));
    }

    #[test]
    fn spurious_line_is_harmless() {
        let mut p: Vec<f64> = (0..205)
            .map(|i| if i % 20 < 2 { 1.0 } else { 0.5 })
            .collect();
        p[47] = 0.9; // random peak in the data
        assert_eq!(count_cells(&p), Some(10));
    }
}
