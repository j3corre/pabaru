//! Inner code: Reed-Solomon over GF(256).
//!
//! Format v1 uses RS(200, 140): 140 data bytes + 60 parity bytes.
//! Such a code corrects up to 30 arbitrarily placed erroneous bytes (half the parity).
//!
//! Decoding has four steps:
//! 1. **Syndromes** – evaluate the received word at the generator's roots.
//!    If all are zero, the word is error-free.
//! 2. **Berlekamp-Massey** – find the *error locator* Λ(x) from the syndromes.
//! 3. **Chien search** – try all positions; roots of Λ(x) point at the errors.
//! 4. **Forney algorithm** – compute the error magnitude at each position and correct it.

use crate::EccError;
use crate::gf256::{alpha_pow, div, inv, mul, poly_add, poly_eval, poly_mul, poly_scale};

/// Number of data bytes in one format v1 block.
pub const DATA_LEN: usize = 140;
/// Number of parity bytes in one format v1 block.
pub const PARITY_LEN: usize = 60;
/// Total length of a format v1 codeword (block).
pub const CODEWORD_LEN: usize = DATA_LEN + PARITY_LEN;

/// Reed-Solomon encoder/decoder with a fixed number of parity bytes.
#[derive(Debug, Clone)]
pub struct ReedSolomon {
    /// Number of parity bytes (nsym).
    nsym: usize,
    /// Generator polynomial g(x) = (x - α^0)(x - α^1)…(x - α^(nsym-1)).
    generator: Vec<u8>,
}

impl ReedSolomon {
    /// Creates an encoder with `nsym` parity bytes.
    pub fn new(nsym: usize) -> Self {
        assert!(nsym > 0 && nsym < 255, "nsym must be in range 1..255");
        let mut generator = vec![1u8];
        for i in 0..nsym {
            // In GF(2^n) subtraction equals addition, hence (x + α^i).
            generator = poly_mul(&generator, &[1, alpha_pow(i as i32)]);
        }
        Self { nsym, generator }
    }

    /// Encoder for format v1: RS(200, 140).
    pub fn v1() -> Self {
        Self::new(PARITY_LEN)
    }

    /// Number of parity bytes.
    pub fn parity_len(&self) -> usize {
        self.nsym
    }

    /// Encodes a message: returns `message || parity` (systematic code).
    pub fn encode(&self, msg: &[u8]) -> Vec<u8> {
        assert!(
            msg.len() + self.nsym <= 255,
            "codeword must not be longer than 255 bytes"
        );
        // Polynomial division of the message (shifted by nsym) by the generator.
        // The remainder is the parity.
        let mut out = vec![0u8; msg.len() + self.nsym];
        out[..msg.len()].copy_from_slice(msg);
        for i in 0..msg.len() {
            let coef = out[i];
            if coef != 0 {
                for (j, &g) in self.generator.iter().enumerate().skip(1) {
                    out[i + j] ^= mul(g, coef);
                }
            }
        }
        // The division overwrote the beginning – put the original message back.
        out[..msg.len()].copy_from_slice(msg);
        out
    }

    /// Computes the syndromes. Returns a vector of length nsym + 1 with a leading zero
    /// (the leading zero simplifies indexing in later steps).
    fn syndromes(&self, word: &[u8]) -> Vec<u8> {
        let mut s = vec![0u8; self.nsym + 1];
        for i in 0..self.nsym {
            s[i + 1] = poly_eval(word, alpha_pow(i as i32));
        }
        s
    }

    /// Corrects the codeword in place. Returns the number of corrected bytes.
    ///
    /// On failure returns [`EccError::TooManyErrors`] and leaves the word unchanged.
    pub fn correct(&self, word: &mut [u8]) -> Result<usize, EccError> {
        if word.len() <= self.nsym || word.len() > 255 {
            return Err(EccError::InvalidLength {
                expected: self.nsym + 1,
                got: word.len(),
            });
        }
        let synd = self.syndromes(word);
        if synd.iter().all(|&s| s == 0) {
            return Ok(0); // The word is fine.
        }

        // Step 2: error locator.
        let err_loc = self.find_error_locator(&synd)?;
        // Step 3: error positions.
        let mut rev = err_loc.clone();
        rev.reverse();
        let err_pos = find_errors(&rev, word.len())?;
        // Step 4: correction.
        let mut fixed = word.to_vec();
        correct_errata(&mut fixed, &synd, &err_pos);

        // Check: after correction all syndromes must be zero.
        // Otherwise we "corrected" to a different valid word or failed.
        if self.syndromes(&fixed).iter().any(|&s| s != 0) {
            return Err(EccError::TooManyErrors);
        }
        word.copy_from_slice(&fixed);
        Ok(err_pos.len())
    }

