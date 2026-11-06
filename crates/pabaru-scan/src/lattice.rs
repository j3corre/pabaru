//! Local grid tracking – robustness against crumpled paper.
//!
//! Crumpled paper cannot be described by a single transform (homography)
//! for the whole page: grid lines are wavy and at folds they even "jump".
//! So we locate every **node** (intersection of grid lines) separately:
//!
//! 1. The initial node position estimate comes from the global model
//!    (frame) or from the frameless grid estimate (`seed.rs`).
//! 2. A node is refined by 1D search: a shift across the vertical line and
//!    a shift across the horizontal line. The score is a "ridge" – line
//!    darkness minus darkness right next to it – so shadows and darker
//!    areas do not matter.
//! 3. Refinement proceeds from the most confident node to its neighbours
//!    (best-first). A neighbour's prediction = refined node + local grid
//!    vector, so the deformation is tracked step by step and the error does
//!    not accumulate across the page.
//! 4. A cell is then read bilinearly between its four corners – this
//!    handles local rotation, scaling and shear.

use std::cmp::Ordering;
use std::collections::BinaryHeap;

use pabaru_layout::CELL_DOTS;
use pabaru_layout::grid::GRID_OFFSET;

use crate::gray::Gray;
use crate::grid::GridModel;

/// A point in scan pixels.
pub type Pt = (f64, f64);

/// Nodes of a `cols × rows` cell grid (i.e. `(cols+1) × (rows+1)` nodes).
#[derive(Debug, Clone)]
pub struct Lattice {
    pub cols: u32,
    pub rows: u32,
    /// Node positions (row by row). `None` = the node lies off the paper.
    pub nodes: Vec<Option<Pt>>,
    /// Node confidence (higher = more distinct intersection).
    pub confidence: Vec<f64>,
}

impl Lattice {
    /// Index of node `(i, j)` – `i` is the line column, `j` the line row.
    #[inline]
    pub fn idx(&self, i: u32, j: u32) -> usize {
        (j * (self.cols + 1) + i) as usize
    }

    fn node(&self, i: i64, j: i64) -> Option<Pt> {
        if i < 0 || j < 0 || i > self.cols as i64 || j > self.rows as i64 {
            return None;
        }
        self.nodes[self.idx(i as u32, j as u32)]
    }

    /// Initial lattice from the global model (frame + homography).
    pub fn from_model(model: &GridModel) -> Self {
        let (cols, rows) = (model.cols, model.rows);
        let mut nodes = Vec::with_capacity(((cols + 1) * (rows + 1)) as usize);
        for j in 0..=rows {
            for i in 0..=cols {
                // The grid line centre lies on dot GRID_OFFSET + 41·k (+0.5 = dot centre).
                let u = (GRID_OFFSET + i * CELL_DOTS) as f64 + 0.5;
                let v = (GRID_OFFSET + j * CELL_DOTS) as f64 + 0.5;
                nodes.push(Some(model.dots_to_px.apply(u, v)));
            }
        }
        let n = nodes.len();
        Self {
            cols,
            rows,
            nodes,
            confidence: vec![0.0; n],
        }
    }

    /// Corners of cell `(c, r)`: top-left, top-right, bottom-left, bottom-right.
    pub fn cell_corners(&self, c: u32, r: u32) -> Option<[Pt; 4]> {
        Some([
            self.nodes[self.idx(c, r)]?,
            self.nodes[self.idx(c + 1, r)]?,
            self.nodes[self.idx(c, r + 1)]?,
            self.nodes[self.idx(c + 1, r + 1)]?,
        ])
    }

    /// Local grid vectors at a node (a step of one **dot** along columns and
    /// rows), estimated from the neighbouring nodes.
    fn local_axes(&self, i: u32, j: u32) -> Option<(Pt, Pt)> {
        let (i, j) = (i as i64, j as i64);
        let axis = |a: Option<Pt>, b: Option<Pt>, steps: f64| {
            let (a, b) = (a?, b?);
            Some(((b.0 - a.0) / steps, (b.1 - a.1) / steps))
        };
        let cell = CELL_DOTS as f64;
        let u = axis(self.node(i - 1, j), self.node(i + 1, j), 2.0 * cell)
            .or_else(|| axis(self.node(i, j), self.node(i + 1, j), cell))
            .or_else(|| axis(self.node(i - 1, j), self.node(i, j), cell))?;
        let v = axis(self.node(i, j - 1), self.node(i, j + 1), 2.0 * cell)
            .or_else(|| axis(self.node(i, j), self.node(i, j + 1), cell))
            .or_else(|| axis(self.node(i, j - 1), self.node(i, j), cell))?;
        Some((u, v))
    }
}

/// Darkness at a point (0 = white, 255 = black).
#[inline]
fn dark(img: &Gray, p: Pt) -> f64 {
    255.0 - img.sample(p.0, p.1)
}

