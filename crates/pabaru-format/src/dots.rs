//! Mapping of block bytes to dots in a 40×40 square.
//!
//! Rule (part of the v1 specification): bit number `d = byte * 8 + bit`
//! (bit 0 = the byte's most significant bit, MSB) lies at `x = d % 40`, `y = d / 40`,
//! i.e. row by row, left to right and top to bottom. Bit 1 = black dot.

use crate::{BLOCK_DOTS, BLOCK_LEN};

/// Returns whether dot `(x, y)` of the block is black.
#[inline]
pub fn dot_at(codeword: &[u8; BLOCK_LEN], x: usize, y: usize) -> bool {
    debug_assert!(x < BLOCK_DOTS && y < BLOCK_DOTS);
    let d = y * BLOCK_DOTS + x;
    (codeword[d / 8] >> (7 - d % 8)) & 1 == 1
}

/// Assembles a codeword from dots. The function `is_black(x, y)` tells
/// whether a dot is black (typically supplied by the scan decoder).
pub fn codeword_from_dots(mut is_black: impl FnMut(usize, usize) -> bool) -> [u8; BLOCK_LEN] {
    let mut cw = [0u8; BLOCK_LEN];
    for y in 0..BLOCK_DOTS {
        for x in 0..BLOCK_DOTS {
            if is_black(x, y) {
                let d = y * BLOCK_DOTS + x;
                cw[d / 8] |= 1 << (7 - d % 8);
            }
        }
    }
    cw
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut cw = [0u8; BLOCK_LEN];
        for (i, b) in cw.iter_mut().enumerate() {
            *b = (i * 37 + 11) as u8;
        }
        let back = codeword_from_dots(|x, y| dot_at(&cw, x, y));
        assert_eq!(back, cw);
    }

    #[test]
    fn first_dot_is_msb_of_first_byte() {
        let mut cw = [0u8; BLOCK_LEN];
        cw[0] = 0x80;
        assert!(dot_at(&cw, 0, 0));
        assert!(!dot_at(&cw, 1, 0));
    }
}