    /// Correction with **erasures**: `erase_pos` are positions of bytes known
    /// to be unreliable (e.g. ambiguously read dots). The code can then
    /// correct more: `2 · errors + erasures ≤ nsym` (without erasures only
    /// `2 · errors ≤ nsym`). Returns the number of corrected bytes.
    ///
    /// Procedure: "Forney syndromes" remove the effect of known erasures,
    /// Berlekamp-Massey then finds only the remaining unknown errors and the
    /// Forney algorithm computes values for erasures and errors at once.
    pub fn correct_with_erasures(
        &self,
        word: &mut [u8],
        erase_pos: &[usize],
    ) -> Result<usize, EccError> {
        let n = word.len();
        if n <= self.nsym || n > 255 {
            return Err(EccError::InvalidLength {
                expected: self.nsym + 1,
                got: n,
            });
        }
        if erase_pos.len() > self.nsym || erase_pos.iter().any(|&p| p >= n) {
            return Err(EccError::TooManyErrors);
        }
        let synd = self.syndromes(word);
        if synd.iter().all(|&s| s == 0) {
            return Ok(0);
        }
        // Forney syndromes (without the leading zero).
        let mut fsynd: Vec<u8> = synd[1..].to_vec();
        for &p in erase_pos {
            let x = alpha_pow((n - 1 - p) as i32);
            for j in 0..fsynd.len() - 1 {
                fsynd[j] = mul(fsynd[j], x) ^ fsynd[j + 1];
            }
        }
        // Berlekamp-Massey for unknown errors only (fewer iterations).
        let iterations = self.nsym - erase_pos.len();
        let mut err_loc = vec![1u8];
        let mut old_loc = vec![1u8];
        for i in 0..iterations {
            let mut delta = fsynd[i];
            for j in 1..err_loc.len() {
                if i >= j {
                    delta ^= mul(err_loc[err_loc.len() - 1 - j], fsynd[i - j]);
                }
            }
            old_loc.push(0);
            if delta != 0 {
                if old_loc.len() > err_loc.len() {
                    let new_loc = poly_scale(&old_loc, delta);
                    old_loc = poly_scale(&err_loc, inv(delta));
                    err_loc = new_loc;
                }
                err_loc = poly_add(&err_loc, &poly_scale(&old_loc, delta));
            }
        }
        while err_loc.len() > 1 && err_loc[0] == 0 {
            err_loc.remove(0);
        }
        let errs = err_loc.len() - 1;
        if errs * 2 + erase_pos.len() > self.nsym {
            return Err(EccError::TooManyErrors);
        }
        let mut rev = err_loc.clone();
        rev.reverse();
        let err_pos = find_errors(&rev, n)?;
        // Erasures + found errors (deduplicated).
        let mut all: Vec<usize> = erase_pos.to_vec();
        for p in err_pos {
            if !all.contains(&p) {
                all.push(p);
            }
        }
        let mut fixed = word.to_vec();
        correct_errata(&mut fixed, &synd, &all);
        if self.syndromes(&fixed).iter().any(|&s| s != 0) {
            return Err(EccError::TooManyErrors);
        }
        let changed = fixed
            .iter()
            .zip(word.iter())
            .filter(|(a, b)| a != b)
            .count();
        word.copy_from_slice(&fixed);
        Ok(changed)
    }

