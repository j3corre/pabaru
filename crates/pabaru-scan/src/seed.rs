//! Finding the grid without a frame – for torn-off pieces and damaged edges.
//!
//! When the frame is missing (the page is torn, a corner is ripped off), we
//! rely on the regularity of the grid itself:
//!
//! 1. **Angle** – dark pixels are projected onto an axis at various angles.
//!    At the correct angle they concentrate into narrow bands (lines and dot
//!    rows), so the histogram is "peaky" (large sum of squares).
//! 2. **Dot pitch** – from the autocorrelation of the profile (dots form a
//!    regular sequence).
//! 3. **Grid line pitch and phase** – lines are every 41 dots and are
//!    continuous, so they form the highest regular peaks in the profile
//!    (a "comb").
//! 4. This yields an initial node lattice, which `lattice::track` refines.
//!
//! We do not need absolute column and row numbers – every block carries its
//! index in its header.

use pabaru_layout::CELL_DOTS;

use crate::frame::Region;
use crate::gray::Gray;
use crate::lattice::Lattice;

/// Histogram of point projections onto the axis with direction `angle` (bin = 1 px).
fn projection(points: &[(f32, f32)], angle: f64) -> (Vec<f64>, f64) {
    let (s, c) = angle.sin_cos();
    let (s, c) = (s as f32, c as f32);
    let qs: Vec<f32> = points.iter().map(|&(x, y)| x * c + y * s).collect();
    let min = qs.iter().copied().fold(f32::INFINITY, f32::min);
    let max = qs.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    let mut hist = vec![0.0; (max - min) as usize + 2];
    for q in qs {
        hist[(q - min) as usize] += 1.0;
    }
    (hist, min as f64)
}

/// "Peakiness" of a histogram: sum of squares (higher = more concentrated).
fn peakiness(hist: &[f64]) -> f64 {
    hist.iter().map(|h| h * h).sum()
}

/// Best angle in the range ±45° (coarse in 1° steps, then fine in 0.1° steps).
fn find_angle(points: &[(f32, f32)]) -> f64 {
    let score = |deg: f64| peakiness(&projection(points, deg.to_radians()).0);
    let coarse = (-45..45)
        .map(|d| d as f64)
        .max_by(|a, b| score(*a).total_cmp(&score(*b)))
        .unwrap_or(0.0);
    (-10..=10)
        .map(|k| coarse + k as f64 * 0.1)
        .max_by(|a, b| score(*a).total_cmp(&score(*b)))
        .unwrap_or(coarse)
}

/// Dot pitch (px) from the autocorrelation of the profile.
fn dot_pitch(profile: &[f64]) -> Option<f64> {
    let n = profile.len();
    let mean = profile.iter().sum::<f64>() / n as f64;
    let centered: Vec<f64> = profile.iter().map(|v| v - mean).collect();
    let corr = |lag: usize| -> f64 {
        centered[..n - lag]
            .iter()
            .zip(&centered[lag..])
            .map(|(a, b)| a * b)
            .sum::<f64>()
            / (n - lag) as f64
    };
    let max_lag = 64.min(n / 4);
    if max_lag < 4 {
        return None;
    }
    let r: Vec<f64> = (0..=max_lag).map(corr).collect();
    // Local maxima of the autocorrelation (from lag 2 on).
    let peaks: Vec<usize> = (2..max_lag)
        .filter(|&l| r[l] > r[l - 1] && r[l] >= r[l + 1] && r[l] > 0.0)
        .collect();
    let &best = peaks.iter().max_by(|a, b| r[**a].total_cmp(&r[**b]))?;
    // Prefer the fundamental period: the first maximum that is strong enough.
    let fundamental = peaks
        .iter()
        .copied()
        .find(|&l| r[l] >= 0.6 * r[best])
        .unwrap_or(best);
    // Sub-sample refinement with a parabola through the neighbouring values.
    let l = fundamental;
    let (a, b, c) = (r[l - 1], r[l], r[l + 1]);
    let denom = a - 2.0 * b + c;
    let shift = if denom.abs() > 1e-9 {
        0.5 * (a - c) / denom
    } else {
        0.0
    };
    Some(l as f64 + shift.clamp(-0.5, 0.5))
}

/// Profile value with linear interpolation.
fn at(profile: &[f64], x: f64) -> f64 {
    if x < 0.0 {
        return 0.0;
    }
    let i = x as usize;
    if i + 1 >= profile.len() {
        return 0.0;
    }
    let f = x - i as f64;
    profile[i] * (1.0 - f) + profile[i + 1] * f
}

/// Pitch and phase of the grid lines (a "comb" with pitch ≈ 41 dots).
fn comb(profile: &[f64], pitch0: f64) -> (f64, f64) {
    let mut best = (pitch0, 0.0, f64::NEG_INFINITY);
    let mut pitch = pitch0 * 0.97;
    while pitch <= pitch0 * 1.03 {
        let mut phase = 0.0;
        while phase < pitch {
            let mut sum = 0.0;
            let mut count = 0;
            let mut x = phase;
            while x < profile.len() as f64 {
                sum += at(profile, x);
                count += 1;
                x += pitch;
            }
            let score = sum / count.max(1) as f64;
            if score > best.2 {
                best = (pitch, phase, score);
            }
            phase += 0.5;
        }
        pitch += 0.05 * pitch0 / 20.0;
    }
    (best.0, best.1)
}

