//! Grayscale image and basic operations on it.

/// Grayscale image, 1 byte per pixel (0 = black, 255 = white).
///
/// Coordinates: pixel `(i, j)` covers the area `[i, i+1) × [j, j+1)`,
/// so its centre is at `(i + 0.5, j + 0.5)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gray {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl Gray {
    /// New image filled with `value`.
    pub fn new(width: u32, height: u32, value: u8) -> Self {
        Self {
            width,
            height,
            data: vec![value; (width * height) as usize],
        }
    }

    /// Pixel value; returns white outside the image.
    #[inline]
    pub fn get(&self, x: i64, y: i64) -> u8 {
        if x < 0 || y < 0 || x >= self.width as i64 || y >= self.height as i64 {
            return 255;
        }
        self.data[(y as usize) * self.width as usize + x as usize]
    }

    #[inline]
    pub fn set(&mut self, x: u32, y: u32, v: u8) {
        let w = self.width as usize;
        self.data[y as usize * w + x as usize] = v;
    }

    /// Bilinear interpolation at an arbitrary (fractional) point.
    pub fn sample(&self, fx: f64, fy: f64) -> f64 {
        let x = fx - 0.5;
        let y = fy - 0.5;
        let x0 = x.floor();
        let y0 = y.floor();
        let ax = x - x0;
        let ay = y - y0;
        let (x0, y0) = (x0 as i64, y0 as i64);
        let p = |dx: i64, dy: i64| self.get(x0 + dx, y0 + dy) as f64;
        let top = p(0, 0) * (1.0 - ax) + p(1, 0) * ax;
        let bottom = p(0, 1) * (1.0 - ax) + p(1, 1) * ax;
        top * (1.0 - ay) + bottom * ay
    }

    /// Downscales the image `factor`× by averaging blocks of pixels.
    pub fn downscale(&self, factor: u32) -> Gray {
        if factor <= 1 {
            return self.clone();
        }
        let w = self.width.div_ceil(factor);
        let h = self.height.div_ceil(factor);
        let mut out = Gray::new(w, h, 255);
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0u32;
                let mut n = 0u32;
                for yy in y * factor..((y + 1) * factor).min(self.height) {
                    for xx in x * factor..((x + 1) * factor).min(self.width) {
                        sum += self.get(xx as i64, yy as i64) as u32;
                        n += 1;
                    }
                }
                out.set(x, y, (sum / n.max(1)) as u8);
            }
        }
        out
    }

    /// Otsu's method: finds the threshold that best separates dark and light pixels.
    /// Dark pixels are those with value `<= threshold`.
    pub fn otsu_threshold(&self) -> u8 {
        let mut hist = [0u64; 256];
        for &v in &self.data {
            hist[v as usize] += 1;
        }
        let total = self.data.len() as f64;
        let sum_all: f64 = hist
            .iter()
            .enumerate()
            .map(|(i, &c)| i as f64 * c as f64)
            .sum();
        let (mut w_b, mut sum_b) = (0.0, 0.0);
        let (mut best_t, mut best_var) = (127u8, -1.0);
        for (t, &c) in hist.iter().enumerate() {
            w_b += c as f64;
            if w_b == 0.0 {
                continue;
            }
            let w_f = total - w_b;
            if w_f == 0.0 {
                break;
            }
            sum_b += t as f64 * c as f64;
            let m_b = sum_b / w_b;
            let m_f = (sum_all - sum_b) / w_f;
            // Between-class variance – we maximise it.
            let var = w_b * w_f * (m_b - m_f).powi(2);
            if var > best_var {
                best_var = var;
                best_t = t as u8;
            }
        }
        best_t
    }
}

impl From<&image::GrayImage> for Gray {
    fn from(img: &image::GrayImage) -> Self {
        Self {
            width: img.width(),
            height: img.height(),
            data: img.as_raw().clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn otsu_separates_two_values() {
        let mut g = Gray::new(10, 10, 200);
        for x in 0..5 {
            for y in 0..10 {
                g.set(x, y, 40);
            }
        }
        let t = g.otsu_threshold();
        assert!((40..200).contains(&t));
    }

    #[test]
    fn bilinear_sampling() {
        let mut g = Gray::new(2, 1, 0);
        g.set(1, 0, 100);
        assert_eq!(g.sample(0.5, 0.5), 0.0);
        assert_eq!(g.sample(1.0, 0.5), 50.0);
    }
}
