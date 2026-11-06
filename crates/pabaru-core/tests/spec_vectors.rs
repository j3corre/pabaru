//! Test vectors for `docs/SPEC.md`.
//!
//! Every concrete value printed in the specification is computed and asserted
//! here, so the document cannot silently drift away from the code. Most
//! sections also contain an *independent reference implementation*
//! transcribed from the pseudocode in SPEC.md (module [`reference`]) and
//! compare it with the production code / the `reed-solomon-simd` library.
//!
//! * `PABARU_SPEC_PRINT=1` prints all vectors in the format used by SPEC.md.
//! * `PABARU_SPEC_WRITE=1` additionally regenerates `docs/img/spec/sample-page.png`.

#![allow(
    clippy::needless_range_loop,
    clippy::manual_memcpy,
    clippy::too_many_arguments,
    clippy::type_complexity
)]

use std::collections::BTreeMap;

use pabaru_format::{BLOCK_LEN, Block, BlockHeader, PAYLOAD_LEN, Superblock, dots, whiten};

// ===========================================================================
// Helpers
// ===========================================================================

/// Hex dump, 16 bytes per line, lowercase, bytes separated by spaces.
fn hex16(b: &[u8]) -> String {
    b.chunks(16)
        .map(|c| {
            c.iter()
                .map(|x| format!("{x:02x}"))
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Parses a hex string (any whitespace is ignored).
fn unhex(s: &str) -> Vec<u8> {
    let digits: Vec<u8> = s.bytes().filter(|c| !c.is_ascii_whitespace()).collect();
    assert!(digits.len().is_multiple_of(2), "odd number of hex digits");
    digits
        .chunks(2)
        .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
        .collect()
}

/// The specification itself: every vector must appear in it verbatim.
const SPEC: &str = include_str!("../../../docs/SPEC.md");

fn printing() -> bool {
    std::env::var_os("PABARU_SPEC_PRINT").is_some()
}

/// Asserts that `actual` equals the hex string `expected` (as printed in SPEC.md).
fn assert_hex(name: &str, actual: &[u8], expected: &str) {
    if printing() {
        println!("=== {name} ({} bytes)\n{}\n", actual.len(), hex16(actual));
    }
    assert!(
        SPEC.contains(&hex16(actual)),
        "docs/SPEC.md does not contain vector `{name}`"
    );
    assert_eq!(
        hex16(actual),
        hex16(&unhex(expected)),
        "vector `{name}` changed"
    );
}

fn assert_text(name: &str, actual: &str, expected: &str) {
    if printing() {
        println!("=== {name}\n{actual}\n");
    }
    assert_eq!(actual, expected, "vector `{name}` changed");
    assert!(
        SPEC.contains(actual),
        "docs/SPEC.md does not contain vector `{name}`"
    );
}

/// Deterministic pseudo-random bytes for the self-checks (xorshift64*).
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
    /// `count` distinct values from `0..n`.
    fn distinct(&mut self, n: usize, count: usize) -> Vec<usize> {
        let mut all: Vec<usize> = (0..n).collect();
        for i in 0..count {
            let j = i + self.below(n - i);
            all.swap(i, j);
        }
        all.truncate(count);
        all
    }
}

// ===========================================================================
// Reference implementation transcribed from docs/SPEC.md
// ===========================================================================

mod reference {
    // ---------------------------------------------------------------- §5.4
    /// Whitening sequence (xorshift32, seed 0x50414252).
    pub fn whitening_sequence() -> [u8; 200] {
        let mut out = [0u8; 200];
        let mut state: u32 = 0x5041_4252;
        for b in out.iter_mut() {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            *b = (state >> 24) as u8;
        }
        out
    }

    // ---------------------------------------------------------------- §5.2
    /// CRC-32C (Castagnoli), reflected, poly 0x82F63B78, init/xorout 0xFFFFFFFF.
    pub fn crc32c(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0x82F6_3B78
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    // ---------------------------------------------------------------- §5.3
    /// GF(2^8) with polynomial 0x11D, α = 2.
    pub struct Gf256 {
        pub exp: [u8; 255],
        pub log: [u8; 256],
    }

    impl Gf256 {
        pub fn new() -> Self {
            let mut exp = [0u8; 255];
            let mut log = [0u8; 256];
            let mut x: u16 = 1;
            for i in 0..255 {
                exp[i] = x as u8;
                log[x as usize] = i as u8;
                x <<= 1;
                if x & 0x100 != 0 {
                    x ^= 0x11D;
                }
            }
            Self { exp, log }
        }
        pub fn mul(&self, a: u8, b: u8) -> u8 {
            if a == 0 || b == 0 {
                0
            } else {
                self.exp[(self.log[a as usize] as usize + self.log[b as usize] as usize) % 255]
            }
        }
        pub fn inv(&self, a: u8) -> u8 {
            assert!(a != 0);
            self.exp[(255 - self.log[a as usize] as usize) % 255]
        }
        pub fn pow_alpha(&self, e: usize) -> u8 {
            self.exp[e % 255]
        }
    }

    /// g(x) = Π_{i=0}^{59} (x + α^i); coefficients from the HIGHEST power (g[0] = 1).
    pub fn rs_generator(gf: &Gf256) -> Vec<u8> {
        let mut g = vec![1u8];
        for i in 0..60 {
            let a = gf.pow_alpha(i);
            let mut next = vec![0u8; g.len() + 1];
            for (j, &c) in g.iter().enumerate() {
                next[j] ^= c; // · x
                next[j + 1] ^= gf.mul(c, a); // · α^i
            }
            g = next;
        }
        g
    }

    /// Systematic RS(200,140) encoding: message ‖ (message·x^60 mod g).
    pub fn rs_encode(gf: &Gf256, msg: &[u8; 140]) -> [u8; 200] {
        let g = rs_generator(gf);
        let mut rem = [0u8; 60];
        for &m in msg {
            let factor = m ^ rem[0];
            rem.copy_within(1.., 0);
            rem[59] = 0;
            for j in 0..60 {
                rem[j] ^= gf.mul(g[j + 1], factor);
            }
        }
        let mut out = [0u8; 200];
        out[..140].copy_from_slice(msg);
        out[140..].copy_from_slice(&rem);
        out
    }

    /// Syndromes S_j = r(α^j), j = 0..59; byte p is the coefficient of x^(199-p).
    pub fn rs_syndromes(gf: &Gf256, r: &[u8; 200]) -> [u8; 60] {
        let mut s = [0u8; 60];
        for j in 0..60 {
            let a = gf.pow_alpha(j);
            let mut acc = 0u8;
            for &b in r.iter() {
                acc = gf.mul(acc, a) ^ b; // Horner from the highest power
            }
            s[j] = acc;
        }
        s
    }

    fn poly_eval_low(gf: &Gf256, p: &[u8], x: u8) -> u8 {
        // p[i] = coefficient of x^i
        let mut acc = 0u8;
        for &c in p.iter().rev() {
            acc = gf.mul(acc, x) ^ c;
        }
        acc
    }

    /// Errors-and-erasures decoder (§5.3.3). Returns the corrected word or None.
    pub fn rs_decode(gf: &Gf256, r: &[u8; 200], erasures: &[usize]) -> Option<[u8; 200]> {
        let s = rs_syndromes(gf, r);
        if s.iter().all(|&v| v == 0) {
            return Some(*r);
        }
        let rho = erasures.len();
        if rho > 60 {
            return None;
        }
        let loc = |p: usize| gf.pow_alpha(199 - p); // X_p
        // Γ(x) = Π (1 + X_p x), low-first coefficients.
        let mut gamma = vec![1u8];
        for &p in erasures {
            let xp = loc(p);
            let mut next = vec![0u8; gamma.len() + 1];
            for (i, &c) in gamma.iter().enumerate() {
                next[i] ^= c;
                next[i + 1] ^= gf.mul(c, xp);
            }
            gamma = next;
        }
        // Berlekamp–Massey initialised with the erasure locator.
        let mut lambda = gamma.clone();
        let mut b = gamma.clone();
        let mut l = rho;
        for k in (rho + 1)..=60 {
            let mut delta = 0u8;
            for j in 0..=l.min(lambda.len() - 1) {
                if k > j {
                    delta ^= gf.mul(lambda[j], s[k - 1 - j]);
                }
            }
            // xB
            let mut xb = vec![0u8];
            xb.extend_from_slice(&b);
            if delta == 0 {
                b = xb;
                continue;
            }
            let mut t = lambda.clone();
            t.resize(t.len().max(xb.len()), 0);
            for (i, &c) in xb.iter().enumerate() {
                t[i] ^= gf.mul(delta, c);
            }
            if 2 * l < k + rho {
                // 2L ≤ k + ρ − 1
                l = k + rho - l;
                let dinv = gf.inv(delta);
                b = lambda.iter().map(|&c| gf.mul(c, dinv)).collect();
            } else {
                b = xb;
            }
            lambda = t;
        }
        while lambda.len() > 1 && *lambda.last().unwrap() == 0 {
            lambda.pop();
        }
        let deg = lambda.len() - 1;
        if deg != l || 2 * (l - rho) + rho > 60 {
            return None;
        }
        // Chien search over the 200 valid positions.
        let positions: Vec<usize> = (0..200)
            .filter(|&p| poly_eval_low(gf, &lambda, gf.inv(loc(p))) == 0)
            .collect();
        if positions.len() != deg {
            return None;
        }
        // Ω(x) = S(x)·Λ(x) mod x^60.
        let mut omega = [0u8; 60];
        for i in 0..60 {
            for (j, &c) in lambda.iter().enumerate() {
                if i + j < 60 {
                    omega[i + j] ^= gf.mul(s[i], c);
                }
            }
        }
        // Λ'(x): formal derivative (odd terms only in characteristic 2).
        let dlambda: Vec<u8> = (1..lambda.len())
            .map(|i| if i % 2 == 1 { lambda[i] } else { 0 })
            .collect();
        let mut out = *r;
        for &p in &positions {
            let xp = loc(p);
            let xinv = gf.inv(xp);
            let num = gf.mul(xp, poly_eval_low(gf, &omega, xinv));
            let den = poly_eval_low(gf, &dlambda, xinv);
            if den == 0 {
                return None;
            }
            out[p] ^= gf.mul(num, gf.inv(den));
        }
        if rs_syndromes(gf, &out).iter().any(|&v| v != 0) {
            return None;
        }
        Some(out)
    }

    // ---------------------------------------------------------------- §4.2
    pub const GF_POLYNOMIAL: u32 = 0x1002D;
    pub const CANTOR_BASIS: [u16; 16] = [
        0x0001, 0xACCA, 0x3C0E, 0x163E, 0xC582, 0xED2E, 0x914C, 0x4012, 0x6C98, 0x10D8, 0x6A72,
        0xB900, 0xFDB8, 0xFB34, 0xFF38, 0x991E,
    ];

    /// GF(2^16) in the Cantor-basis representation (§4.2).
    pub struct Gf16 {
        pub log: Vec<u16>,  // 65536 entries, log[0] = 65535 (sentinel)
        pub exp: Vec<u16>,  // 65536 entries, exp[65535] = exp[0] = 1
        pub skew: Vec<u16>, // 65535 entries, field ELEMENTS (not logs)
    }

    impl Gf16 {
        pub fn new() -> Self {
            // Step 1: discrete logarithm in the polynomial basis.
            let mut lfsr_log = vec![0u16; 65536];
            let mut state: u32 = 1;
            for i in 0..65535u32 {
                lfsr_log[state as usize] = i as u16;
                state <<= 1;
                if state >= 65536 {
                    state ^= GF_POLYNOMIAL;
                }
            }
            lfsr_log[0] = 65535;
            // Step 2: Cantor-basis value -> polynomial-basis value.
            let mut poly = vec![0u16; 65536];
            for i in 0..16 {
                let w = 1usize << i;
                for j in 0..w {
                    poly[j + w] = poly[j] ^ CANTOR_BASIS[i];
                }
            }
            // Step 3/4.
            let mut log = vec![0u16; 65536];
            let mut exp = vec![0u16; 65536];
            for x in 0..65536 {
                log[x] = lfsr_log[poly[x] as usize];
            }
            for x in 0..65536 {
                exp[log[x] as usize] = x as u16;
            }
            exp[65535] = exp[0];
            let mut gf = Self {
                log,
                exp,
                skew: Vec::new(),
            };
            gf.skew = gf.build_skew();
            gf
        }

        pub fn mul(&self, a: u16, b: u16) -> u16 {
            if a == 0 || b == 0 {
                return 0;
            }
            let s = (self.log[a as usize] as u32 + self.log[b as usize] as u32) % 65535;
            self.exp[s as usize]
        }

        pub fn inv(&self, a: u16) -> u16 {
            assert!(a != 0);
            let l = self.log[a as usize] as u32;
            self.exp[((65535 - l) % 65535) as usize]
        }

        /// Skew factors (§4.2.3), as field elements.
        fn build_skew(&self) -> Vec<u16> {
            let mut skew = vec![0u16; 65535];
            let mut temp = [0u16; 15];
            for i in 1..16 {
                temp[i - 1] = 1 << i;
            }
            for m in 0..15 {
                let step = 1usize << (m + 1);
                skew[(1 << m) - 1] = 0;
                for i in m..15 {
                    let s = 1usize << (i + 1);
                    let mut j = (1usize << m) - 1;
                    while j < s {
                        skew[j + s] = skew[j] ^ temp[i];
                        j += step;
                    }
                }
                let t = temp[m];
                let d = self.mul(t, t ^ 1);
                let dinv = self.inv(d);
                for i in m + 1..15 {
                    temp[i] = self.mul(self.mul(temp[i], temp[i] ^ 1), dinv);
                }
            }
            skew
        }

        // ------------------------------------------------------------ §4.3
        /// In-place FFT over `data` (length = power of two), skew offset `delta`.
        pub fn fft(&self, data: &mut [u16], delta: usize) {
            let size = data.len();
            let mut dist = size / 2;
            while dist > 0 {
                let mut r = 0;
                while r < size {
                    let s = self.skew[r + dist + delta - 1];
                    for i in r..r + dist {
                        data[i] ^= self.mul(s, data[i + dist]);
                        data[i + dist] ^= data[i];
                    }
                    r += 2 * dist;
                }
                dist /= 2;
            }
        }

        /// In-place inverse FFT.
        pub fn ifft(&self, data: &mut [u16], delta: usize) {
            let size = data.len();
            let mut dist = 1;
            while dist < size {
                let mut r = 0;
                while r < size {
                    let s = self.skew[r + dist + delta - 1];
                    for i in r..r + dist {
                        data[i + dist] ^= data[i];
                        data[i] ^= self.mul(s, data[i + dist]);
                    }
                    r += 2 * dist;
                }
                dist *= 2;
            }
        }
    }

    pub fn formal_derivative(data: &mut [u16]) {
        for i in 1..data.len() {
            let width = 1usize << i.trailing_zeros();
            for j in 0..width {
                data[i - width + j] ^= data[i + j];
            }
        }
    }

    // ---------------------------------------------------------------- §4.4
    /// true = high rate, false = low rate; None = unsupported (k, m).
    pub fn use_high_rate(k: usize, m: usize) -> Option<bool> {
        if k == 0 || m == 0 || k > 65536 || m > 65536 {
            return None;
        }
        let (kp, mp) = (k.next_power_of_two(), m.next_power_of_two());
        if kp.min(mp) + k.max(m) > 65536 {
            return None;
        }
        Some(if kp != mp { kp > mp } else { k <= m })
    }

    /// Encodes one symbol column: `data[i]` = symbol of data shard i.
    pub fn encode_high(gf: &Gf16, data: &[u16], m: usize) -> Vec<u16> {
        let k = data.len();
        let t = m.next_power_of_two();
        let mut acc = vec![0u16; t];
        let mut c = 0;
        while c * t < k {
            let mut buf = vec![0u16; t];
            for i in 0..t {
                if c * t + i < k {
                    buf[i] = data[c * t + i];
                }
            }
            gf.ifft(&mut buf, c * t + t);
            for i in 0..t {
                acc[i] ^= buf[i];
            }
            c += 1;
        }
        gf.fft(&mut acc, 0);
        acc.truncate(m);
        acc
    }

    pub fn encode_low(gf: &Gf16, data: &[u16], m: usize) -> Vec<u16> {
        let k = data.len();
        let t = k.next_power_of_two();
        let mut buf = vec![0u16; t];
        buf[..k].copy_from_slice(data);
        gf.ifft(&mut buf, 0);
        let mut parity = Vec::with_capacity(m);
        let mut c = 0;
        while c * t < m {
            let mut out = buf.clone();
            gf.fft(&mut out, c * t + t);
            for i in 0..t {
                if c * t + i < m {
                    parity.push(out[i]);
                }
            }
            c += 1;
        }
        parity
    }

    /// Error-locator values loc[i] = Π_{j ∈ E, j ≠ i} (i XOR j) for i < n (direct definition).
    pub fn locator_naive(gf: &Gf16, erased: &[bool], n: usize) -> Vec<u16> {
        let e: Vec<usize> = (0..65536).filter(|&j| erased[j]).collect();
        (0..n)
            .map(|i| {
                let mut p = 1u16;
                for &j in &e {
                    if j != i {
                        p = gf.mul(p, (i ^ j) as u16);
                    }
                }
                p
            })
            .collect()
    }

    /// Same values via the fast Walsh–Hadamard transform (mod 65535), returned as logs.
    pub fn locator_log_fwht(gf: &Gf16, erased: &[bool]) -> Vec<u16> {
        fn fwht(v: &mut [u32]) {
            let mut h = 1;
            while h < v.len() {
                for i in (0..v.len()).step_by(2 * h) {
                    for j in i..i + h {
                        let (a, b) = (v[j], v[j + h]);
                        v[j] = (a + b) % 65535;
                        v[j + h] = (a + 65535 - b) % 65535;
                    }
                }
                h *= 2;
            }
        }
        let mut lw: Vec<u32> = gf.log.iter().map(|&l| l as u32 % 65535).collect();
        lw[0] = 0;
        fwht(&mut lw);
        let mut e: Vec<u32> = erased.iter().map(|&b| b as u32).collect();
        fwht(&mut e);
        for i in 0..65536 {
            e[i] = (e[i] as u64 * lw[i] as u64 % 65535) as u32;
        }
        fwht(&mut e);
        e.iter().map(|&v| v as u16).collect()
    }

    /// Recovers the missing data symbols of one column.
    /// `orig[i]` / `rec[i]` are None when shard i is missing.
    /// How [`decode`] computes the error-locator values (all three must agree).
    #[derive(Clone, Copy, Debug)]
    pub enum Loc {
        /// Direct definition over the whole erased set E.
        Naive,
        /// Fast Walsh–Hadamard transform (what the library does).
        Fwht,
        /// Direct definition over E ∩ [0, W) only (optimisation note in §4.5).
        Truncated,
    }

    /// loc[i] = Π_{j ∈ E, j < w, j ≠ i} (i XOR j) for i < w.
    pub fn locator_truncated(gf: &Gf16, erased: &[bool], w: usize) -> Vec<u16> {
        let e: Vec<usize> = (0..w).filter(|&j| erased[j]).collect();
        (0..w)
            .map(|i| {
                e.iter()
                    .filter(|&&j| j != i)
                    .fold(1u16, |p, &j| gf.mul(p, (i ^ j) as u16))
            })
            .collect()
    }

    pub fn decode(gf: &Gf16, orig: &[Option<u16>], rec: &[Option<u16>], mode: Loc) -> Vec<u16> {
        let (k, m) = (orig.len(), rec.len());
        let high = use_high_rate(k, m).expect("unsupported");
        let mut erased = vec![false; 65536];
        let (t, n_end, orig_pos): (usize, usize, Box<dyn Fn(usize) -> usize>);
        let mut known: Vec<(usize, u16)> = Vec::new();
        if high {
            t = m.next_power_of_two();
            n_end = t + k;
            orig_pos = Box::new(move |i| t + i);
            for i in 0..m {
                match rec[i] {
                    Some(v) => known.push((i, v)),
                    None => erased[i] = true,
                }
            }
            for i in m..t {
                erased[i] = true;
            }
            for i in 0..k {
                match orig[i] {
                    Some(v) => known.push((t + i, v)),
                    None => erased[t + i] = true,
                }
            }
        } else {
            t = k.next_power_of_two();
            n_end = t + m;
            orig_pos = Box::new(|i| i);
            for i in 0..k {
                match orig[i] {
                    Some(v) => known.push((i, v)),
                    None => erased[i] = true,
                }
            }
            for i in 0..m {
                match rec[i] {
                    Some(v) => known.push((t + i, v)),
                    None => erased[t + i] = true,
                }
            }
            for i in n_end..65536 {
                erased[i] = true;
            }
        }
        let w = n_end.next_power_of_two();
        let loc: Vec<u16> = match mode {
            Loc::Fwht => {
                let logs = locator_log_fwht(gf, &erased);
                (0..w).map(|i| gf.exp[logs[i] as usize]).collect()
            }
            Loc::Naive => locator_naive(gf, &erased, w),
            Loc::Truncated => locator_truncated(gf, &erased, w),
        };
        let mut work = vec![0u16; w];
        for &(p, v) in &known {
            work[p] = gf.mul(v, loc[p]);
        }
        gf.ifft(&mut work, 0);
        formal_derivative(&mut work);
        gf.fft(&mut work, 0);
        (0..k)
            .map(|i| match orig[i] {
                Some(v) => v,
                None => {
                    let p = orig_pos(i);
                    gf.mul(work[p], gf.inv(loc[p]))
                }
            })
            .collect()
    }

    // ---------------------------------------------------------------- §4.5
    /// Generic fallback decoder: Gaussian elimination on the generator matrix.
    pub fn decode_gauss(gf: &Gf16, orig: &[Option<u16>], rec: &[Option<u16>]) -> Vec<u16> {
        let (k, m) = (orig.len(), rec.len());
        let high = use_high_rate(k, m).unwrap();
        // Generator: row r of parity = Σ_i G[r][i] · d_i.
        let mut g = vec![vec![0u16; k]; m];
        for i in 0..k {
            let mut unit = vec![0u16; k];
            unit[i] = 1;
            let p = if high {
                encode_high(gf, &unit, m)
            } else {
                encode_low(gf, &unit, m)
            };
            for r in 0..m {
                g[r][i] = p[r];
            }
        }
        // Pick k equations: rows of the identity (data) and of G (parity).
        let mut rows: Vec<(Vec<u16>, u16)> = Vec::new();
        for i in 0..k {
            if let Some(v) = orig[i] {
                let mut row = vec![0u16; k];
                row[i] = 1;
                rows.push((row, v));
            }
        }
        for r in 0..m {
            if let Some(v) = rec[r] {
                rows.push((g[r].clone(), v));
            }
        }
        rows.truncate(k);
        assert_eq!(rows.len(), k, "not enough shards");
        // Gauss–Jordan elimination.
        for col in 0..k {
            let piv = (col..k).find(|&r| rows[r].0[col] != 0).expect("singular");
            rows.swap(col, piv);
            let inv = gf.inv(rows[col].0[col]);
            for x in rows[col].0.iter_mut() {
                *x = gf.mul(*x, inv);
            }
            rows[col].1 = gf.mul(rows[col].1, inv);
            for r in 0..k {
                if r != col && rows[r].0[col] != 0 {
                    let f = rows[r].0[col];
                    let (pc, pv) = (rows[col].0.clone(), rows[col].1);
                    for c in 0..k {
                        rows[r].0[c] ^= gf.mul(f, pc[c]);
                    }
                    rows[r].1 ^= gf.mul(f, pv);
                }
            }
        }
        rows.iter().map(|r| r.1).collect()
    }

    // ---------------------------------------------------------------- §4.1
    /// Symbol j (0..63) of a 128-byte shard.
    pub fn symbol(shard: &[u8], j: usize) -> u16 {
        let base = 64 * (j / 32) + j % 32;
        shard[base] as u16 | (shard[base + 32] as u16) << 8
    }

    pub fn set_symbol(shard: &mut [u8], j: usize, v: u16) {
        let base = 64 * (j / 32) + j % 32;
        shard[base] = v as u8;
        shard[base + 32] = (v >> 8) as u8;
    }

    /// Encodes whole 128-byte shards symbol by symbol.
    pub fn encode_shards(gf: &Gf16, data: &[Vec<u8>], m: usize) -> Vec<Vec<u8>> {
        let k = data.len();
        let high = use_high_rate(k, m).unwrap();
        let mut parity = vec![vec![0u8; 128]; m];
        for j in 0..64 {
            let col: Vec<u16> = data.iter().map(|s| symbol(s, j)).collect();
            let p = if high {
                encode_high(gf, &col, m)
            } else {
                encode_low(gf, &col, m)
            };
            for r in 0..m {
                set_symbol(&mut parity[r], j, p[r]);
            }
        }
        parity
    }

    // ---------------------------------------------------------------- §6.4
    /// Fisher–Yates shuffle with splitmix64 (cell order on a page).
    pub fn shuffle<T>(items: &mut [T], seed: u64) {
        let mut state = seed;
        let mut next = || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        for i in (1..items.len()).rev() {
            let j = (next() % (i as u64 + 1)) as usize;
            items.swap(i, j);
        }
    }
}

use reference as r;

// ===========================================================================
// §5.4 Whitening
// ===========================================================================

const WHITENING_FIRST_32: &str = "
79 18 e7 ba a6 4a 93 af 47 e3 12 5f c5 db 85 63
f3 28 08 03 2a 29 7a 00 a5 f6 12 59 14 c4 e6 f9";

#[test]
fn whitening_sequence() {
    let seq = r::whitening_sequence();
    let mut lib = [0u8; BLOCK_LEN];
    whiten(&mut lib);
    assert_eq!(seq, lib, "reference whitening differs from pabaru-format");
    assert_hex("whitening[0..32]", &seq[..32], WHITENING_FIRST_32);
}

// ===========================================================================
// §5.2 CRC32C
// ===========================================================================

#[test]
fn crc32c_vectors() {
    // Standard check value of CRC-32C.
    assert_eq!(r::crc32c(b"123456789"), 0xE306_9283);
    assert_eq!(crc32c::crc32c(b"123456789"), 0xE306_9283);
    // Header + payload of the sample block (§10.4).
    let body = sample_block_body();
    let crc = r::crc32c(&body);
    assert_eq!(crc, crc32c::crc32c(&body));
    assert_text(
        "crc32c(sample block body)",
        &format!("{crc:08x}"),
        "3c919e21",
    );
}

// ===========================================================================
// §5.3 Inner Reed–Solomon
// ===========================================================================

#[test]
fn inner_rs_generator_and_parity() {
    let gf = r::Gf256::new();
    let g = r::rs_generator(&gf);
    assert_eq!(g.len(), 61);
    assert_hex(
        "g(x) coefficients, x^60 first",
        &g,
        "01 68 84 06 cd 3a 15 7d 8d 48 8d 56 c1 b2 22 56
3b 18 31 cc 40 11 83 04 a7 07 ba 7c 56 22 bd e6
d3 4a 94 0b 8c e6 a2 76 b1 e8 97 60 31 6b 03 32
7f be 44 ae ac 5e 0c a2 4c e1 80 27 2c",
    );

    let mut msg = [0u8; 140];
    for (i, b) in msg.iter_mut().enumerate() {
        *b = i as u8;
    }
    let cw = r::rs_encode(&gf, &msg);
    let lib = pabaru_ecc::inner::ReedSolomon::v1().encode(&msg);
    assert_eq!(
        &cw[..],
        &lib[..],
        "reference RS encoder differs from pabaru-ecc"
    );
    assert_hex(
        "RS parity for data 00..8b",
        &cw[140..],
        "62 d6 ae 8e ef 42 90 d5 5c 2c a0 9c 5c cb 4f be
2a 60 c8 36 0d 05 a3 02 ae e3 c5 7f f1 58 c0 7a
ed 72 b1 5e 9f 57 e5 d6 61 ad 15 6d 35 36 e6 1e
d4 2a 87 7b 14 a6 93 18 00 97 12 d5",
    );
}

#[test]
fn inner_rs_reference_decoder_corrects() {
    let gf = r::Gf256::new();
    let mut rng = Rng(0x5EED_0001);
    for round in 0..300 {
        let msg: [u8; 140] = rng.bytes(140).try_into().unwrap();
        let clean = r::rs_encode(&gf, &msg);
        let erasures = rng.below(61);
        let errors = (60 - erasures) / 2;
        let pos = rng.distinct(200, erasures + errors);
        let mut bad = clean;
        for &p in &pos {
            bad[p] ^= 1 + rng.below(255) as u8;
        }
        // Sometimes leave an erased byte intact (erasure flags may be wrong).
        if round % 7 == 0 && erasures > 0 {
            bad[pos[0]] = clean[pos[0]];
        }
        let fixed = r::rs_decode(&gf, &bad, &pos[..erasures]);
        assert_eq!(fixed, Some(clean), "round {round}: e={errors} ρ={erasures}");
        // The production decoder agrees.
        let mut lib = bad.to_vec();
        let rs = pabaru_ecc::inner::ReedSolomon::v1();
        if erasures == 0 {
            rs.correct(&mut lib).unwrap();
        } else {
            rs.correct_with_erasures(&mut lib, &pos[..erasures])
                .unwrap();
        }
        assert_eq!(&lib[..], &clean[..]);
    }
    // Beyond capacity, the reference decoder must not return a wrong word silently
    // in the common case (it either fails or the CRC would catch it).
    let msg = [0x42u8; 140];
    let mut bad = r::rs_encode(&gf, &msg);
    for b in bad.iter_mut().take(40) {
        *b ^= 0xFF;
    }
    assert_eq!(r::rs_decode(&gf, &bad, &[]), None);
}

// ===========================================================================
// §5 / §10.4 Complete block
// ===========================================================================

const SAMPLE_ID: u32 = 0x1234_ABCD;
const SAMPLE_INDEX: u32 = 7;

fn sample_payload() -> [u8; PAYLOAD_LEN] {
    let mut p = [0u8; PAYLOAD_LEN];
    for (i, b) in p.iter_mut().enumerate() {
        *b = i as u8;
    }
    p
}

fn sample_block_body() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&SAMPLE_ID.to_le_bytes());
    body.extend_from_slice(&SAMPLE_INDEX.to_le_bytes());
    body.extend_from_slice(&sample_payload());
    body
}

/// Reference block encoder (§5): header ‖ payload ‖ CRC ‖ parity, then whitening.
fn ref_encode_block(backup_id: u32, index: u32, payload: &[u8; 128]) -> ([u8; 200], [u8; 200]) {
    let gf = r::Gf256::new();
    let mut data = [0u8; 140];
    data[0..4].copy_from_slice(&backup_id.to_le_bytes());
    data[4..8].copy_from_slice(&index.to_le_bytes());
    data[8..136].copy_from_slice(payload);
    let crc = r::crc32c(&data[..136]);
    data[136..140].copy_from_slice(&crc.to_le_bytes());
    let cw = r::rs_encode(&gf, &data);
    let seq = r::whitening_sequence();
    let mut printed = cw;
    for i in 0..200 {
        printed[i] ^= seq[i];
    }
    (cw, printed)
}

/// Reference block decoder: de-whiten, RS, CRC. Returns (backup_id, index, payload).
fn ref_decode_block(printed: &[u8; 200]) -> Option<(u32, u32, [u8; 128])> {
    let gf = r::Gf256::new();
    let seq = r::whitening_sequence();
    let mut cw = *printed;
    for i in 0..200 {
        cw[i] ^= seq[i];
    }
    let cw = r::rs_decode(&gf, &cw, &[])?;
    let crc = u32::from_le_bytes(cw[136..140].try_into().unwrap());
    if r::crc32c(&cw[..136]) != crc {
        return None;
    }
    Some((
        u32::from_le_bytes(cw[0..4].try_into().unwrap()),
        u32::from_le_bytes(cw[4..8].try_into().unwrap()),
        cw[8..136].try_into().unwrap(),
    ))
}

/// Renders the dots of a block as text rows ('#' = black = 1, '.' = white = 0).
fn dot_rows(printed: &[u8; 200], rows: usize) -> String {
    (0..rows)
        .map(|y| {
            (0..40)
                .map(|x| {
                    let d = y * 40 + x;
                    if (printed[d / 8] >> (7 - d % 8)) & 1 == 1 {
                        '#'
                    } else {
                        '.'
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn complete_block() {
    let (cw, printed) = ref_encode_block(SAMPLE_ID, SAMPLE_INDEX, &sample_payload());
    let lib = Block {
        header: BlockHeader {
            backup_id: SAMPLE_ID,
            index: SAMPLE_INDEX,
        },
        payload: sample_payload(),
    }
    .encode();
    assert_eq!(
        printed, lib,
        "reference block encoder differs from pabaru-format"
    );
    assert_hex(
        "block before whitening",
        &cw,
        "cd ab 34 12 07 00 00 00 00 01 02 03 04 05 06 07
08 09 0a 0b 0c 0d 0e 0f 10 11 12 13 14 15 16 17
18 19 1a 1b 1c 1d 1e 1f 20 21 22 23 24 25 26 27
28 29 2a 2b 2c 2d 2e 2f 30 31 32 33 34 35 36 37
38 39 3a 3b 3c 3d 3e 3f 40 41 42 43 44 45 46 47
48 49 4a 4b 4c 4d 4e 4f 50 51 52 53 54 55 56 57
58 59 5a 5b 5c 5d 5e 5f 60 61 62 63 64 65 66 67
68 69 6a 6b 6c 6d 6e 6f 70 71 72 73 74 75 76 77
78 79 7a 7b 7c 7d 7e 7f 21 9e 91 3c 1c 5c 6e 7f
71 0c 35 e3 09 9f 37 e5 f2 6c ee d6 59 27 ed 47
f8 a4 77 92 81 62 9d ef 5b 89 22 1c b2 6f d5 19
87 04 a4 e7 b5 6a 68 5b 53 49 20 81 b8 fc 7c e4
98 5d 28 41 80 dd 34 c2",
    );
    assert_hex(
        "block after whitening (printed)",
        &printed,
        "b4 b3 d3 a8 a1 4a 93 af 47 e2 10 5c c1 de 83 64
fb 21 02 08 26 24 74 0f b5 e7 00 4a 00 d1 f0 ee
d7 5e 44 7b 78 9f 30 c7 dd 71 05 9a d8 03 16 38
d7 d3 26 3e 37 c3 b4 47 d6 31 6e 34 6e dc b5 a0
b0 17 37 e0 ba 70 d3 ad 6d e7 c9 68 5a 0e 51 cc
0a 68 6e be 8d 05 6b d3 37 96 34 5a 5a 64 ad 8f
7e da f7 6a 9a 67 fc 68 55 38 eb 33 ce d2 f2 f6
08 8e dd 25 a5 50 e3 63 67 91 27 97 01 18 8e 34
4a 35 0e 8b 36 27 59 1c af ab 86 66 25 08 9e e1
1f 70 ab da d9 ec 09 04 00 86 9b 7b 67 1c 7a 11
ce 56 80 02 9b c9 9f 05 b1 e5 13 87 80 5d e4 13
d8 58 68 bc 89 f6 49 00 ea 99 d5 fd 7a 08 85 58
90 37 2c 7c 81 5d 15 da",
    );
    // Dot mapping (§5.5) agrees with pabaru-format.
    for y in 0..40 {
        for x in 0..40 {
            let d = y * 40 + x;
            assert_eq!(
                (printed[d / 8] >> (7 - d % 8)) & 1 == 1,
                dots::dot_at(&printed, x, y)
            );
        }
    }
    assert_text(
        "first 4 dot rows",
        &dot_rows(&printed, 4),
        "#.##.#..#.##..####.#..###.#.#...#.#....#
.#..#.#.#..#..###.#.####.#...######...#.
...#.....#.###..##.....###.####.#.....##
.##..#..#####.##..#....#......#.....#...",
    );
    assert_eq!(
        ref_decode_block(&printed),
        Some((SAMPLE_ID, SAMPLE_INDEX, sample_payload()))
    );
}

// ===========================================================================
// §4 Outer code
// ===========================================================================

#[test]
fn gf16_tables_match_library() {
    use reed_solomon_simd::engine::tables;
    let gf = r::Gf16::new();
    let lib = tables::get_exp_log();
    assert_eq!(&gf.log[..], &lib.log[..], "log table");
    assert_eq!(&gf.exp[..], &lib.exp[..], "exp table");
    let lib_skew = tables::get_skew();
    for i in 0..65535 {
        assert_eq!(gf.log[gf.skew[i] as usize], lib_skew[i], "skew[{i}]");
    }
    // Spot values printed in SPEC.md (appendix A.2).
    let spots = format!(
        "log[1]={} log[2]={} log[3]={} log[0xACCA]={} exp[1]={:#06x} exp[2]={:#06x} exp[100]={:#06x}\n\
         skew[0..8]={:04x?}\nskew[65534]={:#06x}",
        gf.log[1],
        gf.log[2],
        gf.log[3],
        gf.log[0xACCA],
        gf.exp[1],
        gf.exp[2],
        gf.exp[100],
        &gf.skew[..8],
        gf.skew[65534],
    );
    assert_text("GF(2^16) spot values", &spots, "log[1]=0 log[2]=21845 log[3]=43690 log[0xACCA]=64373 exp[1]=0x4690 exp[2]=0x65d8 exp[100]=0xdeed
skew[0..8]=[0000, 0000, 0002, 0000, 0004, 0002, 0006, 0000]
skew[65534]=0xfffe");
}

/// Small outer-code example: data symbol 0 of shard i = 0x0101·(i+1) ^ 0x1000·i, symbol 1 = 0x00FF ^ i.
/// Other symbols are zero. Returns (data shards, symbol table text).
fn outer_example(k: usize, m: usize) -> (Vec<Vec<u8>>, Vec<Vec<u8>>) {
    let data: Vec<Vec<u8>> = (0..k)
        .map(|i| {
            let mut s = vec![0u8; 128];
            r::set_symbol(&mut s, 0, (0x0101 * (i as u16 + 1)) ^ (0x1000 * i as u16));
            r::set_symbol(&mut s, 1, 0x00FF ^ i as u16);
            s
        })
        .collect();
    let parity = pabaru_ecc::outer::encode(&data, m).unwrap();
    (data, parity)
}

fn symbols_text(shards: &[Vec<u8>], first: usize) -> String {
    shards
        .iter()
        .enumerate()
        .map(|(i, s)| {
            format!(
                "shard {:>2}: s0={:04x} s1={:04x}",
                first + i,
                r::symbol(s, 0),
                r::symbol(s, 1)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn outer_small_examples() {
    let gf = r::Gf16::new();
    let cases: [(usize, usize, &str); 6] = [
        (
            3,
            2,
            "k=3 m=2 rate=high
shard  0: s0=0101 s1=00ff
shard  1: s0=1202 s1=00fe
shard  2: s0=2303 s1=00fd
shard  3: s0=bf82 s1=00e7
shard  4: s0=8f82 s1=001b",
        ),
        (
            2,
            5,
            "k=2 m=5 rate=low
shard  0: s0=0101 s1=00ff
shard  1: s0=1202 s1=00fe
shard  2: s0=2cad s1=00fd
shard  3: s0=3fae s1=00fc
shard  4: s0=44ea s1=00fb
shard  5: s0=57e9 s1=00fa
shard  6: s0=6946 s1=00f9",
        ),
        (
            3,
            4,
            "k=3 m=4 rate=high
shard  0: s0=0101 s1=00ff
shard  1: s0=1202 s1=00fe
shard  2: s0=2303 s1=00fd
shard  3: s0=2d90 s1=002f
shard  4: s0=2b94 s1=006a
shard  5: s0=df52 s1=0076
shard  6: s0=e956 s1=00cf",
        ),
        (
            4,
            3,
            "k=4 m=3 rate=low
shard  0: s0=0101 s1=00ff
shard  1: s0=1202 s1=00fe
shard  2: s0=2303 s1=00fd
shard  3: s0=3404 s1=00fc
shard  4: s0=4763 s1=00fb
shard  5: s0=5f54 s1=00fa
shard  6: s0=6cf9 s1=00f9",
        ),
        (
            1,
            3,
            "k=1 m=3 rate=low
shard  0: s0=0101 s1=00ff
shard  1: s0=0101 s1=00ff
shard  2: s0=0101 s1=00ff
shard  3: s0=0101 s1=00ff",
        ),
        (
            5,
            11,
            "k=5 m=11 rate=low
shard  0: s0=0101 s1=00ff
shard  1: s0=1202 s1=00fe
shard  2: s0=2303 s1=00fd
shard  3: s0=3404 s1=00fc
shard  4: s0=4505 s1=00fb
shard  5: s0=0875 s1=0056
shard  6: s0=f913 s1=00ce
shard  7: s0=253c s1=0079
shard  8: s0=6167 s1=00af
shard  9: s0=ccb9 s1=001f
shard 10: s0=f5ac s1=0022
shard 11: s0=bbaa s1=00e7
shard 12: s0=7683 s1=006f
shard 13: s0=318b s1=00c0
shard 14: s0=12a7 s1=0010
shard 15: s0=9169 s1=0093",
        ),
    ];
    for (k, m, expected) in cases {
        let (data, parity) = outer_example(k, m);
        // All other symbols are zero.
        for p in &parity {
            for j in 2..64 {
                assert_eq!(r::symbol(p, j), 0);
            }
        }
        let rate = if r::use_high_rate(k, m).unwrap() {
            "high"
        } else {
            "low"
        };
        let text = format!(
            "k={k} m={m} rate={rate}\n{}\n{}",
            symbols_text(&data, 0),
            symbols_text(&parity, k)
        );
        assert_eq!(r::encode_shards(&gf, &data, m), parity, "k={k} m={m}");
        assert_text(&format!("outer k={k} m={m}"), &text, expected);
    }
    // k = 1: every parity shard is a copy of the data shard.
    let (data, parity) = outer_example(1, 7);
    assert!(parity.iter().all(|p| *p == data[0]));
}

#[test]
fn outer_full_shard_example_and_recovery() {
    // k=3, m=2 with full 128-byte shards: byte j of data shard i = (i·64 + j·3) mod 256.
    let k = 3;
    let data: Vec<Vec<u8>> = (0..k)
        .map(|i| (0..128).map(|j| ((i * 64 + j * 3) % 256) as u8).collect())
        .collect();
    let parity = pabaru_ecc::outer::encode(&data, 2).unwrap();
    let gf = r::Gf16::new();
    assert_eq!(r::encode_shards(&gf, &data, 2), parity);
    assert_hex(
        "outer k=3 m=2 parity shard 3 (index k+0)",
        &parity[0],
        "30 63 64 47 40 13 3e c5 1a 49 4e aa ad fe 21 da
f7 a4 a3 80 87 d4 b4 4f 90 c3 c4 15 12 41 9e 65
7c 73 71 7f 7d 72 3d 34 31 3e 3c 37 35 3a 3f 36
79 76 74 7a 78 77 87 8e 8b 84 86 c2 c0 cf ca c3
cb 98 9f bc bb e8 c5 3e e1 b2 b5 af a8 fb 24 df
f2 a1 a6 85 82 d1 cc 37 e8 bb bc 6d 6a 39 e6 1d
f1 fe fc f2 f0 ff b0 b9 bc b3 b1 c7 c5 ca cf c6
89 86 84 8a 88 87 77 7e 7b 74 76 32 30 3f 3a 33",
    );
    assert_hex(
        "outer k=3 m=2 parity shard 4 (index k+1)",
        &parity[1],
        "f0 a0 a2 8e 8c dc ec 10 c2 92 90 4b 49 19 cb 37
07 57 55 79 7b 2b b6 4a 98 c8 ca 04 06 56 84 78
5c 50 57 56 51 5d 0f 01 09 05 02 76 71 7d 75 7b
29 25 22 23 24 28 e5 eb e3 ef e8 b3 b4 b8 b0 be
4b 1b 19 35 37 67 57 ab 79 29 2b 0e 0c 5c 8e 72
42 12 10 3c 3e 6e 0e f2 20 70 72 bc be ee 3c c0
11 1d 1a 1b 1c 10 42 4c 44 48 4f c6 c1 cd c5 cb
99 95 92 93 94 98 55 5b 53 5f 58 03 04 08 00 0e",
    );

    // Recovery: shards 0 and 2 lost, shards 1, 3, 4 received.
    let mut received = BTreeMap::new();
    received.insert(1usize, data[1].clone());
    received.insert(3, parity[0].clone());
    received.insert(4, parity[1].clone());
    let back = pabaru_ecc::outer::decode(3, 2, &received).unwrap();
    assert_eq!(back, data);
    // Reference decoder, symbol by symbol.
    for j in 0..64 {
        let orig = vec![None, Some(r::symbol(&data[1], j)), None];
        let rec = vec![
            Some(r::symbol(&parity[0], j)),
            Some(r::symbol(&parity[1], j)),
        ];
        let want: Vec<u16> = data.iter().map(|s| r::symbol(s, j)).collect();
        assert_eq!(r::decode(&gf, &orig, &rec, r::Loc::Naive), want);
        assert_eq!(r::decode(&gf, &orig, &rec, r::Loc::Fwht), want);
        assert_eq!(r::decode(&gf, &orig, &rec, r::Loc::Truncated), want);
        assert_eq!(r::decode_gauss(&gf, &orig, &rec), want);
    }
}

/// Encodes with a specific rate of the library (bypassing the rate selection).
fn lib_encode_rate(high: bool, data: &[Vec<u8>], m: usize) -> Vec<Vec<u8>> {
    use reed_solomon_simd::engine::Naive;
    use reed_solomon_simd::rate::{HighRateEncoder, LowRateEncoder, RateEncoder};
    let k = data.len();
    if high {
        let mut e = HighRateEncoder::new(k, m, 128, Naive::new(), None).unwrap();
        for s in data {
            e.add_original_shard(s).unwrap();
        }
        e.encode()
            .unwrap()
            .recovery_iter()
            .map(<[u8]>::to_vec)
            .collect()
    } else {
        let mut e = LowRateEncoder::new(k, m, 128, Naive::new(), None).unwrap();
        for s in data {
            e.add_original_shard(s).unwrap();
        }
        e.encode()
            .unwrap()
            .recovery_iter()
            .map(<[u8]>::to_vec)
            .collect()
    }
}

#[test]
fn outer_reference_matches_library_sweep() {
    use reed_solomon_simd::rate::{HighRate, LowRate, Rate};
    type E = reed_solomon_simd::engine::Naive;
    let gf = r::Gf16::new();
    let mut rng = Rng(0x5EED_0002);
    let mut cases: Vec<(usize, usize)> = Vec::new();
    for k in 1..=9 {
        for m in 1..=9 {
            cases.push((k, m));
        }
    }
    cases.extend([
        (100, 7),
        (7, 100),
        (33, 31),
        (31, 33),
        (64, 64),
        (65, 64),
        (1, 40),
        (40, 1),
    ]);
    for (k, m) in cases {
        let data: Vec<Vec<u8>> = (0..k).map(|_| rng.bytes(128)).collect();
        let lib = pabaru_ecc::outer::encode(&data, m).unwrap();
        assert_eq!(
            r::encode_shards(&gf, &data, m),
            lib,
            "default rate k={k} m={m}"
        );
        // Each rate separately (where the library supports it).
        let col: Vec<u16> = data.iter().map(|s| r::symbol(s, 5)).collect();
        if <HighRate<E> as Rate<E>>::supports(k, m) {
            let p = lib_encode_rate(true, &data, m);
            let refp = r::encode_high(&gf, &col, m);
            for i in 0..m {
                assert_eq!(r::symbol(&p[i], 5), refp[i], "high k={k} m={m}");
            }
        }
        if <LowRate<E> as Rate<E>>::supports(k, m) {
            let p = lib_encode_rate(false, &data, m);
            let refp = r::encode_low(&gf, &col, m);
            for i in 0..m {
                assert_eq!(r::symbol(&p[i], 5), refp[i], "low k={k} m={m}");
            }
        }
        // Random loss of up to m shards; recover with the library and the reference.
        let nlost = rng.below(m + 1);
        let lost = rng.distinct(k + m, nlost);
        let mut received = BTreeMap::new();
        for i in 0..k + m {
            if !lost.contains(&i) {
                let s = if i < k { &data[i] } else { &lib[i - k] };
                received.insert(i, s.clone());
            }
        }
        assert_eq!(pabaru_ecc::outer::decode(k, m, &received).unwrap(), data);
        for j in [0usize, 31, 32, 63] {
            let orig: Vec<Option<u16>> = (0..k)
                .map(|i| received.get(&i).map(|s| r::symbol(s, j)))
                .collect();
            let rec: Vec<Option<u16>> = (0..m)
                .map(|i| received.get(&(k + i)).map(|s| r::symbol(s, j)))
                .collect();
            let want: Vec<u16> = data.iter().map(|s| r::symbol(s, j)).collect();
            assert_eq!(
                r::decode(&gf, &orig, &rec, r::Loc::Fwht),
                want,
                "fft k={k} m={m}"
            );
            assert_eq!(
                r::decode(&gf, &orig, &rec, r::Loc::Truncated),
                want,
                "truncated locator k={k} m={m}"
            );
            if k + m <= 20 {
                assert_eq!(
                    r::decode(&gf, &orig, &rec, r::Loc::Naive),
                    want,
                    "naive k={k} m={m}"
                );
            }
            if k <= 40 {
                assert_eq!(r::decode_gauss(&gf, &orig, &rec), want, "gauss k={k} m={m}");
            }
        }
    }
}

#[test]
fn outer_rate_selection_and_limits() {
    let pairs = [
        ((3, 2), Some(true)),
        ((2, 5), Some(false)),
        ((3, 4), Some(true)),
        ((4, 3), Some(false)),
        ((4, 4), Some(true)),
        ((1, 1), Some(true)),
        ((1, 9), Some(false)),
        ((40_000, 25_000), None),
        ((32_768, 32_768), Some(true)),
        ((60_000, 4_000), Some(true)),
        ((60_000, 5_000), None),
    ];
    for ((k, m), want) in pairs {
        assert_eq!(r::use_high_rate(k, m), want, "k={k} m={m}");
        assert_eq!(
            reed_solomon_simd::ReedSolomonEncoder::supports(k, m),
            want.is_some()
        );
    }
}

// ===========================================================================
// §3 Encryption
// ===========================================================================

const PASSWORD: &str = "correct horse battery staple";
const PLAINTEXT: &[u8] = b"Hello, paper!\n";

fn test_salt() -> [u8; 16] {
    core::array::from_fn(|i| i as u8)
}

fn test_nonce() -> [u8; 24] {
    core::array::from_fn(|i| 0xA0 + i as u8)
}

fn argon2id(password: &str, salt: &[u8; 16], m: u32, t: u32, p: u32) -> [u8; 32] {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(m, t, p, Some(32)).unwrap();
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .unwrap();
    key
}

fn xchacha_seal(key: &[u8; 32], nonce: &[u8; 24], plain: &[u8]) -> Vec<u8> {
    use chacha20poly1305::aead::{Aead, KeyInit, Payload};
    use chacha20poly1305::{XChaCha20Poly1305, XNonce};
    let cipher = XChaCha20Poly1305::new(key.into());
    let ct = cipher
        .encrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: plain,
                aad: b"",
            },
        )
        .unwrap();
    let mut out = nonce.to_vec();
    out.extend_from_slice(&ct);
    out
}

#[test]
fn encryption_vectors() {
    let salt = test_salt();
    // Small parameters (fast) – m = 256 KiB, t = 1, p = 1.
    let key = argon2id(PASSWORD, &salt, 256, 1, 1);
    assert_hex(
        "argon2id key (m=256, t=1, p=1)",
        &key,
        "8b 34 e4 a5 88 8a b0 48 e9 62 c0 74 a3 14 d5 45
4f 4f 25 bd 88 ab e8 e3 d0 21 38 86 72 c1 d1 a4",
    );
    let blob = xchacha_seal(&key, &test_nonce(), PLAINTEXT);
    assert_hex(
        "encrypted stream (m=256, t=1, p=1)",
        &blob,
        "a0 a1 a2 a3 a4 a5 a6 a7 a8 a9 aa ab ac ad ae af
b0 b1 b2 b3 b4 b5 b6 b7 db 13 fb ec 4a a7 18 b3
e2 dd 07 01 a4 47 4a 4a 8f ec 27 c1 c6 bc 17 1d
bd ec c7 80 6a c8",
    );
    let cfg = pabaru_crypto::KdfConfig {
        mem_kib: 256,
        iterations: 1,
        parallelism: 1,
    };
    assert_eq!(
        pabaru_crypto::decrypt(&blob, PASSWORD, &salt, cfg).unwrap(),
        PLAINTEXT
    );
    // Pabaru's own encrypt() has the same layout (random nonce).
    let own = pabaru_crypto::encrypt(PLAINTEXT, PASSWORD, &salt, cfg).unwrap();
    assert_eq!(own.len(), PLAINTEXT.len() + 40);
    assert_eq!(
        xchacha_seal(&key, own[..24].try_into().unwrap(), PLAINTEXT),
        own
    );

    // Default parameters – m = 65536 KiB, t = 3, p = 1.
    let def = pabaru_crypto::KdfConfig::default();
    assert_eq!(
        (def.mem_kib, def.iterations, def.parallelism),
        (65536, 3, 1)
    );
    let key = argon2id(PASSWORD, &salt, 65536, 3, 1);
    assert_hex(
        "argon2id key (defaults m=65536, t=3, p=1)",
        &key,
        "0d 1a 3c 65 23 c8 f0 6e 4e 0a f9 c5 15 aa 5b 54
48 cf eb d6 83 8f 2d 52 c3 d8 b6 ef 8d dc 3c 2e",
    );
    let blob = xchacha_seal(&key, &test_nonce(), PLAINTEXT);
    assert_eq!(
        pabaru_crypto::decrypt(&blob, PASSWORD, &salt, def).unwrap(),
        PLAINTEXT
    );
}

// ===========================================================================
// §10.7 Complete tiny backup
// ===========================================================================

/// The stream produced by `pabaru_archive::pack` (zstd level 19) for a single
/// file `hello.txt` = "Hello, paper!\n" (mtime 2026-01-01T00:00:00Z, uid 501,
/// gid 0, mode 0644). Hard-coded because tar headers depend on the file system
/// and zstd output may change between zstd versions.
const SAMPLE_STREAM: &str = "
28 b5 2f fd 00 68 2d 08 00 52 cd 29 19 50 d7 03
6c 57 77 55 35 65 b1 32 2a 9b 81 08 70 85 90 c2
9f a3 33 8f 24 c8 40 90 65 0c 63 71 48 fc 29 9d
b3 ca af 0f ed 7b b5 86 bb fb 3b fc 80 fb d4 46
de a7 a1 8e b5 f7 6b c5 3e af 3d 9c 91 55 de dc
38 7f 34 c7 3b 86 a5 ef d5 0d 06 e3 59 9f 6b 19
69 3d db c3 91 f9 67 9e 30 c5 d0 cf 5d a4 07 68
63 2c 53 d1 b3 c4 08 7c c2 e6 36 71 51 54 86 84
5c 61 4b 88 80 a2 f7 75 65 c3 47 44 b0 ae 48 67
79 13 62 04 3c 18 63 86 b7 d6 20 7d ea ef d0 eb
a4 d4 3a b4 2e 2d 11 5b e7 27 b2 70 55 68 19 66
30 46 04 25 20 c0 48 82 ab 30 79 d7 68 f4 07 15
29 57 6f 80 f5 4a 43 05 76 04 50 22 b0 58 8c f1
2e e1 bd 24 66 80 0c 88 fa 8c cb 56 00 0a 44 0d
10 72 18 19 06 ab d6 0e d2 13 6c 39 37 c9 fe 74
a3 f5 19 f4 11 26 02 70 60 44 40 1f 82 45 07 c5
05 d2 0a c0 5b 1b c2 92 4c e0 5a d4 35 01
";

const SAMPLE_MANIFEST: &str = r#"{
  "version": 1,
  "roots": [
    {
      "name": "hello.txt"
    }
  ],
  "files": [
    {
      "path": "hello.txt",
      "size": 14,
      "blake3": "1e2d86e062b69f53aab70501467d18c81dce9252b086930a551966d4cf92c5f9"
    }
  ]
}"#;

const SAMPLE_BACKUP_ID: u32 = 0x1234_ABCD;

fn sample_params() -> pabaru_layout::PrintParams {
    pabaru_layout::PrintParams {
        printer_dpi: 600,
        px_per_dot: 4,
        dot_fill_pct: 70,
        paper: pabaru_layout::Paper::Custom {
            width_mm: 40.0,
            height_mm: 40.0,
        },
        margin_mm: 3.0,
        header_mm: 3.0,
    }
}

#[test]
fn sample_stream_contents() {
    let stream = unhex(SAMPLE_STREAM);
    assert_eq!(stream.len(), 270);
    let tar = zstd::decode_all(&stream[..]).unwrap();
    assert_eq!(tar.len(), 3072);
    // Entry 1: manifest header + data, entry 2: file header + data, 2 zero blocks.
    assert_eq!(&tar[0..21], b".pabaru/manifest.json");
    assert_eq!(&tar[257..263], b"ustar ");
    assert_eq!(&tar[263..265], b" \0");
    assert_eq!(tar[156], 0, "manifest typeflag is NUL");
    assert_eq!(&tar[124..136], b"00000000350\0", "manifest size 232");
    assert_eq!(&tar[512..512 + 232], SAMPLE_MANIFEST.as_bytes());
    assert_eq!(&tar[1024..1033], b"hello.txt");
    assert_eq!(tar[1024 + 156], b'0');
    assert_eq!(
        &tar[1024 + 100..1024 + 108],
        b"0100644\0",
        "mode incl. S_IFREG"
    );
    assert_eq!(&tar[1536..1536 + 14], PLAINTEXT);
    assert!(tar[2048..].iter().all(|&b| b == 0));
    let manifest = pabaru_archive::read_manifest(&stream).unwrap();
    assert_eq!(manifest.files[0].path, "hello.txt");
    assert_eq!(
        manifest.files[0].blake3,
        blake3::hash(PLAINTEXT).to_hex().to_string()
    );
    // Full restore through the production code.
    let dir = tempfile::tempdir().unwrap();
    pabaru_archive::unpack(&stream, dir.path()).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("hello.txt")).unwrap(),
        PLAINTEXT
    );
}

/// Reads the page bitmap like an ideal scanner (§7): the colour of every
/// dot centre. Returns the printed 200-byte block of every cell (row order).
fn read_cells_from_bitmap(
    bmp: &pabaru_render::Bitmap,
    g: &pabaru_layout::PageGeometry,
    fill_pct: u8,
) -> Vec<[u8; 200]> {
    let ppd = g.px_per_dot;
    let off = sample_offset(ppd, fill_pct);
    let mut out = Vec::new();
    for row in 0..g.rows {
        for col in 0..g.cols {
            let (ox, oy) = (7 + 41 * col, 7 + 41 * row);
            let mut cw = [0u8; 200];
            for y in 0..40u32 {
                for x in 0..40u32 {
                    let px = g.region_x_px + (ox + x) * ppd + off;
                    let py = g.region_y_px + (oy + y) * ppd + off;
                    if bmp.is_black(px, py) {
                        let d = (y * 40 + x) as usize;
                        cw[d / 8] |= 1 << (7 - d % 8);
                    }
                }
            }
            out.push(cw);
        }
    }
    out
}

/// Pixel offset inside a dot's `ppd × ppd` square that lies inside the printed
/// data dot (SPEC §7.3): `pad + ⌊size/2⌋`.
fn sample_offset(ppd: u32, fill_pct: u8) -> u32 {
    let size = ((ppd as f32 * fill_pct as f32 / 100.0).round() as u32).clamp(1, ppd);
    let pad = (ppd - size) / 2;
    pad + size / 2
}

#[test]
fn dot_sampling_offset_for_all_dot_sizes() {
    // Every px_per_dot the encoder accepts in practice and every fill value: reading
    // pixel pad + ⌊size/2⌋ of every dot recovers the block exactly.
    let block = Block {
        header: BlockHeader {
            backup_id: SAMPLE_ID,
            index: SAMPLE_INDEX,
        },
        payload: sample_payload(),
    }
    .encode();
    let mut naive_centre_fails = false;
    for ppd in 1..=24u32 {
        let mut seen = std::collections::BTreeSet::new();
        for fill in (0..=100u8).chain([255]) {
            let size = ((ppd as f32 * fill as f32 / 100.0).round() as u32).clamp(1, ppd);
            if !seen.insert(size) {
                continue; // same rendering as an earlier fill value
            }
            let wd = pabaru_layout::geometry::region_dots(1);
            let g = pabaru_layout::PageGeometry {
                cols: 1,
                rows: 1,
                page_w_px: wd * ppd,
                page_h_px: wd * ppd,
                region_x_px: 0,
                region_y_px: 0,
                region_w_dots: wd,
                region_h_dots: wd,
                px_per_dot: ppd,
            };
            let grid = pabaru_layout::DotGrid::build(1, 1, &[block]);
            let bmp = pabaru_render::rasterize(&g, &grid, fill);
            assert_eq!(
                read_cells_from_bitmap(&bmp, &g, fill),
                vec![block],
                "ppd={ppd} fill={fill}"
            );
            assert_eq!(
                ref_render(&g, fill, &[block]),
                bmp.data,
                "ppd={ppd} fill={fill}"
            );
            // The geometric centre ⌊ppd/2⌋ is NOT always inside the dot.
            let centre = ppd / 2;
            let pad = (ppd - size) / 2;
            if !(pad..pad + size).contains(&centre) {
                naive_centre_fails = true;
            }
        }
    }
    assert!(naive_centre_fails);
    // The example from SPEC §7.3: ppd = 2, fill 70 % -> 1-pixel dot at offset 0.
    assert_eq!(sample_offset(2, 70), 0);
}

#[test]
fn complete_tiny_backup() {
    let stream = unhex(SAMPLE_STREAM);
    let packed = pabaru_archive::Packed {
        manifest: pabaru_archive::read_manifest(&stream).unwrap(),
        stream: stream.clone(),
        raw_bytes: 14,
    };
    let opts = pabaru_core::EncodeOptions {
        params: sample_params(),
        redundancy_pct: 20,
        survive_page_loss: false,
        password: None,
        name: "SPEC sample".into(),
        ..Default::default()
    };
    let backup = pabaru_core::encode_packed_with_id(&packed, &opts, SAMPLE_BACKUP_ID).unwrap();
    let g = backup.geometry;
    let info = &backup.info;
    let summary = format!(
        "page {}x{} px, region at ({}, {}) px, {}x{} dots, grid {}x{} cells\n\
         k={} m={} pages={}",
        g.page_w_px,
        g.page_h_px,
        g.region_x_px,
        g.region_y_px,
        g.region_w_dots,
        g.region_h_dots,
        g.cols,
        g.rows,
        info.data_shards,
        info.parity_shards,
        info.pages
    );
    assert_text(
        "sample geometry",
        &summary,
        "page 944x944 px, region at (118, 140) px, 177x177 dots, grid 4x4 cells
k=3 m=9 pages=1",
    );
    assert_text(
        "sample header lines",
        &backup.header_lines(0).join("\n"),
        "Pabaru paper backup | page 1/1 | ID 1234ABCD
Blocks: 3 data + 9 parity | dots 150 dpi | print at 100% scale | scan at >= 450 dpi in grayscale
Restore: pabaru decode <scans> [-o <folder>]",
    );

    // Superblock (§6), built field by field from the spec.
    let mut sb = [0u8; 128];
    sb[0..4].copy_from_slice(b"PBRU");
    sb[4] = 1;
    sb[5] = 0b01; // compressed, not encrypted
    sb[6..14].copy_from_slice(&(stream.len() as u64).to_le_bytes());
    sb[14..18].copy_from_slice(&info.data_shards.to_le_bytes());
    sb[18..22].copy_from_slice(&info.parity_shards.to_le_bytes());
    sb[22..24].copy_from_slice(&0u16.to_le_bytes());
    sb[24..26].copy_from_slice(&1u16.to_le_bytes());
    sb[26..28].copy_from_slice(&(g.cols as u16).to_le_bytes());
    sb[28..30].copy_from_slice(&(g.rows as u16).to_le_bytes());
    sb[30..46].copy_from_slice(&blake3::hash(&stream).as_bytes()[..16]);
    // 46..71: KDF fields are zero without encryption.
    sb[71] = 11;
    sb[72..83].copy_from_slice(b"SPEC sample");
    assert_hex(
        "sample superblock payload",
        &sb,
        "50 42 52 55 01 01 0e 01 00 00 00 00 00 00 03 00
00 00 09 00 00 00 00 00 01 00 04 00 04 00 27 1c
f5 c8 52 0a f5 c4 55 b1 60 5d a9 76 ef 40 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 0b 53 50 45 43 20 73 61 6d
70 6c 65 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00
00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00",
    );
    let parsed = Superblock::from_payload(&sb).unwrap();
    assert_eq!(parsed.name, "SPEC sample");

    // Render the page and read it back like a scanner.
    let bmp = backup.render_page(0);
    let bitmap_hash = blake3::hash(&bmp.data).to_hex().to_string();
    assert_text(
        "sample page bitmap BLAKE3",
        &bitmap_hash,
        "8770a1a05104398e54a7e892c04bca7208bdea6be4758f15b864de49e2de6a4c",
    );
    let cells = read_cells_from_bitmap(&bmp, &g, opts.params.dot_fill_pct);
    // Appendix §11.4 lists every printed block of the sample page.
    let mut missing = Vec::new();
    for (i, cell) in cells.iter().enumerate() {
        let name = format!("sample cell {i} (row {}, column {})", i / 4, i % 4);
        if printing() {
            println!("=== {name}\n{}\n", hex16(cell));
        }
        if !SPEC.contains(&hex16(cell)) {
            missing.push(name);
        }
    }
    assert!(missing.is_empty(), "docs/SPEC.md lacks {missing:?}");
    // The rendering rules of §7 reproduce the production bitmap exactly.
    assert_eq!(ref_render(&g, opts.params.dot_fill_pct, &cells), bmp.data);
    let mut layout = Vec::new();
    let mut shards: BTreeMap<usize, Vec<u8>> = BTreeMap::new();
    for cell in &cells {
        let (id, index, payload) = ref_decode_block(cell).expect("cell decodes");
        assert_eq!(id, SAMPLE_BACKUP_ID);
        if index == u32::MAX {
            assert_eq!(payload, sb, "superblock copy");
            layout.push("SB".to_string());
        } else {
            layout.push(index.to_string());
            shards.insert(index as usize, payload.to_vec());
        }
    }
    let layout_text = layout
        .chunks(g.cols as usize)
        .map(|r| r.iter().map(|c| format!("{c:>3}")).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    assert_text(
        "sample cell layout",
        &layout_text,
        " 11 SB SB  9
 10  7  8  0
 SB  6  5  2
  3  4 SB  1",
    );
    // The superblock block as printed.
    let (_, sb_printed) = ref_encode_block(SAMPLE_BACKUP_ID, u32::MAX, &sb);
    assert!(cells.contains(&sb_printed));
    assert_hex(
        "sample superblock block (printed)",
        &sb_printed,
        "b4 b3 d3 a8 59 b5 6c 50 17 a1 40 0a c4 da 8b 62
f3 28 08 03 2a 29 79 00 a5 f6 1b 59 14 c4 e6 f9
ce 47 5a 60 60 82 09 c4 08 98 75 b3 09 e2 65 ae
9f a7 a5 63 f4 ae 9a 68 e6 00 5c 07 5a e9 83 97
88 2e 0d db 86 4d ed 92 2d a6 8b 2b 1e 4b 17 80
11 71 61 b6 e1 3b 44 f1 17 ab 03 09 0e 31 fb d8
26 83 ad 31 c6 3a a2 37 35 59 89 50 aa b7 94 91
60 e7 b7 4e c9 3d 8d 0c 17 e0 55 e4 75 6d f8 43
32 4c 74 f0 4a 5a 27 63 c3 e7 35 d0 f5 8f c5 c1
0f 21 ad ae 8d 4d 2d 7f db 54 d4 47 79 0a 2f 63
06 98 05 d9 ba b0 2b d8 78 ce 20 41 27 92 ef 19
69 5a a1 ae fe 03 83 3e 75 f1 54 ab 7e 8b 1a 90
73 4d 7b e7 d2 a6 18 ee",
    );

    // Shuffle (§6.4): the reference reproduces the cell order.
    let (k, m) = (info.data_shards as usize, info.parity_shards as usize);
    let mut expect: Vec<String> = (0..k + m).map(|i| i.to_string()).collect();
    expect.resize(16, "SB".into());
    r::shuffle(&mut expect, SAMPLE_BACKUP_ID as u64);
    assert_eq!(expect, layout);

    // Data shards are the stream; parity from the reference encoder.
    let data: Vec<Vec<u8>> = (0..k).map(|i| shards[&i].clone()).collect();
    let mut joined = data.concat();
    joined.truncate(stream.len());
    assert_eq!(joined, stream);
    let gf = r::Gf16::new();
    let parity = r::encode_shards(&gf, &data, m);
    for (i, p) in parity.iter().enumerate() {
        assert_eq!(&shards[&(k + i)], p, "parity shard {}", k + i);
    }
    assert_hex(
        "sample parity shard k+0",
        &parity[0],
        "2a 92 69 9c 2b 0a b7 70 c4 a1 c0 89 54 56 bd fd
bb 03 d1 b8 ba cb 15 d4 ab 36 26 35 57 3f 48 6f
eb 29 25 d7 1f eb e5 6c 3e 46 22 da 0d 9e a1 db
97 4c e6 4b ba b5 ce 67 b2 82 c8 f7 12 bd 69 ef
0f d8 f5 8d ec da 39 f6 dd b9 ca a8 86 fd 8f df
24 36 90 74 fe 56 c5 8f 13 ea c0 1a 36 5b 67 33
bd ca 96 5a 75 a9 89 7b 59 57 ca 19 68 d9 83 1b
d9 d7 29 e3 d1 e7 0f dc 04 12 71 76 02 3b ba 8a",
    );

    // Lose all three data shards: recover them from parity with the reference decoder.
    for j in 0..64 {
        let orig = vec![None; k];
        let rec: Vec<Option<u16>> = (0..m).map(|i| Some(r::symbol(&parity[i], j))).collect();
        let want: Vec<u16> = data.iter().map(|s| r::symbol(s, j)).collect();
        assert_eq!(r::decode(&gf, &orig, &rec, r::Loc::Fwht), want);
    }

    // The production decoder restores the file from the rendered page.
    let mut decoder = pabaru_core::Decoder::new();
    let gray = bitmap_to_gray(&bmp);
    decoder.add_image(&gray).unwrap();
    let dir = tempfile::tempdir().unwrap();
    decoder
        .restore(None, pabaru_core::Destination::Folder(dir.path()))
        .unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("hello.txt")).unwrap(),
        PLAINTEXT
    );

    if std::env::var_os("PABARU_SPEC_WRITE").is_some() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../docs/img/spec/sample-page.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(&path).unwrap();
        pabaru_render::png::write_png(file, &bmp, opts.params.printer_dpi).unwrap();
    }
}

/// Reference renderer transcribed from SPEC.md §7.1–7.2. Returns the 1-bit
/// bitmap rows (1 = white, MSB = leftmost pixel) of a whole page.
fn ref_render(g: &pabaru_layout::PageGeometry, fill_pct: u8, cells: &[[u8; 200]]) -> Vec<u8> {
    let (cols, rows) = (g.cols as i64, g.rows as i64);
    let (wd, hd) = (41 * cols + 13, 41 * rows + 13);
    let is_line = |x: i64, y: i64| -> bool {
        if x < 0 || y < 0 || x >= wd || y >= hd {
            return false;
        }
        let in_frame = (4..wd - 4).contains(&x) && (4..hd - 4).contains(&y);
        let inside = (7..wd - 7).contains(&x) && (7..hd - 7).contains(&y);
        let frame = in_frame && !inside;
        let vline =
            (x - 6) % 41 == 0 && x >= 6 && x <= 6 + 41 * cols && (6..=6 + 41 * rows).contains(&y);
        let hline =
            (y - 6) % 41 == 0 && y >= 6 && y <= 6 + 41 * rows && (6..=6 + 41 * cols).contains(&x);
        frame || vline || hline
    };
    let ppd = g.px_per_dot;
    let size = ((ppd as f32 * fill_pct as f32 / 100.0).round() as u32).clamp(1, ppd);
    let pad = (ppd - size) / 2;
    let stride = (g.page_w_px as usize).div_ceil(8);
    let mut bmp = vec![0xFFu8; stride * g.page_h_px as usize];
    let mut fill = |x0: u32, y0: u32, w: u32, h: u32| {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                bmp[y as usize * stride + x as usize / 8] &= !(0x80 >> (x % 8));
            }
        }
    };
    for y in 0..hd {
        for x in 0..wd {
            let (px, py) = (
                g.region_x_px + x as u32 * ppd,
                g.region_y_px + y as u32 * ppd,
            );
            if is_line(x, y) {
                let vertical = is_line(x, y - 1) || is_line(x, y + 1);
                let horizontal = is_line(x - 1, y) || is_line(x + 1, y);
                match (vertical, horizontal) {
                    (true, false) => fill(px + pad, py, size, ppd),
                    (false, true) => fill(px, py + pad, ppd, size),
                    _ => fill(px, py, ppd, ppd),
                }
                continue;
            }
            // Data dot of cell (c, r)?
            let (cx, cy) = (x - 7, y - 7);
            if cx < 0
                || cy < 0
                || cx >= 41 * cols
                || cy >= 41 * rows
                || cx % 41 == 40
                || cy % 41 == 40
            {
                continue;
            }
            let cell = &cells[((cy / 41) * cols + cx / 41) as usize];
            let d = ((cy % 41) * 40 + cx % 41) as usize;
            if (cell[d / 8] >> (7 - d % 8)) & 1 == 1 {
                fill(px + pad, py + pad, size, size);
            }
        }
    }
    bmp
}

fn bitmap_to_gray(bmp: &pabaru_render::Bitmap) -> pabaru_core::Gray {
    let mut img = image::GrayImage::new(bmp.width, bmp.height);
    for y in 0..bmp.height {
        for x in 0..bmp.width {
            let v = if bmp.is_black(x, y) { 0 } else { 255 };
            img.put_pixel(x, y, image::Luma([v]));
        }
    }
    pabaru_core::Gray::from(&img)
}
