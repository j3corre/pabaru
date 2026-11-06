//! Reading individual grid cells (blocks).
//!
//! A cell is given by its edges (`lattice::Patch` – polylines along the grid
//! lines). For every dot of the block:
//! 1. **Position** – a Coons patch built from the edges, so local rotation,
//!    scaling, shear and even curvature (crumpled paper) are handled.
//! 2. **Sampling** – brightness at the dot centre and at 4 points around it
//!    (averaged).
//! 3. **Threshold** – computed separately for each cell with the 2-means
//!    method (splitting the values into two groups), so it does not matter
//!    if the scan is lighter/darker elsewhere.

use pabaru_format::BLOCK_DOTS;
use pabaru_layout::CELL_DOTS;

use crate::gray::Gray;
use crate::lattice::{Patch, Pt};

/// Measured dot values of one cell, in scan orientation.
#[derive(Debug, Clone)]
pub struct CellSamples {
    /// Column and row of the cell on the scan.
    pub col: u32,
    pub row: u32,
    /// `true` = black dot, row by row (BLOCK_DOTS × BLOCK_DOTS).
    pub bits: Vec<bool>,
    /// Confidence of each dot: distance of the measured value from the
    /// threshold (small = the dot was unclear, grey).
    pub margin: Vec<f32>,
}

const N: usize = BLOCK_DOTS;

/// Reads the cell given by the edges `patch`. `shift` offsets the reading by
/// a fraction of a dot (for a retry on an unreadable cell).
pub fn sample_patch(
    img: &Gray,
    patch: &Patch,
    col: u32,
    row: u32,
    shift: (f64, f64),
) -> CellSamples {
    let cell = CELL_DOTS as f64;
    let corners = patch.corners();
    // Local vectors of one dot (for sampling around the centre).
    let u = (
        (corners[1].0 - corners[0].0) / cell,
        (corners[1].1 - corners[0].1) / cell,
    );
    let v = (
        (corners[2].0 - corners[0].0) / cell,
        (corners[2].1 - corners[0].1) / cell,
    );
    let darkness = |p: Pt| 255.0 - img.sample(p.0, p.1);

    let mut values = Vec::with_capacity(N * N);
    for y in 0..N {
        for x in 0..N {
            // The node lies on the line centre; data starts 1 dot further in.
            let s = (x as f64 + 1.0 + shift.0) / cell;
            let t = (y as f64 + 1.0 + shift.1) / cell;
            let c = patch.point(s, t);
            let d = 0.15;
            let off = |a: f64, b: f64| (c.0 + u.0 * a + v.0 * b, c.1 + u.1 * a + v.1 * b);
            let sum = darkness(c) * 2.0
                + darkness(off(-d, 0.0))
                + darkness(off(d, 0.0))
                + darkness(off(0.0, -d))
                + darkness(off(0.0, d));
            values.push(sum / 6.0);
        }
    }

    let t = two_means_threshold(&values);
    CellSamples {
        col,
        row,
        bits: values.iter().map(|&v| v > t).collect(),
        margin: values.iter().map(|&v| (v - t).abs() as f32).collect(),
    }
}

/// Recommended anchor spacings (in dots) for retries.
pub const ANCHOR_STEPS: [usize; 2] = [4, 3];

