//! Data whitening.
//!
//! Compressed or encrypted data looks random, but headers, zero padding etc.
//! do not. Long runs of identical bits would create large uniform areas on
//! paper and the decoder would lose track of the dot grid.
//!
//! Therefore every block is XORed with a fixed pseudorandom sequence. XOR is
//! its own inverse: calling [`whiten`] a second time restores the original data.

use std::sync::OnceLock;

use crate::BLOCK_LEN;

/// Generates the fixed sequence using a simple LFSR (xorshift32).
fn sequence() -> &'static [u8; BLOCK_LEN] {
    static SEQ: OnceLock<[u8; BLOCK_LEN]> = OnceLock::new();
    SEQ.get_or_init(|| {
        let mut out = [0u8; BLOCK_LEN];
        // Fixed seed – part of the format v1 specification, MUST NOT change.
        let mut state: u32 = 0x5041_4252; // "PABR"
        for b in out.iter_mut() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *b = (state >> 24) as u8;
        }
        out
    })
}

/// XORs the data with the whitening sequence (in place). Works for data
/// of length at most [`BLOCK_LEN`].
pub fn whiten(data: &mut [u8]) {
    for (d, s) in data.iter_mut().zip(sequence().iter()) {
        *d ^= s;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_whitening_restores_original_data() {
        let orig = [0u8; BLOCK_LEN];
        let mut d = orig;
        whiten(&mut d);
        assert_ne!(d, orig);
        whiten(&mut d);
        assert_eq!(d, orig);
    }

    #[test]
    fn sequence_is_balanced() {
        // Roughly half of the bits should be ones.
        let ones: u32 = sequence().iter().map(|b| b.count_ones()).sum();
        let total = (BLOCK_LEN * 8) as u32;
        assert!(ones > total * 2 / 5 && ones < total * 3 / 5, "ones: {ones}");
    }
}
