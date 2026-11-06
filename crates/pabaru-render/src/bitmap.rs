//! A simple 1-bit bitmap.

/// Black-and-white image with 1 bit per pixel.
///
/// Rows are padded to whole bytes; the most significant bit of a byte is the
/// leftmost pixel. Bit value 1 = white, 0 = black (PDF DeviceGray and PNG convention).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bitmap {
    pub width: u32,
    pub height: u32,
    /// Number of bytes per row.
    pub stride: usize,
    pub data: Vec<u8>,
}

impl Bitmap {
    /// Creates an all-white bitmap.
    pub fn new_white(width: u32, height: u32) -> Self {
        let stride = (width as usize).div_ceil(8);
        Self {
            width,
            height,
            stride,
            data: vec![0xFF; stride * height as usize],
        }
    }

    /// Is the pixel black? Pixels outside the image are white.
    pub fn is_black(&self, x: u32, y: u32) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        let byte = self.data[y as usize * self.stride + x as usize / 8];
        (byte >> (7 - x % 8)) & 1 == 0
    }

    /// Sets the pixel to black.
    pub fn set_black(&mut self, x: u32, y: u32) {
        if x < self.width && y < self.height {
            self.data[y as usize * self.stride + x as usize / 8] &= !(1 << (7 - x % 8));
        }
    }

    /// Fills a rectangle with black (clipped to the image size).
    pub fn fill_rect(&mut self, x: u32, y: u32, w: u32, h: u32) {
        for yy in y..(y + h).min(self.height) {
            for xx in x..(x + w).min(self.width) {
                self.set_black(xx, yy);
            }
        }
    }

    /// A bitmap row as bytes.
    pub fn row(&self, y: u32) -> &[u8] {
        let s = y as usize * self.stride;
        &self.data[s..s + self.stride]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn draw_and_read() {
        let mut b = Bitmap::new_white(10, 3);
        assert_eq!(b.stride, 2);
        b.fill_rect(8, 1, 5, 1); // overflows the edge – gets clipped
        assert!(b.is_black(8, 1) && b.is_black(9, 1));
        assert!(!b.is_black(7, 1) && !b.is_black(8, 0));
        assert!(!b.is_black(100, 100));
    }
}