/// Size of the central window (px) used to estimate the angle and pitch.
const WINDOW: u32 = 900;

/// Estimates the initial node lattice in the area `region`.
///
/// Angle, pitch and phase are measured in a central window at full
/// resolution (no subsampling – with small dots that would cause aliasing);
/// the lattice is then extended over the whole region.
pub fn lattice_for(img: &Gray, region: &Region, threshold: u8) -> Option<Lattice> {
    let w = region.max_x - region.min_x;
    let h = region.max_y - region.min_y;
    if w < 50 || h < 50 {
        return None;
    }
    // Central window.
    let side = WINDOW.min(w).min(h);
    let cx = region.min_x + w / 2;
    let cy = region.min_y + h / 2;
    let (x0, y0) = (cx - side / 2, cy - side / 2);
    let mut points = Vec::new();
    for y in y0..y0 + side {
        for x in x0..x0 + side {
            if img.get(x as i64, y as i64) <= threshold {
                points.push((x as f32, y as f32));
            }
        }
    }
    if points.len() < 1000 {
        return None;
    }

    let angle = find_angle(&points);
    let (ra, rb) = (
        angle.to_radians(),
        angle.to_radians() + std::f64::consts::FRAC_PI_2,
    );
    // Profiles in two perpendicular directions (u = line columns, v = line rows).
    let (prof_u, min_u) = projection(&points, ra);
    let (prof_v, min_v) = projection(&points, rb);

    let pitch_dot = dot_pitch(&prof_u)
        .or_else(|| dot_pitch(&prof_v))
        .filter(|p| *p >= 1.5)?;
    let line0 = pitch_dot * CELL_DOTS as f64;
    if line0 * 1.2 > side as f64 {
        // The window is smaller than one cell – the line pitch cannot be measured reliably.
        return None;
    }
    let (pu, phase_u) = comb(&prof_u, line0);
    let (pv, phase_v) = comb(&prof_v, line0);
    // Absolute position of one line in the rotated coordinates.
    let (u0, v0) = (min_u + phase_u, min_v + phase_v);

    // Extent of the whole region in the rotated coordinates.
    let (s, c) = ra.sin_cos();
    let corners = [
        (region.min_x as f64, region.min_y as f64),
        (region.max_x as f64, region.min_y as f64),
        (region.min_x as f64, region.max_y as f64),
        (region.max_x as f64, region.max_y as f64),
    ];
    let proj_u = |p: (f64, f64)| p.0 * c + p.1 * s;
    let proj_v = |p: (f64, f64)| -p.0 * s + p.1 * c;
    let umin = corners
        .iter()
        .map(|p| proj_u(*p))
        .fold(f64::INFINITY, f64::min);
    let umax = corners
        .iter()
        .map(|p| proj_u(*p))
        .fold(f64::NEG_INFINITY, f64::max);
    let vmin = corners
        .iter()
        .map(|p| proj_v(*p))
        .fold(f64::INFINITY, f64::min);
    let vmax = corners
        .iter()
        .map(|p| proj_v(*p))
        .fold(f64::NEG_INFINITY, f64::max);
    let i0 = ((umin - u0) / pu).floor() as i64;
    let i1 = ((umax - u0) / pu).ceil() as i64;
    let j0 = ((vmin - v0) / pv).floor() as i64;
    let j1 = ((vmax - v0) / pv).ceil() as i64;
    let cols = (i1 - i0).max(0) as u32;
    let rows = (j1 - j0).max(0) as u32;
    if cols == 0 || rows == 0 {
        return None;
    }
    let mut nodes = Vec::with_capacity(((cols + 1) * (rows + 1)) as usize);
    for j in 0..=rows {
        for i in 0..=cols {
            let u = u0 + (i0 + i as i64) as f64 * pu;
            let v = v0 + (j0 + j as i64) as f64 * pv;
            // Inverse rotation: (u, v) are projections onto the axes (cos, sin) and (−sin, cos).
            let x = u * c - v * s;
            let y = u * s + v * c;
            let inside = x >= region.min_x as f64
                && y >= region.min_y as f64
                && x <= region.max_x as f64
                && y <= region.max_y as f64;
            nodes.push(inside.then_some((x, y)));
        }
    }
    let n = nodes.len();
    Some(Lattice {
        cols,
        rows,
        nodes,
        confidence: vec![0.0; n],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_from_periodic_profile() {
        // Profile with a 6 px period (dots) and stronger lines every 41·6 px.
        let p: Vec<f64> = (0..3000)
            .map(|i| {
                let dot = if i % 6 < 3 { 1.0 } else { 0.0 };
                let line = if i % 246 < 3 { 3.0 } else { 0.0 };
                dot + line
            })
            .collect();
        let pitch = dot_pitch(&p).unwrap();
        assert!((pitch - 6.0).abs() < 0.3, "pitch {pitch}");
        let (lp, phase) = comb(&p, pitch * 41.0);
        assert!((lp - 246.0).abs() < 1.5, "line pitch {lp}");
        assert!(phase < 3.0 || phase > 243.0, "phase {phase}");
    }
}