    /// Berlekamp-Massey: finds the error locator polynomial Λ(x).
    fn find_error_locator(&self, synd: &[u8]) -> Result<Vec<u8>, EccError> {
        let mut err_loc = vec![1u8];
        let mut old_loc = vec![1u8];
        // Syndromes have a leading zero prepended, hence the shift by 1.
        let shift = synd.len() - self.nsym;
        for i in 0..self.nsym {
            let k = i + shift;
            // Discrepancy: how much the current locator "misses" the next syndrome.
            let mut delta = synd[k];
            for j in 1..err_loc.len() {
                delta ^= mul(err_loc[err_loc.len() - 1 - j], synd[k - j]);
            }
            old_loc.push(0); // Multiply by x.
            if delta != 0 {
                if old_loc.len() > err_loc.len() {
                    let new_loc = poly_scale(&old_loc, delta);
                    old_loc = poly_scale(&err_loc, inv(delta));
                    err_loc = new_loc;
                }
                err_loc = poly_add(&err_loc, &poly_scale(&old_loc, delta));
            }
        }
        // Strip leading zeros.
        while err_loc.len() > 1 && err_loc[0] == 0 {
            err_loc.remove(0);
        }
        let errs = err_loc.len() - 1;
        if errs * 2 > self.nsym {
            return Err(EccError::TooManyErrors);
        }
        Ok(err_loc)
    }
}

/// Chien search: tries all positions and returns those with an error.
fn find_errors(err_loc_rev: &[u8], nmess: usize) -> Result<Vec<usize>, EccError> {
    let errs = err_loc_rev.len() - 1;
    let mut pos = Vec::with_capacity(errs);
    for i in 0..nmess {
        if poly_eval(err_loc_rev, alpha_pow(i as i32)) == 0 {
            pos.push(nmess - 1 - i);
        }
    }
    // The number of roots found must match the locator's degree.
    if pos.len() != errs {
        return Err(EccError::TooManyErrors);
    }
    Ok(pos)
}