/// Reads a cell with **local re-detection of the dot raster**.
///
/// A large cell (~2 cm with coarse dots) may be wavier inside than
/// interpolation from its edges can capture. The data dots themselves,
/// however, lie on a regular raster: at the correct phase a sample hits the
/// dot centre (dark if the dot is black), and the gaps diagonally between
/// dots (±0.5 dot) are always white.
///
/// So at anchor dots (every 4 dots) we search for the ±0.5-dot shift that
/// maximises the "raster energy" in a 5 × 5 dot window. Anchors are
/// processed from the cell edge inwards, each starting from its neighbours'
/// shift, so even a smooth larger drift is tracked. Each dot's shift is
/// then a bilinear interpolation between the anchors.
pub fn sample_patch_adaptive(
    img: &Gray,
    patch: &Patch,
    col: u32,
    row: u32,
    anchor_step: usize,
) -> CellSamples {
    let cell = CELL_DOTS as f64;
    let corners = patch.corners();
    let u = (
        (corners[1].0 - corners[0].0) / cell,
        (corners[1].1 - corners[0].1) / cell,
    );
    let v = (
        (corners[2].0 - corners[0].0) / cell,
        (corners[2].1 - corners[0].1) / cell,
    );
    let darkness = |p: Pt| 255.0 - img.sample(p.0, p.1);
    // Initial dot centre positions (Coons patch from the cell edges).
    let base: Vec<Pt> = (0..N * N)
        .map(|k| {
            let (x, y) = (k % N, k / N);
            patch.point((x as f64 + 1.0) / cell, (y as f64 + 1.0) / cell)
        })
        .collect();
    let at = |x: usize, y: usize, a: f64, b: f64| {
        let p = base[y * N + x];
        (p.0 + u.0 * a + v.0 * b, p.1 + u.1 * a + v.1 * b)
    };
    // Raster energy in the window around anchor (ax, ay) at shift (a, b):
    // the variance of the samples. At the correct phase every sample is
    // clearly black or white (high variance); at a wrong phase samples fall
    // on dot edges and give grey midtones (low variance). Independent of
    // shadow brightness.
    let energy = |ax: usize, ay: usize, a: f64, b: f64| -> f64 {
        let (mut sum, mut sum2, mut n) = (0.0, 0.0, 0.0);
        for y in ay.saturating_sub(3)..(ay + 4).min(N) {
            for x in ax.saturating_sub(3)..(ax + 4).min(N) {
                let d = darkness(at(x, y, a, b));
                sum += d;
                sum2 += d * d;
                n += 1.0;
            }
        }
        let mean = sum / n;
        (sum2 / n - mean * mean) * n / 100.0
    };
    // 1D search for the best shift around the prediction `start` (±0.3 dot
    // in steps of 0.05). The raster is periodic – a shift by a whole dot
    // would give the same energy, hence the small range and a mild penalty
    // for distance from the prediction.
    let search = |start: f64, f: &dyn Fn(f64) -> f64| -> f64 {
        (-6..=6)
            .map(|k| start + k as f64 * 0.05)
            .max_by(|p, q| {
                let pen = |x: f64| 2000.0 * (x - start) * (x - start);
                (f(*p) - pen(*p)).total_cmp(&(f(*q) - pen(*q)))
            })
            .unwrap_or(start)
    };

    // Anchors at dots 2, 6, 10, …, 38 in both directions.
    let anchors: Vec<usize> = (2..N).step_by(anchor_step).collect();
    let na = anchors.len();
    let mut field: Vec<Option<(f64, f64)>> = vec![None; na * na];
    // Order: in "rings" from the cell edge inwards (the edge is reliable).
    let ring = |i: usize, j: usize| i.min(j).min(na - 1 - i).min(na - 1 - j);
    let mut order: Vec<(usize, usize)> =
        (0..na).flat_map(|j| (0..na).map(move |i| (i, j))).collect();
    order.sort_by_key(|&(i, j)| ring(i, j));
    for (i, j) in order {
        // Prediction = mean of the already computed neighbours (otherwise no shift).
        let mut sum = (0.0, 0.0);
        let mut n = 0.0;
        for (di, dj) in [
            (-1i64, 0i64),
            (1, 0),
            (0, -1),
            (0, 1),
            (-1, -1),
            (1, 1),
            (-1, 1),
            (1, -1),
        ] {
            let (ni, nj) = (i as i64 + di, j as i64 + dj);
            if ni < 0 || nj < 0 || ni >= na as i64 || nj >= na as i64 {
                continue;
            }
            if let Some(o) = field[nj as usize * na + ni as usize] {
                sum = (sum.0 + o.0, sum.1 + o.1);
                n += 1.0;
            }
        }
        let pred = if n > 0.0 {
            (sum.0 / n, sum.1 / n)
        } else {
            (0.0, 0.0)
        };
        let (ax, ay) = (anchors[i], anchors[j]);
        let a = search(pred.0, &|a| energy(ax, ay, a, pred.1));
        let b = search(pred.1, &|b| energy(ax, ay, a, b));
        let a = search(a, &|a| energy(ax, ay, a, b));
        field[j * na + i] = Some((a, b));
    }

    // Second, symmetric pass: the first pass worked as a chain (errors could
    // accumulate in one direction). Now every anchor is searched again
    // around the mean of ALL its neighbours from the first pass.
    let first = field.clone();
    for j in 0..na {
        for i in 0..na {
            let mut sum = (0.0, 0.0);
            let mut n = 0.0;
            for dj in -1i64..=1 {
                for di in -1i64..=1 {
                    let (ni, nj) = (i as i64 + di, j as i64 + dj);
                    if ni < 0 || nj < 0 || ni >= na as i64 || nj >= na as i64 {
                        continue;
                    }
                    if let Some(o) = first[nj as usize * na + ni as usize] {
                        sum = (sum.0 + o.0, sum.1 + o.1);
                        n += 1.0;
                    }
                }
            }
            let pred = (sum.0 / n, sum.1 / n);
            let (ax, ay) = (anchors[i], anchors[j]);
            let a = search(pred.0, &|a| energy(ax, ay, a, pred.1));
            let b = search(pred.1, &|b| energy(ax, ay, a, b));
            field[j * na + i] = Some((a, b));
        }
    }

    // Dot shift = bilinear interpolation between the anchors.
    let offset = |x: usize, y: usize| -> (f64, f64) {
        let fx = ((x as f64 - 2.0) / anchor_step as f64).clamp(0.0, (na - 1) as f64);
        let fy = ((y as f64 - 2.0) / anchor_step as f64).clamp(0.0, (na - 1) as f64);
        let (i0, j0) = (fx.floor() as usize, fy.floor() as usize);
        let (i1, j1) = ((i0 + 1).min(na - 1), (j0 + 1).min(na - 1));
        let (tx, ty) = (fx - i0 as f64, fy - j0 as f64);
        let g = |i: usize, j: usize| field[j * na + i].unwrap_or((0.0, 0.0));
        let lerp =
            |p: (f64, f64), q: (f64, f64), t: f64| (p.0 + (q.0 - p.0) * t, p.1 + (q.1 - p.1) * t);
        lerp(
            lerp(g(i0, j0), g(i1, j0), tx),
            lerp(g(i0, j1), g(i1, j1), tx),
            ty,
        )
    };

    let mut values = Vec::with_capacity(N * N);
    for y in 0..N {
        for x in 0..N {
            let (a, b) = offset(x, y);
            let c = at(x, y, a, b);
            let d = 0.15;
            let off = |p: f64, q: f64| (c.0 + u.0 * p + v.0 * q, c.1 + u.1 * p + v.1 * q);
            let sum = darkness(c) * 2.0
                + darkness(off(-d, 0.0))
                + darkness(off(d, 0.0))
                + darkness(off(0.0, -d))
                + darkness(off(0.0, d));
            values.push(sum / 6.0);
        }
    }
    let t = two_means_threshold(&values);
    CellSamples {
        col,
        row,
        bits: values.iter().map(|&v| v > t).collect(),
        margin: values.iter().map(|&v| (v - t).abs() as f32).collect(),
    }
}