/// Line "ridge" at point `p`: darkness minus the mean darkness 1.2 dots away
/// on both sides along `n` (perpendicular to the line).
#[inline]
fn ridge(img: &Gray, p: Pt, n: Pt) -> f64 {
    let side = |s: f64| dark(img, (p.0 + n.0 * s, p.1 + n.1 * s));
    dark(img, p) - 0.5 * (side(1.2) + side(-1.2))
}

/// Finds the shift (in dots) of the line passing through point `c` in
/// direction `along` (the line), searching in direction `across`
/// (perpendicular). Returns (shift, contrast).
fn search_line(img: &Gray, c: Pt, along: Pt, across: Pt, radius: f64) -> (f64, f64) {
    // Points on the cross arm: 1.5 to 10 dots from the node on both sides
    // (the centre is skipped – the perpendicular line passes there, which is
    // always dark).
    const ARM: [f64; 10] = [1.5, 2.5, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5];
    let steps = (radius / 0.25).round() as i32;
    let mut scores = Vec::with_capacity((2 * steps + 1) as usize);
    for k in -steps..=steps {
        let off = k as f64 * 0.25;
        let base = (c.0 + across.0 * off, c.1 + across.1 * off);
        let mut sum = 0.0;
        for t in ARM {
            for sgn in [-1.0, 1.0] {
                let p = (base.0 + along.0 * t * sgn, base.1 + along.1 * t * sgn);
                sum += ridge(img, p, across);
            }
        }
        scores.push((off, sum / (2 * ARM.len()) as f64));
    }
    // Centre of the maximum plateau (with wide lines several shifts share the same score).
    let max = scores.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
    let mut sorted: Vec<f64> = scores.iter().map(|s| s.1).collect();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];
    let limit = max - (max - median) * 0.1;
    let best: Vec<f64> = scores
        .iter()
        .filter(|s| s.1 >= limit)
        .map(|s| s.0)
        .collect();
    let offset = best.iter().sum::<f64>() / best.len() as f64;
    (offset, max - median)
}

/// Refines a node around point `pred`. Returns (position, confidence).
fn refine_node(img: &Gray, pred: Pt, u: Pt, v: Pt, radius: f64) -> (Pt, f64) {
    let mut p = pred;
    let mut conf = 0.0;
    // Two iterations: shift across the vertical line (along u) and across the horizontal one (along v).
    for iter in 0..2 {
        let r = if iter == 0 { radius } else { 1.0 };
        let (du, cu) = search_line(img, p, v, u, r);
        p = (p.0 + u.0 * du, p.1 + u.1 * du);
        let (dv, cv) = search_line(img, p, u, v, r);
        p = (p.0 + v.0 * dv, p.1 + v.1 * dv);
        conf = cu.min(cv);
    }
    (p, conf)
}

/// Priority queue item (best-first by the parent's confidence).
struct Item {
    priority: f64,
    node: (u32, u32),
    pred: Pt,
}

impl PartialEq for Item {
    fn eq(&self, other: &Self) -> bool {
        self.priority == other.priority
    }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Item {
    fn cmp(&self, other: &Self) -> Ordering {
        self.priority.total_cmp(&other.priority)
    }
}

/// Position of a line that merges with the frame, from the **data dot columns**.
///
/// Data dot centres lie exactly 2, 3, 4 … dots from the line centre (dots
/// and lines share the raster). The mean cross profile over 3 columns (dot
/// centre minus the gaps on both sides) therefore locates the line without
/// bias – independent of the frame thickness and of toner spread (which
/// widens a dot equally on both sides). The first column (1 dot from the
/// line) is skipped so that samples never hit the line itself.
///
/// The raster is periodic: the profile has maxima every 1 dot and they all
/// "fit". So the dot columns determine the phase exactly, but which peak is
/// the right one must be decided by another estimate – `target` (the shift
/// where we expect the line). The search covers only ±0.5 dot around `p`
/// (a wider range caught false peaks near the line), so the prediction `p`
/// must be accurate to ±0.5 dot; of the peaks found, the one closest to
/// `target` is chosen.
///
/// Returns the shift (in dots, along `n_in` = into the grid), or `None` if
/// the signal is weak.
fn dot_column_offset(img: &Gray, p: Pt, n_in: Pt, arms: &[Pt], target: f64) -> Option<f64> {
    let at = |o: f64, a: &Pt| {
        let q = (p.0 + n_in.0 * o + a.0, p.1 + n_in.1 * o + a.1);
        dark(img, q)
    };
    let scores: Vec<(f64, f64)> = (-20..=20)
        .map(|k| {
            let o = k as f64 * 0.025;
            let mut sum = 0.0;
            for col in 2..=4 {
                let c = col as f64 + o;
                for a in arms {
                    sum += at(c, a) - 0.5 * (at(c - 0.5, a) + at(c + 0.5, a));
                }
            }
            (o, sum / (3 * arms.len().max(1)) as f64)
        })
        .collect();
    let max = scores.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
    if max <= 30.0 {
        return None;
    }
    // Peaks = contiguous runs above half the maximum; take the plateau centre of each.
    let mut peaks = Vec::new();
    let mut k = 0;
    while k < scores.len() {
        if scores[k].1 < max * 0.5 {
            k += 1;
            continue;
        }
        let start = k;
        while k < scores.len() && scores[k].1 >= max * 0.5 {
            k += 1;
        }
        let segment = &scores[start..k];
        let seg_max = segment
            .iter()
            .map(|s| s.1)
            .fold(f64::NEG_INFINITY, f64::max);
        peaks.push(contiguous_plateau_center(segment, seg_max * 0.95));
    }
    peaks
        .into_iter()
        .min_by(|a, b| (a - target).abs().total_cmp(&(b - target).abs()))
}

/// Centre of the **contiguous** plateau around the main maximum (points above `limit`).
///
/// Averaging all points above the threshold is not enough: at the edge of
/// the range there may be part of the maximum of the neighbouring dot
/// column, which would shift the result.
fn contiguous_plateau_center(scores: &[(f64, f64)], limit: f64) -> f64 {
    let best = scores
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.1.total_cmp(&b.1.1))
        .map(|(i, _)| i)
        .unwrap_or(0);
    let mut lo = best;
    while lo > 0 && scores[lo - 1].1 >= limit {
        lo -= 1;
    }
    let mut hi = best;
    while hi + 1 < scores.len() && scores[hi + 1].1 >= limit {
        hi += 1;
    }
    (scores[lo].0 + scores[hi].0) / 2.0
}

