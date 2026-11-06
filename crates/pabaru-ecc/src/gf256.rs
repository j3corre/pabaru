//! Arithmetic in the finite field GF(2^8) = GF(256).
//!
//! Every byte (0–255) is a field element. Both addition and subtraction are XOR.
//! Multiplication uses logarithm and exponent tables:
//! `a * b = exp[log[a] + log[b]]`.
//!
//! The primitive polynomial is 0x11d (x^8 + x^4 + x^3 + x^2 + 1),
//! the same as in QR codes. The generator (α) is 2.

/// Tables for fast multiplication. Computed once at compile time (`const fn`).
struct Tables {
    /// exp[i] = α^i. Length 512 so that multiplication needs no modulo 255.
    exp: [u8; 512],
    /// log[x] = i such that α^i = x. log[0] is meaningless (zero has no logarithm).
    log: [u8; 256],
}

const fn build_tables() -> Tables {
    let mut exp = [0u8; 512];
    let mut log = [0u8; 256];
    let mut x: u16 = 1;
    let mut i = 0;
    while i < 255 {
        exp[i] = x as u8;
        log[x as usize] = i as u8;
        // Multiply x by two (shift left) and if it overflows 8 bits,
        // reduce it by the primitive polynomial.
        x <<= 1;
        if x & 0x100 != 0 {
            x ^= 0x11d;
        }
        i += 1;
    }
    // Fill the second half of the table with a copy so exp[a + b] works even for a + b >= 255.
    while i < 512 {
        exp[i] = exp[i - 255];
        i += 1;
    }
    Tables { exp, log }
}

static TABLES: Tables = build_tables();

/// Product of two field elements.
#[inline]
pub fn mul(a: u8, b: u8) -> u8 {
    if a == 0 || b == 0 {
        return 0;
    }
    TABLES.exp[TABLES.log[a as usize] as usize + TABLES.log[b as usize] as usize]
}

/// Quotient `a / b`. Division by zero is a programming error, hence `panic`.
#[inline]
pub fn div(a: u8, b: u8) -> u8 {
    assert!(b != 0, "division by zero in GF(256)");
    if a == 0 {
        return 0;
    }
    let la = TABLES.log[a as usize] as usize;
    let lb = TABLES.log[b as usize] as usize;
    TABLES.exp[(la + 255 - lb) % 255]
}

/// Inverse element: `inv(a) * a == 1`.
#[inline]
pub fn inv(a: u8) -> u8 {
    div(1, a)
}

/// Power α^power (power may be negative).
#[inline]
pub fn alpha_pow(power: i32) -> u8 {
    TABLES.exp[power.rem_euclid(255) as usize]
}

// ---------------------------------------------------------------------------
// Polynomials over GF(256).
//
// CONVENTION: a polynomial is a slice of coefficients where index 0 is the HIGHEST power.
// E.g. [1, 2, 3] means x^2 + 2x + 3.
// ---------------------------------------------------------------------------

/// Evaluates the polynomial at `x` using Horner's scheme.
pub fn poly_eval(p: &[u8], x: u8) -> u8 {
    let mut y = 0u8;
    for &c in p {
        y = mul(y, x) ^ c;
    }
    y
}

/// Multiplies a polynomial by a scalar.
pub fn poly_scale(p: &[u8], s: u8) -> Vec<u8> {
    p.iter().map(|&c| mul(c, s)).collect()
}

/// Adds two polynomials (aligned by the lowest power, i.e. from the right).
pub fn poly_add(p: &[u8], q: &[u8]) -> Vec<u8> {
    let len = p.len().max(q.len());
    let mut r = vec![0u8; len];
    for (i, &c) in p.iter().enumerate() {
        r[i + len - p.len()] = c;
    }
    for (i, &c) in q.iter().enumerate() {
        r[i + len - q.len()] ^= c;
    }
    r
}

/// Multiplies two polynomials.
pub fn poly_mul(p: &[u8], q: &[u8]) -> Vec<u8> {
    let mut r = vec![0u8; p.len() + q.len() - 1];
    for (j, &qj) in q.iter().enumerate() {
        if qj == 0 {
            continue;
        }
        for (i, &pi) in p.iter().enumerate() {
            r[i + j] ^= mul(pi, qj);
        }
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mul_and_div_are_inverse() {
        for a in 1..=255u8 {
            for b in 1..=255u8 {
                assert_eq!(div(mul(a, b), b), a);
            }
        }
    }

    #[test]
    fn inverse() {
        for a in 1..=255u8 {
            assert_eq!(mul(a, inv(a)), 1);
        }
    }

    #[test]
    fn alpha_has_order_255() {
        assert_eq!(alpha_pow(0), 1);
        assert_eq!(alpha_pow(255), 1);
        assert_eq!(alpha_pow(-1), inv(2));
    }
}