/// Forney algorithm: computes the error magnitudes and corrects them.
fn correct_errata(word: &mut [u8], synd: &[u8], err_pos: &[usize]) {
    let n = word.len();
    // Convert positions to powers of x (coefficients counted from the end).
    let coef_pos: Vec<usize> = err_pos.iter().map(|&p| n - 1 - p).collect();

    // Locator from known positions: Π (1 - x·α^i).
    let mut e_loc = vec![1u8];
    for &i in &coef_pos {
        e_loc = poly_mul(&e_loc, &poly_add(&[1], &[alpha_pow(i as i32), 0]));
    }

    // Error evaluator Ω(x) = [S(x) · Λ(x)] mod x^(nsym+1).
    let mut synd_rev = synd.to_vec();
    synd_rev.reverse();
    let product = poly_mul(&synd_rev, &e_loc);
    let keep = e_loc.len(); // (number of errors + 1)
    let mut err_eval = product[product.len() - keep..].to_vec();
    err_eval.reverse();

    // X_i = α^(position) – "locators" of the individual errors.
    let xs: Vec<u8> = coef_pos.iter().map(|&p| alpha_pow(p as i32)).collect();

    for (i, &xi) in xs.iter().enumerate() {
        let xi_inv = inv(xi);
        // Locator derivative at X_i^-1 (formal derivative = product of the other terms).
        let mut err_loc_prime = 1u8;
        for (j, &xj) in xs.iter().enumerate() {
            if j != i {
                err_loc_prime = mul(err_loc_prime, 1 ^ mul(xi_inv, xj));
            }
        }
        // Ω(X_i^-1), the evaluator is stored reversed.
        let mut ev = err_eval.clone();
        ev.reverse();
        let y = mul(xi, poly_eval(&ev, xi_inv)); // fcr = 0 => multiply by X_i^1
        let magnitude = div(y, err_loc_prime);
        word[err_pos[i]] ^= magnitude;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn no_errors_corrects_nothing() {
        let rs = ReedSolomon::v1();
        let msg: Vec<u8> = (0..DATA_LEN as u32).map(|i| (i * 7) as u8).collect();
        let mut cw = rs.encode(&msg);
        assert_eq!(cw.len(), CODEWORD_LEN);
        assert_eq!(rs.correct(&mut cw), Ok(0));
        assert_eq!(&cw[..DATA_LEN], &msg[..]);
    }

    #[test]
    fn corrects_maximum_number_of_errors() {
        let rs = ReedSolomon::v1();
        let msg: Vec<u8> = (0..DATA_LEN).map(|i| i as u8).collect();
        let clean = rs.encode(&msg);
        let mut cw = clean.clone();
        for i in 0..30 {
            cw[i * 6] ^= 0xA5;
        }
        assert_eq!(rs.correct(&mut cw), Ok(30));
        assert_eq!(cw, clean);
    }

    #[test]
    fn too_many_errors_returns_error() {
        let rs = ReedSolomon::v1();
        let msg = vec![42u8; DATA_LEN];
        let mut cw = rs.encode(&msg);
        for b in cw.iter_mut().take(40) {
            *b ^= 0xFF;
        }
        let before = cw.clone();
        assert!(rs.correct(&mut cw).is_err());
        assert_eq!(cw, before, "on failure the word must not change");
    }

    #[test]
    fn erasures_double_correction() {
        // 50 corrupted bytes: uncorrectable without erasures, correctable as erasures.
        let rs = ReedSolomon::v1();
        let msg: Vec<u8> = (0..DATA_LEN).map(|i| (i * 3) as u8).collect();
        let clean = rs.encode(&msg);
        let mut cw = clean.clone();
        let pos: Vec<usize> = (0..50).map(|i| i * 4).collect();
        for &p in &pos {
            cw[p] ^= 0x5A;
        }
        assert!(rs.clone().correct(&mut cw.clone()).is_err());
        assert_eq!(rs.correct_with_erasures(&mut cw, &pos), Ok(50));
        assert_eq!(cw, clean);
    }

    proptest! {
        // Combination of unknown errors and erasures: 2·errors + erasures ≤ 60.
        #[test]
        fn errors_and_erasures(
            msg in proptest::collection::vec(any::<u8>(), DATA_LEN),
            erasures in 0usize..=60,
            seed in any::<u64>(),
        ) {
            let rs = ReedSolomon::v1();
            let clean = rs.encode(&msg);
            let errors = (PARITY_LEN - erasures) / 2;
            // Random distinct positions derived from the seed.
            let mut s = seed | 1;
            let mut used = std::collections::BTreeSet::new();
            while used.len() < erasures + errors {
                s ^= s << 13; s ^= s >> 7; s ^= s << 17;
                used.insert((s % CODEWORD_LEN as u64) as usize);
            }
            let used: Vec<usize> = used.into_iter().collect();
            let erase_pos = &used[..erasures];
            let mut cw = clean.clone();
            for &p in &used {
                cw[p] ^= 0xA7;
            }
            prop_assert!(rs.correct_with_erasures(&mut cw, erase_pos).is_ok());
            prop_assert_eq!(cw, clean);
        }
    }

    proptest! {
        // Random message and up to 30 random errors at random positions.
        #[test]
        fn random_errors_are_corrected(
            msg in proptest::collection::vec(any::<u8>(), DATA_LEN),
            errors in proptest::collection::btree_map(0usize..CODEWORD_LEN, 1u8..=255, 0..=30),
        ) {
            let rs = ReedSolomon::v1();
            let clean = rs.encode(&msg);
            let mut cw = clean.clone();
            for (&pos, &val) in &errors {
                cw[pos] ^= val;
            }
            prop_assert_eq!(rs.correct(&mut cw), Ok(errors.len()));
            prop_assert_eq!(cw, clean);
        }

        // Shorter code with a different parity count (generality of the implementation).
        #[test]
        fn general_code(
            msg in proptest::collection::vec(any::<u8>(), 1..100),
            nsym in 2usize..40,
            seed in any::<u64>(),
        ) {
            let rs = ReedSolomon::new(nsym);
            let clean = rs.encode(&msg);
            let mut cw = clean.clone();
            let n = cw.len();
            let count = (nsym / 2).min(n);
            // Errors placed deterministically from the seed.
            let mut s = seed;
            let mut used = std::collections::BTreeSet::new();
            while used.len() < count {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
                used.insert((s >> 33) as usize % n);
            }
            for &p in &used { cw[p] ^= 0x5A; }
            prop_assert!(rs.correct(&mut cw).is_ok());
            prop_assert_eq!(cw, clean);
        }
    }
}