/// Line centre position from the cross darkness profile: the maximum, then
/// the centroid in a ±0.6-dot window. `arms` are points along the line
/// (vectors from `p`). Returns the shift along `across` (in dots), or `None`
/// if the contrast is weak.
fn line_center_offset(img: &Gray, p: Pt, across: Pt, arms: &[Pt]) -> Option<f64> {
    const STEP: f64 = 0.125;
    // Cross darkness profile over ±1.5 dots.
    let profile: Vec<(f64, f64)> = (-12..=12)
        .map(|k| {
            let off = k as f64 * STEP;
            let base = (p.0 + across.0 * off, p.1 + across.1 * off);
            let mean = arms
                .iter()
                .map(|a| dark(img, (base.0 + a.0, base.1 + a.1)))
                .sum::<f64>()
                / arms.len().max(1) as f64;
            (off, mean)
        })
        .collect();
    let mut sorted: Vec<f64> = profile.iter().map(|v| v.1).collect();
    sorted.sort_by(f64::total_cmp);
    let baseline = sorted[sorted.len() / 2];
    let (peak_off, peak) = profile.iter().copied().max_by(|a, b| a.1.total_cmp(&b.1))?;
    // The line must be clearly darker than its surroundings (continuous line vs. random dots).
    if peak - baseline < 50.0 {
        return None;
    }
    // A too-wide maximum = thick frame or dark smudge, not a line.
    let half = baseline + (peak - baseline) / 2.0;
    let wide = profile.iter().filter(|v| v.1 > half).count() as f64 * STEP;
    if wide > 1.2 {
        return None;
    }
    // Fine: darkness centroid in a ±0.6-dot window around the maximum.
    // Neighbouring data dots start only ~0.67 dot from the line centre, so
    // they do not reach into the window.
    let (mut sw, mut so) = (0.0, 0.0);
    for &(off, v) in &profile {
        if (off - peak_off).abs() <= 0.6 {
            let w = (v - baseline).max(0.0);
            sw += w;
            so += w * off;
        }
    }
    (sw > 0.0).then(|| so / sw)
}

/// "Step" at the inner edge of the frame (coarse estimate): dark line and
/// the band behind it (`−n_in`), lighter data area in front of it
/// (`+n_in`). Biased by up to ~0.4 dot, but never jumps by a whole dot
/// column.
#[inline]
fn step(img: &Gray, q: Pt, n_in: Pt) -> f64 {
    dark(img, q) + dark(img, (q.0 - n_in.0, q.1 - n_in.1))
        - 2.0 * dark(img, (q.0 + n_in.0, q.1 + n_in.1))
}

/// How lines that merge with the frame are measured.
///
/// Each one is better in a different situation, so the decoder tries both
/// (the result is always verified by RS + CRC):
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryMode {
    /// Extrapolation from inner lines + data dot columns (±0.5 dot).
    /// Accurate on a clean or mildly damaged page (no needless corrections).
    Exact,
    /// "Step" detector over a wide range. Slightly biased, but tracks even
    /// a strongly wavy edge of crumpled paper.
    Step,
}

