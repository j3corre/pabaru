//! Homography – a projective transformation of the plane.
//!
//! Describes how flat paper maps onto the scan: translation, rotation,
//! scaling, shear and perspective. It is determined from 4 point pairs
//! (frame corners on paper ↔ corners on the scan).

/// Transformation `(u, v) -> (x, y)`:
/// `x = (h0 u + h1 v + h2) / (h6 u + h7 v + 1)`,
/// `y = (h3 u + h4 v + h5) / (h6 u + h7 v + 1)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Homography {
    h: [f64; 8],
}

impl Homography {
    /// Computes the homography from 4 point pairs `src[i] -> dst[i]`.
    /// Returns `None` if the points are degenerate (e.g. three collinear).
    pub fn from_points(src: [(f64, f64); 4], dst: [(f64, f64); 4]) -> Option<Self> {
        // System of 8 linear equations in 8 unknowns (8×9 matrix incl. right-hand side).
        let mut m = [[0.0f64; 9]; 8];
        for i in 0..4 {
            let (u, v) = src[i];
            let (x, y) = dst[i];
            m[2 * i] = [u, v, 1.0, 0.0, 0.0, 0.0, -u * x, -v * x, x];
            m[2 * i + 1] = [0.0, 0.0, 0.0, u, v, 1.0, -u * y, -v * y, y];
        }
        // Gaussian elimination with partial pivoting.
        for col in 0..8 {
            let pivot = (col..8).max_by(|&a, &b| m[a][col].abs().total_cmp(&m[b][col].abs()))?;
            if m[pivot][col].abs() < 1e-12 {
                return None;
            }
            m.swap(col, pivot);
            let pivot_row = m[col];
            for (row, r) in m.iter_mut().enumerate() {
                if row != col {
                    let f = r[col] / pivot_row[col];
                    for (k, v) in r.iter_mut().enumerate().skip(col) {
                        *v -= f * pivot_row[k];
                    }
                }
            }
        }
        let mut h = [0.0; 8];
        for i in 0..8 {
            h[i] = m[i][8] / m[i][i];
        }
        Some(Self { h })
    }

    /// Maps point `(u, v)` to `(x, y)`.
    #[inline]
    pub fn apply(&self, u: f64, v: f64) -> (f64, f64) {
        let h = &self.h;
        let w = h[6] * u + h[7] * v + 1.0;
        (
            (h[0] * u + h[1] * v + h[2]) / w,
            (h[3] * u + h[4] * v + h[5]) / w,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_match() {
        let src = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let dst = [(10.0, 20.0), (110.0, 25.0), (105.0, 130.0), (5.0, 120.0)];
        let h = Homography::from_points(src, dst).unwrap();
        for i in 0..4 {
            let (x, y) = h.apply(src[i].0, src[i].1);
            assert!((x - dst[i].0).abs() < 1e-9 && (y - dst[i].1).abs() < 1e-9);
        }
    }

    #[test]
    fn degenerate_points() {
        let src = [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0)];
        assert!(Homography::from_points(src, src).is_none());
    }
}