/// Finds the threshold between two clusters of values (iteratively:
/// threshold = mean of the cluster centres).
fn two_means_threshold(values: &[f64]) -> f64 {
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let mut t = (min + max) / 2.0;
    for _ in 0..10 {
        let (mut lo, mut nlo, mut hi, mut nhi) = (0.0, 0usize, 0.0, 0usize);
        for &v in values {
            if v > t {
                hi += v;
                nhi += 1;
            } else {
                lo += v;
                nlo += 1;
            }
        }
        if nlo == 0 || nhi == 0 {
            break;
        }
        let nt = (lo / nlo as f64 + hi / nhi as f64) / 2.0;
        if (nt - t).abs() < 0.01 {
            break;
        }
        t = nt;
    }
    t
}

/// One of the 8 symmetries of a square (4 rotations × mirroring). Returns
/// the position on the scan for dot `(x, y)` on paper.
#[inline]
pub fn orient(t: u8, x: usize, y: usize) -> (usize, usize) {
    let m = N - 1;
    let (x, y) = if t >= 4 { (m - x, y) } else { (x, y) };
    match t % 4 {
        0 => (x, y),
        1 => (m - y, x),
        2 => (m - x, m - y),
        _ => (y, m - x),
    }
}

impl CellSamples {
    /// Assembles the codeword for the given orientation.
    pub fn codeword(&self, t: u8) -> [u8; pabaru_format::BLOCK_LEN] {
        pabaru_format::dots::codeword_from_dots(|x, y| {
            let (ix, iy) = orient(t, x, y);
            self.bits[iy * N + ix]
        })
    }

    /// The `count` least reliable bytes of the codeword (in orientation `t`).
    /// A byte's reliability = the smallest confidence among its 8 dots. These
    /// bytes are passed to Reed-Solomon as erasures.
    pub fn unreliable_bytes(&self, t: u8, count: usize) -> Vec<usize> {
        let mut bytes: Vec<(f32, usize)> = (0..pabaru_format::BLOCK_LEN)
            .map(|i| {
                let worst = (0..8)
                    .map(|b| {
                        let d = i * 8 + b;
                        let (ix, iy) = orient(t, d % N, d / N);
                        self.margin[iy * N + ix]
                    })
                    .fold(f32::INFINITY, f32::min);
                (worst, i)
            })
            .collect();
        bytes.sort_by(|a, b| a.0.total_cmp(&b.0));
        bytes.into_iter().take(count).map(|(_, i)| i).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orientations_are_distinct_permutations() {
        let mut seen = std::collections::HashSet::new();
        for t in 0..8 {
            let img: Vec<(usize, usize)> = [(0, 0), (1, 0), (0, 2)]
                .iter()
                .map(|&(x, y)| orient(t, x, y))
                .collect();
            assert!(seen.insert(img), "orientation {t} repeats");
        }
    }

    #[test]
    fn two_clusters() {
        let v = [10.0, 12.0, 11.0, 200.0, 190.0, 210.0];
        let t = two_means_threshold(&v);
        assert!(t > 12.0 && t < 190.0);
    }
}