/// Shift (in dots, along `n_in`) from point `p` to the centre of the line
/// that merges with the frame, according to the chosen mode.
/// `arms` = points along the line for the step detector, `dot_arms` =
/// points at whole dots (centres of data dot rows) for the column detector.
fn frame_line_offset(
    img: &Gray,
    p: Pt,
    n_in: Pt,
    arms: &[Pt],
    dot_arms: &[Pt],
    radius: f64,
    mode: BoundaryMode,
) -> Option<f64> {
    match mode {
        BoundaryMode::Exact => {
            // The dot columns give the exact phase (peaks every 1 dot); we
            // pick the peak closest to the prediction `p` (extrapolation
            // from inner lines). The step detector did not work as a tie-
            // breaker: its bias can exceed half a dot. A wavy edge is
            // handled by the fallback mode `Step`.
            dot_column_offset(img, p, n_in, dot_arms, 0.0)
        }
        BoundaryMode::Step => line_offset(p, n_in, arms, radius, 60.0, |q| step(img, q, n_in)),
    }
}

/// Generic 1D line position search: the shift `off` (in dots) along
/// `across` that maximises the sum of `score(point)` over the arms `arms`
/// (vectors from the centre along the line). Returns the shift, or `None`
/// if the signal is weak.
fn line_offset(
    c: Pt,
    across: Pt,
    arms: &[Pt],
    radius: f64,
    min_score: f64,
    score: impl Fn(Pt) -> f64,
) -> Option<f64> {
    let steps = (radius / 0.25).round() as i32;
    let mut scores = Vec::with_capacity((2 * steps + 1) as usize);
    for k in -steps..=steps {
        let off = k as f64 * 0.25;
        let base = (c.0 + across.0 * off, c.1 + across.1 * off);
        let s: f64 = arms
            .iter()
            .map(|a| score((base.0 + a.0, base.1 + a.1)))
            .sum::<f64>()
            / arms.len().max(1) as f64;
        scores.push((off, s));
    }
    let max = scores.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
    if max < min_score {
        return None;
    }
    let min = scores
        .iter()
        .map(|s| s.1)
        .fold(f64::INFINITY, f64::min)
        .max(0.0);
    let limit = max - (max - min) * 0.1;
    let best: Vec<f64> = scores
        .iter()
        .filter(|s| s.1 >= limit)
        .map(|s| s.0)
        .collect();
    Some(best.iter().sum::<f64>() / best.len() as f64)
}

/// Arms for the dot column detector: **whole** dots 1–10 along `dir`, so
/// that samples land on the centres of data dot rows (not in the gaps
/// between them).
fn dot_arms(dir: Pt, both: bool) -> Vec<Pt> {
    let mut out = Vec::new();
    for t in 1..=10 {
        let t = t as f64;
        out.push((dir.0 * t, dir.1 * t));
        if both {
            out.push((-dir.0 * t, -dir.1 * t));
        }
    }
    out
}

/// Cross arms: points 1.5–10.5 dots away along `dir`
/// (`both` = on both sides, otherwise only in direction `dir`).
fn arms(dir: Pt, both: bool) -> Vec<Pt> {
    let mut out = Vec::new();
    for k in 0..10 {
        let t = 1.5 + k as f64;
        out.push((dir.0 * t, dir.1 * t));
        if both {
            out.push((-dir.0 * t, -dir.1 * t));
        }
    }
    out
}

/// Refines nodes on the grid boundary, where lines merge with the frame.
///
/// Called only for a lattice found via the frame (we know the boundary =
/// the frame). The position is first predicted by extrapolation from inner
/// nodes (which track the deformation) and then searched for:
/// * the line coinciding with the frame by the "step" detector ([`step`]),
/// * the perpendicular line (leading inwards) by a ridge with arms pointing
///   inwards only.
pub fn refine_frame_boundary(img: &Gray, lat: &mut Lattice, mode: BoundaryMode) {
    let (cols, rows) = (lat.cols, lat.rows);
    if cols < 2 || rows < 2 {
        return;
    }
    let get = |lat: &Lattice, i: u32, j: u32| lat.nodes[lat.idx(i, j)];
    let extrapolate = |a: Option<Pt>, b: Option<Pt>| -> Option<Pt> {
        let (a, b) = (a?, b?);
        Some((2.0 * a.0 - b.0, 2.0 * a.1 - b.1))
    };
    // Order: first boundary nodes other than corners (they have inner neighbours), then corners.
    let mut order: Vec<(u32, u32)> = Vec::new();
    for j in 0..=rows {
        for i in 0..=cols {
            let edge_i = i == 0 || i == cols;
            let edge_j = j == 0 || j == rows;
            if edge_i ^ edge_j {
                order.push((i, j));
            }
        }
    }
    order.extend([(0, 0), (cols, 0), (0, rows), (cols, rows)]);

    for (i, j) in order {
        let Some((u, v)) = lat.local_axes(i, j) else {
            continue;
        };
        // Prediction from the interior (mean of the available extrapolations).
        let mut preds = Vec::new();
        if i == 0 {
            preds.extend(extrapolate(get(lat, 1, j), get(lat, 2, j)));
        }
        if i == cols {
            preds.extend(extrapolate(get(lat, cols - 1, j), get(lat, cols - 2, j)));
        }
        if j == 0 {
            preds.extend(extrapolate(get(lat, i, 1), get(lat, i, 2)));
        }
        if j == rows {
            preds.extend(extrapolate(get(lat, i, rows - 1), get(lat, i, rows - 2)));
        }
        let k = lat.idx(i, j);
        let Some(mut p) = (!preds.is_empty())
            .then(|| {
                let n = preds.len() as f64;
                (
                    preds.iter().map(|q| q.0).sum::<f64>() / n,
                    preds.iter().map(|q| q.1).sum::<f64>() / n,
                )
            })
            .or(lat.nodes[k])
        else {
            continue;
        };
        // Directions into the grid.
        let in_u = if i == 0 { u } else { (-u.0, -u.1) };
        let in_v = if j == 0 { v } else { (-v.0, -v.1) };
        let on_u_edge = i == 0 || i == cols; // the vertical line is the frame
        let on_v_edge = j == 0 || j == rows; // the horizontal line is the frame

        // Corner (both lines are the frame): in exact mode it is the
        // intersection of two boundary lines whose neighbouring nodes are
        // already measured – extrapolation is enough. Measuring at a corner
        // is unreliable (too few data dots).
        if on_u_edge && on_v_edge && mode == BoundaryMode::Exact {
            lat.nodes[k] = Some(p);
            continue;
        }

        // The node's vertical line (position along u). If it is the frame:
        // from the data dot columns next to it; otherwise the line profile
        // with arms pointing inwards only (outwards runs the frame, which is
        // dark all over).
        let vert_arms = arms(if on_v_edge { in_v } else { v }, !on_v_edge);
        let du = if on_u_edge {
            // Shift in the inward direction → shift along u.
            let dots = dot_arms(if on_v_edge { in_v } else { v }, !on_v_edge);
            frame_line_offset(img, p, in_u, &vert_arms, &dots, 2.5, mode)
                .map(|t| if i == 0 { t } else { -t })
        } else {
            match mode {
                BoundaryMode::Exact => line_center_offset(img, p, u, &vert_arms),
                BoundaryMode::Step => {
                    line_offset(p, u, &vert_arms, 2.5, 40.0, |q| ridge(img, q, u))
                }
            }
        };
        if let Some(du) = du {
            p = (p.0 + u.0 * du, p.1 + u.1 * du);
        }
        // The node's horizontal line (position along v).
        let hor_arms = arms(if on_u_edge { in_u } else { u }, !on_u_edge);
        let dv = if on_v_edge {
            frame_line_offset(
                img,
                p,
                in_v,
                &hor_arms,
                &dot_arms(if on_u_edge { in_u } else { u }, !on_u_edge),
                2.5,
                mode,
            )
            .map(|t| if j == 0 { t } else { -t })
        } else {
            match mode {
                BoundaryMode::Exact => line_center_offset(img, p, v, &hor_arms),
                BoundaryMode::Step => line_offset(p, v, &hor_arms, 2.5, 40.0, |q| ridge(img, q, v)),
            }
        };
        if let Some(dv) = dv {
            p = (p.0 + v.0 * dv, p.1 + v.1 * dv);
        }
        lat.nodes[k] = Some(p);
    }
}

/// Refines all lattice nodes (see the module description).
pub fn track(img: &Gray, lat: &mut Lattice) {
    let initial = lat.nodes.clone();
    let n = initial.len();
    if n == 0 {
        return;
    }
    // Local axes from the initial lattice.
    let axes: Vec<Option<(Pt, Pt)>> = (0..=lat.rows)
        .flat_map(|j| (0..=lat.cols).map(move |i| (i, j)))
        .map(|(i, j)| lat.local_axes(i, j))
        .collect();

    // 1) Quick refinement of all nodes from their initial positions → choose the start.
    let mut start = None;
    let mut best_conf = f64::NEG_INFINITY;
    for j in 0..=lat.rows {
        for i in 0..=lat.cols {
            let k = lat.idx(i, j);
            let (Some(p), Some((u, v))) = (initial[k], axes[k]) else {
                continue;
            };
            // Boundary nodes lie on the thick frame – better to start inside.
            if i == 0 || j == 0 || i == lat.cols || j == lat.rows {
                continue;
            }
            let (_, conf) = refine_node(img, p, u, v, 2.0);
            if conf > best_conf {
                best_conf = conf;
                start = Some((i, j));
            }
        }
    }
    let Some(start) = start.or(Some((0, 0))) else {
        return;
    };

    // 2) Best-first propagation from the start.
    let mut done = vec![false; n];
    let mut heap = BinaryHeap::new();
    let s = lat.idx(start.0, start.1);
    if let Some(p) = initial[s] {
        heap.push(Item {
            priority: f64::INFINITY,
            node: start,
            pred: p,
        });
    }
    while let Some(Item {
        node: (i, j), pred, ..
    }) = heap.pop()
    {
        let k = lat.idx(i, j);
        if done[k] {
            continue;
        }
        let Some((u0, v0)) = axes[k] else { continue };
        // Axes recomputed from already refined neighbours (more accurate under deformation).
        let (u, v) = lat.local_axes(i, j).unwrap_or((u0, v0));
        let (mut p, mut conf) = refine_node(img, pred, u, v, 4.0);
        if conf < 8.0 {
            // Weak intersection (fold, smudge) – try a larger neighbourhood.
            let (p2, c2) = refine_node(img, pred, u, v, 7.0);
            if c2 > conf {
                (p, conf) = (p2, c2);
            }
        }
        if conf < 4.0 {
            // Uncertain node: keep the prediction.
            p = pred;
        }
        done[k] = true;
        lat.nodes[k] = Some(p);
        lat.confidence[k] = conf;

        // Neighbours: prediction = this node + local grid vector.
        for (di, dj) in [(1i64, 0i64), (-1, 0), (0, 1), (0, -1)] {
            let (ni, nj) = (i as i64 + di, j as i64 + dj);
            if ni < 0 || nj < 0 || ni > lat.cols as i64 || nj > lat.rows as i64 {
                continue;
            }
            let m = lat.idx(ni as u32, nj as u32);
            if done[m] || initial[m].is_none() {
                continue;
            }
            // Vector from the opposite refined neighbour, otherwise from the initial lattice.
            let opposite = (i as i64 - di, j as i64 - dj);
            let opp_done = opposite.0 >= 0
                && opposite.1 >= 0
                && opposite.0 <= lat.cols as i64
                && opposite.1 <= lat.rows as i64
                && done[lat.idx(opposite.0 as u32, opposite.1 as u32)];
            let step = if opp_done {
                let o = lat.nodes[lat.idx(opposite.0 as u32, opposite.1 as u32)].unwrap_or(p);
                (p.0 - o.0, p.1 - o.1)
            } else {
                let (a, b) = (initial[k].unwrap_or(p), initial[m].unwrap_or(p));
                (b.0 - a.0, b.1 - a.1)
            };
            heap.push(Item {
                priority: conf,
                node: (ni as u32, nj as u32),
                pred: (p.0 + step.0, p.1 + step.1),
            });
        }
    }
}

/// Bilinear interpolation between the cell corners (s, t ∈ [0, 1]).
#[inline]
pub fn bilinear(c: &[Pt; 4], s: f64, t: f64) -> Pt {
    let top = (
        c[0].0 + (c[1].0 - c[0].0) * s,
        c[0].1 + (c[1].1 - c[0].1) * s,
    );
    let bottom = (
        c[2].0 + (c[3].0 - c[2].0) * s,
        c[2].1 + (c[3].1 - c[2].1) * s,
    );
    (
        top.0 + (bottom.0 - top.0) * t,
        top.1 + (bottom.1 - top.1) * t,
    )
}

// ---------------------------------------------------------------------------
// Points along the grid lines and the Coons patch
// ---------------------------------------------------------------------------

/// Number of intermediate points on one cell edge (between two nodes).
pub const EDGE_POINTS: usize = 7;

/// Refined points along the grid lines.
///
/// On crumpled paper a line is curved even within a single cell. So between
/// two nodes we additionally locate `EDGE_POINTS` points lying exactly on
/// the line, making the cell edges polylines instead of straight segments.
#[derive(Debug, Clone)]
pub struct Edges {
    /// Horizontal edges: edge from node (i, j) to (i+1, j), index `j·cols + i`.
    horizontal: Vec<Option<[Pt; EDGE_POINTS]>>,
    /// Vertical edges: edge from node (i, j) to (i, j+1), index `j·(cols+1) + i`.
    vertical: Vec<Option<[Pt; EDGE_POINTS]>>,
}

/// Locates a point on the line near `p`: search perpendicular (`across`,
/// one dot) with arms along the line (`along`, ±8 dots – a column of random
/// data dots will then never be entirely black).
fn refine_line_point(img: &Gray, p: Pt, along: Pt, across: Pt) -> Pt {
    let arms: Vec<Pt> = (-8..=8)
        .map(|t| (along.0 * t as f64, along.1 * t as f64))
        .collect();
    match line_center_offset(img, p, across, &arms) {
        Some(off) => (p.0 + across.0 * off, p.1 + across.1 * off),
        None => p,
    }
}

/// Points along the edge from `a` to `b` (the line between two nodes).
fn edge_points(img: &Gray, a: Pt, b: Pt) -> [Pt; EDGE_POINTS] {
    let cell = CELL_DOTS as f64;
    // "One dot" unit vectors along the edge and perpendicular to it.
    let along = ((b.0 - a.0) / cell, (b.1 - a.1) / cell);
    let across = (-along.1, along.0);
    let mut out = [(0.0, 0.0); EDGE_POINTS];
    for (k, o) in out.iter_mut().enumerate() {
        let f = (k + 1) as f64 / (EDGE_POINTS + 1) as f64;
        let p = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
        *o = refine_line_point(img, p, along, across);
    }
    out
}

/// Extrapolates a frame edge from the two neighbouring inner edges:
/// point = 2 · (point on 1st inner line) − (point on 2nd inner line).
fn extrapolate_edge(
    near: Option<[Pt; EDGE_POINTS]>,
    far: Option<[Pt; EDGE_POINTS]>,
) -> Option<[Pt; EDGE_POINTS]> {
    let (near, far) = (near?, far?);
    let mut out = [(0.0, 0.0); EDGE_POINTS];
    for k in 0..EDGE_POINTS {
        out[k] = (2.0 * near[k].0 - far[k].0, 2.0 * near[k].1 - far[k].1);
    }
    Some(out)
}

/// Locates points along all grid edges.
///
/// `frame = Some(_)`: the grid boundary is a thick frame. Lines on the frame
/// are not measured directly (the detector would be biased) but
/// extrapolated from the two nearest inner lines – exact on a clean page,
/// and following the deformation on a wavy one.
pub fn refine_edges(img: &Gray, lat: &Lattice, frame: Option<BoundaryMode>) -> Edges {
    let mode = frame.unwrap_or(BoundaryMode::Exact);
    let frame = frame.is_some();
    let (cols, rows) = (lat.cols, lat.rows);
    let hidx = |i: u32, j: u32| (j * cols + i) as usize;
    let vidx = |i: u32, j: u32| (j * (cols + 1) + i) as usize;
    let mut horizontal = Vec::with_capacity((cols * (rows + 1)) as usize);
    for j in 0..=rows {
        for i in 0..cols {
            let (a, b) = (lat.nodes[lat.idx(i, j)], lat.nodes[lat.idx(i + 1, j)]);
            let on_frame = frame && (j == 0 || j == rows);
            horizontal.push(if on_frame {
                None
            } else {
                a.zip(b).map(|(a, b)| edge_points(img, a, b))
            });
        }
    }
    let mut vertical = Vec::with_capacity(((cols + 1) * rows) as usize);
    for j in 0..rows {
        for i in 0..=cols {
            let (a, b) = (lat.nodes[lat.idx(i, j)], lat.nodes[lat.idx(i, j + 1)]);
            let on_frame = frame && (i == 0 || i == cols);
            vertical.push(if on_frame {
                None
            } else {
                a.zip(b).map(|(a, b)| edge_points(img, a, b))
            });
        }
    }
    // Frame edge between nodes `a`, `b`. Initial points:
    // * Exact: extrapolation from two inner lines (accurate on flat paper),
    // * Step: straight line between the measured boundary nodes (follows a wavy edge).
    // Then refinement with the chosen detector.
    let frame_edge = |a: Option<Pt>,
                      b: Option<Pt>,
                      near: Option<[Pt; EDGE_POINTS]>,
                      far: Option<[Pt; EDGE_POINTS]>| {
        let near = near?;
        let mut pts = match mode {
            BoundaryMode::Exact => extrapolate_edge(Some(near), far)?,
            BoundaryMode::Step => {
                let (a, b) = (a?, b?);
                let mut out = [(0.0, 0.0); EDGE_POINTS];
                for (k, o) in out.iter_mut().enumerate() {
                    let f = (k + 1) as f64 / (EDGE_POINTS + 1) as f64;
                    *o = (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f);
                }
                out
            }
        };
        for k in 0..EDGE_POINTS {
            let p = pts[k];
            // Inward direction: towards the point on the nearest inner line,
            // length of one dot (the inner line is 41 dots away, hence / 41).
            let cell = CELL_DOTS as f64;
            let n_in = ((near[k].0 - p.0) / cell, (near[k].1 - p.1) / cell);
            let along = (-n_in.1, n_in.0);
            let arm: Vec<Pt> = (-4..=4)
                .map(|t| (along.0 * t as f64, along.1 * t as f64))
                .collect();
            if let Some(t) = frame_line_offset(img, p, n_in, &arm, &arm, 1.5, mode) {
                pts[k] = (p.0 + n_in.0 * t, p.1 + n_in.1 * t);
            }
        }
        Some(pts)
    };
    let node = |i: u32, j: u32| lat.nodes[lat.idx(i, j)];
    if frame && rows >= 3 {
        for i in 0..cols {
            horizontal[hidx(i, 0)] = frame_edge(
                node(i, 0),
                node(i + 1, 0),
                horizontal[hidx(i, 1)],
                horizontal[hidx(i, 2)],
            );
            horizontal[hidx(i, rows)] = frame_edge(
                node(i, rows),
                node(i + 1, rows),
                horizontal[hidx(i, rows - 1)],
                horizontal[hidx(i, rows - 2)],
            );
        }
    }
    if frame && cols >= 3 {
        for j in 0..rows {
            vertical[vidx(0, j)] = frame_edge(
                node(0, j),
                node(0, j + 1),
                vertical[vidx(1, j)],
                vertical[vidx(2, j)],
            );
            vertical[vidx(cols, j)] = frame_edge(
                node(cols, j),
                node(cols, j + 1),
                vertical[vidx(cols - 1, j)],
                vertical[vidx(cols - 2, j)],
            );
        }
    }
    Edges {
        horizontal,
        vertical,
    }
}

/// Edges of one cell as polylines (each with `EDGE_POINTS + 2` points).
#[derive(Debug, Clone)]
pub struct Patch {
    pub top: Vec<Pt>,
    pub bottom: Vec<Pt>,
    pub left: Vec<Pt>,
    pub right: Vec<Pt>,
}

/// Point on a polyline for parameter `s ∈ [0, 1]`.
fn polyline(points: &[Pt], s: f64) -> Pt {
    let segs = (points.len() - 1) as f64;
    let x = (s * segs).clamp(0.0, segs);
    let i = (x.floor() as usize).min(points.len() - 2);
    let f = x - i as f64;
    let (a, b) = (points[i], points[i + 1]);
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
}

impl Patch {
    /// Coons patch: a point inside the cell interpolated from all four edges.
    ///
    /// `P(s,t) = (1−t)·T(s) + t·B(s) + (1−s)·L(t) + s·R(t) − bilinear(corners)`
    pub fn point(&self, s: f64, t: f64) -> Pt {
        let (tp, bt) = (polyline(&self.top, s), polyline(&self.bottom, s));
        let (lf, rt) = (polyline(&self.left, t), polyline(&self.right, t));
        let corners = [
            self.top[0],
            *self.top.last().expect("non-empty"),
            self.bottom[0],
            *self.bottom.last().expect("non-empty"),
        ];
        let bl = bilinear(&corners, s, t);
        (
            (1.0 - t) * tp.0 + t * bt.0 + (1.0 - s) * lf.0 + s * rt.0 - bl.0,
            (1.0 - t) * tp.1 + t * bt.1 + (1.0 - s) * lf.1 + s * rt.1 - bl.1,
        )
    }

    /// Cell with straight edges (from the corners only) – a fallback for when
    /// refining points along the lines fails (e.g. very small dots on the scan).
    pub fn straight(&self) -> Patch {
        Self::from_corners(self.corners())
    }

    /// Cell with straight edges from the given corners (TL, TR, BL, BR).
    pub fn from_corners(corners: [Pt; 4]) -> Patch {
        let [tl, tr, bl, br] = corners;
        let line = |a: Pt, b: Pt| -> Vec<Pt> {
            (0..=EDGE_POINTS + 1)
                .map(|k| {
                    let f = k as f64 / (EDGE_POINTS + 1) as f64;
                    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
                })
                .collect()
        };
        Patch {
            top: line(tl, tr),
            bottom: line(bl, br),
            left: line(tl, bl),
            right: line(tr, br),
        }
    }

    /// Cell corners (top-left, top-right, bottom-left, bottom-right).
    pub fn corners(&self) -> [Pt; 4] {
        [
            self.top[0],
            *self.top.last().expect("non-empty"),
            self.bottom[0],
            *self.bottom.last().expect("non-empty"),
        ]
    }
}

impl Edges {
    /// Edges of cell `(c, r)` (or `None` if any corner is missing).
    pub fn patch(&self, lat: &Lattice, c: u32, r: u32) -> Option<Patch> {
        let [tl, tr, bl, br] = lat.cell_corners(c, r)?;
        let h = |j: u32| self.horizontal[(j * lat.cols + c) as usize];
        let v = |i: u32| self.vertical[(r * (lat.cols + 1) + i) as usize];
        let line = |a: Pt, mid: Option<[Pt; EDGE_POINTS]>, b: Pt| {
            let mut pts = vec![a];
            match mid {
                Some(m) => pts.extend_from_slice(&m),
                None => {
                    // No intermediate points: straight line between the corners.
                    for k in 1..=EDGE_POINTS {
                        let f = k as f64 / (EDGE_POINTS + 1) as f64;
                        pts.push((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f));
                    }
                }
            }
            pts.push(b);
            pts
        };
        Some(Patch {
            top: line(tl, h(r), tr),
            bottom: line(bl, h(r + 1), br),
            left: line(tl, v(c), bl),
            right: line(tr, v(c + 1), br),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bilinear_corners() {
        let c = [(0.0, 0.0), (10.0, 0.0), (0.0, 20.0), (10.0, 20.0)];
        assert_eq!(bilinear(&c, 0.0, 0.0), (0.0, 0.0));
        assert_eq!(bilinear(&c, 1.0, 1.0), (10.0, 20.0));
        assert_eq!(bilinear(&c, 0.5, 0.5), (5.0, 10.0));
    }

    #[test]
    fn finds_shifted_line() {
        // Vertical line 2 px wide at x = 20 (pixels 20–21), dot = 2 px.
        let mut g = Gray::new(60, 60, 230);
        for y in 0..60 {
            g.set(20, y, 20);
            g.set(21, y, 20);
        }
        // Search from x = 18 (shift by +1 dot = 2 px).
        let (off, contrast) = search_line(&g, (18.0, 30.0), (0.0, 2.0), (2.0, 0.0), 3.0);
        assert!((off - 1.5).abs() < 0.3, "offset {off}");
        assert!(contrast > 50.0);
    }
}
