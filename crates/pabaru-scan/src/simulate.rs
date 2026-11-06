//! Print and scan simulation.
//!
//! From an ideal bitmap (as it would be sent to the printer) it produces a
//! "scan" with typical defects: rotation, scaling, stretching (uneven scanner
//! feed), blur, noise, spots and limited contrast.
//!
//! Used in tests and in the `pabaru verify` command, so that a backup can be
//! verified without a real printer and scanner.

use pabaru_render::Bitmap;

use crate::gray::Gray;

/// Circular spot (coordinates and radius as a fraction of the output size).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spot {
    pub x: f64,
    pub y: f64,
    pub radius: f64,
    /// Spot colour (0 = black, 255 = white – e.g. a faded area).
    pub value: u8,
}

/// Paper crumpling: smooth waviness + sharp creases.
///
/// Paper does not stretch – a crumpled and flattened page has in-plane
/// displacements of the order of tenths of a millimetre (remaining "bumps"
/// shorten the projection) and jumps at creases where the paper folded a
/// little. The presets correspond to that.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Crumple {
    /// Maximum in-plane paper displacement caused by waviness (mm).
    pub amplitude_mm: f64,
    /// Typical wavelength of the waviness (mm) – smaller = "denser" crumpling.
    pub wavelength_mm: f64,
    /// Number of sharp creases (lines along which the paper "jumps").
    pub creases: u32,
}

impl Crumple {
    /// Medium crumpling (page crushed in a hand and flattened).
    pub const MEDIUM: Crumple = Crumple {
        amplitude_mm: 0.4,
        wavelength_mm: 30.0,
        creases: 3,
    };
    /// Strong crumpling (tightly crushed ball).
    pub const STRONG: Crumple = Crumple {
        amplitude_mm: 0.8,
        wavelength_mm: 25.0,
        creases: 6,
    };
}

/// Page torn into two pieces.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tear {
    /// Rotation of the second piece (degrees).
    pub angle_deg: f64,
    /// Shift of the second piece (mm).
    pub shift_mm: (f64, f64),
    /// Jaggedness of the tear edge (mm).
    pub jag_mm: f64,
}

impl Default for Tear {
    fn default() -> Self {
        Self {
            angle_deg: 4.0,
            shift_mm: (8.0, 10.0),
            jag_mm: 3.0,
        }
    }
}

/// Which piece of the torn page to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Piece {
    Both,
    First,
    Second,
}

/// Simulation parameters.
#[derive(Debug, Clone, PartialEq)]
pub struct ScanSim {
    /// Scan pixels per printer pixel (600 dpi print, 600 dpi scan = 1.0).
    pub scale: f64,
    /// Rotation in degrees (positive = clockwise).
    pub angle_deg: f64,
    /// Vertical stretch (1.0 = none; 1.003 = +0.3 %).
    pub stretch_y: f64,
    /// Blur radius in scan pixels (0 = no blur).
    pub blur_radius: u32,
    /// Noise amplitude (± brightness value).
    pub noise: u8,
    pub spots: Vec<Spot>,
    /// Brightness of black and white (a scanner never gives pure 0 and 255).
    pub black: u8,
    pub white: u8,
    pub seed: u64,
    /// Printer resolution (bitmap px per inch) – for converting mm to pixels.
    pub printer_dpi: f64,
    pub crumple: Option<Crumple>,
    pub tear: Option<Tear>,
}

impl Default for ScanSim {
    fn default() -> Self {
        Self {
            scale: 1.0,
            angle_deg: 0.0,
            stretch_y: 1.0,
            blur_radius: 0,
            noise: 0,
            spots: Vec::new(),
            black: 30,
            white: 225,
            seed: 1,
            printer_dpi: 600.0,
            crumple: None,
            tear: None,
        }
    }
}

/// Simple deterministic random number generator (splitmix64).
struct Rng(u64);

impl Rng {
    fn next_f64(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    }
    /// Random number in the interval [a, b).
    fn range(&mut self, a: f64, b: f64) -> f64 {
        a + (b - a) * self.next_f64()
    }
}

/// One sinusoidal "wave" of paper waviness.
struct Wave {
    /// Propagation direction (unit vector) and wave number (rad/px).
    dir: (f64, f64),
    k: f64,
    phase: f64,
    /// Displacement vector (px).
    disp: (f64, f64),
}

/// Crease: a line beyond which the paper is displaced.
struct Crease {
    point: (f64, f64),
    normal: (f64, f64),
    jump: (f64, f64),
    /// Dark (negative) or light (positive) band along the crease.
    shade: f64,
}

/// Paper deformation field (in bitmap pixels).
struct Deformation {
    waves: Vec<Wave>,
    creases: Vec<Crease>,
    /// Width of the crease band (px).
    crease_width: f64,
}

impl Deformation {
    fn new(c: &Crumple, w: f64, h: f64, px_per_mm: f64, rng: &mut Rng) -> Self {
        let amp = c.amplitude_mm * px_per_mm;
        let lambda = c.wavelength_mm * px_per_mm;
        // Wave weights normalised so that the sum of displacements does not exceed the amplitude.
        let weights: Vec<f64> = (0..6).map(|_| rng.range(0.5, 1.0)).collect();
        let total: f64 = weights.iter().sum();
        let waves = weights
            .iter()
            .map(|w| {
                let a = rng.range(0.0, std::f64::consts::TAU);
                let b = rng.range(0.0, std::f64::consts::TAU);
                let l = lambda * rng.range(0.6, 1.4);
                let m = amp * w / total;
                Wave {
                    dir: (a.cos(), a.sin()),
                    k: std::f64::consts::TAU / l,
                    phase: rng.range(0.0, std::f64::consts::TAU),
                    disp: (m * b.cos(), m * b.sin()),
                }
            })
            .collect();
        let creases = (0..c.creases)
            .map(|_| {
                let a = rng.range(0.0, std::f64::consts::TAU);
                let j = rng.range(0.2, 0.6) * px_per_mm;
                let b = rng.range(0.0, std::f64::consts::TAU);
                Crease {
                    point: (rng.range(0.1, 0.9) * w, rng.range(0.1, 0.9) * h),
                    normal: (a.cos(), a.sin()),
                    jump: (j * b.cos(), j * b.sin()),
                    shade: if rng.next_f64() < 0.5 { -0.35 } else { 0.25 },
                }
            })
            .collect();
        Self {
            waves,
            creases,
            crease_width: 0.4 * px_per_mm,
        }
    }

    /// Paper displacement at a point (px) and darkening/lightening (−1 to 1).
    fn at(&self, x: f64, y: f64) -> ((f64, f64), f64) {
        let (mut dx, mut dy, mut shade) = (0.0, 0.0, 0.0);
        for wv in &self.waves {
            let arg = wv.k * (x * wv.dir.0 + y * wv.dir.1) + wv.phase;
            let s = arg.sin();
            dx += wv.disp.0 * s;
            dy += wv.disp.1 * s;
            // Shading by wave slope (subtle, roughly ±4 %).
            shade += 0.12 * arg.cos() / self.waves.len() as f64;
        }
        for c in &self.creases {
            let d = (x - c.point.0) * c.normal.0 + (y - c.point.1) * c.normal.1;
            if d > 0.0 {
                dx += c.jump.0;
                dy += c.jump.1;
            }
            if d.abs() < self.crease_width {
                shade += c.shade * (1.0 - d.abs() / self.crease_width);
            }
        }
        ((dx, dy), shade)
    }
}

/// Tear: a jagged line across the page centre; the second piece is shifted and rotated.
struct TearMap {
    center: (f64, f64),
    normal: (f64, f64),
    tangent: (f64, f64),
    jag: f64,
    jag_k: [f64; 3],
    /// Rotation (sin, cos) and shift of the second piece.
    rot: (f64, f64),
    shift: (f64, f64),
}

impl TearMap {
    fn new(t: &Tear, w: f64, h: f64, px_per_mm: f64, rng: &mut Rng) -> Self {
        // The tear runs roughly horizontally through the centre, with a random slope.
        let a = std::f64::consts::FRAC_PI_2 + rng.range(-0.5, 0.5);
        let (sin, cos) = t.angle_deg.to_radians().sin_cos();
        Self {
            center: (w * rng.range(0.4, 0.6), h * rng.range(0.4, 0.6)),
            normal: (a.cos(), a.sin()),
            tangent: (-a.sin(), a.cos()),
            jag: t.jag_mm * px_per_mm,
            jag_k: [
                rng.range(0.002, 0.006),
                rng.range(0.01, 0.03),
                rng.range(0.05, 0.1),
            ],
            rot: (sin, cos),
            shift: (t.shift_mm.0 * px_per_mm, t.shift_mm.1 * px_per_mm),
        }
    }

    /// Which side of the tear a point lies on (> 0 = second piece).
    fn side(&self, x: f64, y: f64) -> f64 {
        let (rx, ry) = (x - self.center.0, y - self.center.1);
        let along = rx * self.tangent.0 + ry * self.tangent.1;
        let across = rx * self.normal.0 + ry * self.normal.1;
        let jag: f64 = self
            .jag_k
            .iter()
            .enumerate()
            .map(|(i, k)| (along * k + i as f64).sin() / (i + 1) as f64)
            .sum();
        across - self.jag * jag
    }

    /// Output image point → original paper point, or `None` in the gap.
    fn source(&self, x: f64, y: f64, piece: Piece) -> Option<(f64, f64)> {
        // The second piece lies "on top" – try it first.
        if piece != Piece::First {
            let px = x - self.center.0 - self.shift.0;
            let py = y - self.center.1 - self.shift.1;
            let (sin, cos) = self.rot;
            let ox = px * cos + py * sin + self.center.0;
            let oy = -px * sin + py * cos + self.center.1;
            if self.side(ox, oy) > 0.0 {
                return Some((ox, oy));
            }
        }
        if piece != Piece::Second && self.side(x, y) <= 0.0 {
            return Some((x, y));
        }
        None
    }
}

/// Runs the simulation and returns a grayscale "scan".
pub fn simulate(bmp: &Bitmap, sim: &ScanSim) -> Gray {
    simulate_piece(bmp, sim, Piece::Both)
}

/// Torn page scanned piece by piece: returns two separate images.
/// Without a tear set (`sim.tear`) returns a single image.
pub fn simulate_torn_pieces(bmp: &Bitmap, sim: &ScanSim) -> Vec<Gray> {
    if sim.tear.is_none() {
        return vec![simulate(bmp, sim)];
    }
    vec![
        simulate_piece(bmp, sim, Piece::First),
        simulate_piece(bmp, sim, Piece::Second),
    ]
}

fn simulate_piece(bmp: &Bitmap, sim: &ScanSim, piece: Piece) -> Gray {
    let px_per_mm = sim.printer_dpi / 25.4;
    let mut rng = Rng(sim.seed ^ 0xC0FF_EE00);
    let (bw, bh) = (bmp.width as f64, bmp.height as f64);
    let deform = sim
        .crumple
        .map(|c| Deformation::new(&c, bw, bh, px_per_mm, &mut rng));
    let tear = sim
        .tear
        .map(|t| TearMap::new(&t, bw, bh, px_per_mm, &mut rng));

    let (sin, cos) = sim.angle_deg.to_radians().sin_cos();
    let sw = bmp.width as f64 * sim.scale;
    let sh = bmp.height as f64 * sim.scale * sim.stretch_y;
    // Output size = bounding box of the rotated rectangle + margin (+ shift of the torn piece).
    let extra = sim
        .tear
        .map(|t| (t.shift_mm.0.abs().max(t.shift_mm.1.abs()) + 20.0) * px_per_mm * sim.scale)
        .unwrap_or(0.0);
    let margin = 20.0 + extra;
    let ow = (sw * cos.abs() + sh * sin.abs() + 2.0 * margin).ceil() as u32;
    let oh = (sw * sin.abs() + sh * cos.abs() + 2.0 * margin).ceil() as u32;
    let (ocx, ocy) = (ow as f64 / 2.0, oh as f64 / 2.0);
    let (scx, scy) = (bmp.width as f64 / 2.0, bmp.height as f64 / 2.0);

    let mut out = Gray::new(ow, oh, sim.white);
    let range = sim.white as f64 - sim.black as f64;
    // 3×3 supersampling – mimics a scan pixel "seeing" its whole area.
    const SS: usize = 3;
    // Output point → paper point (rotation, scale, torn piece).
    // `None` = off the paper (gap left by the tear).
    let to_paper = |px: f64, py: f64| -> Option<(f64, f64)> {
        let rx = px * cos + py * sin;
        let ry = -px * sin + py * cos;
        let bx = rx / sim.scale + scx;
        let by = ry / (sim.scale * sim.stretch_y) + scy;
        match &tear {
            Some(t) => t.source(bx, by, piece),
            None => Some((bx, by)),
        }
    };
    let on_paper = |bx: f64, by: f64| bx >= 0.0 && by >= 0.0 && bx < bw && by < bh;

    for y in 0..oh {
        for x in 0..ow {
            // The deformation is smooth – computing it once for the pixel centre
            // is enough (9× faster than for every subsample).
            let center = to_paper(x as f64 + 0.5 - ocx, y as f64 + 0.5 - ocy);
            let ((dx, dy), mut shade) = match (&deform, center) {
                (Some(d), Some((cx, cy))) => d.at(cx, cy),
                _ => ((0.0, 0.0), 0.0),
            };
            // Shading applies only to the paper, not the scanner background.
            if !center.is_some_and(|(cx, cy)| on_paper(cx, cy)) {
                shade = 0.0;
            }
            let mut black = 0usize;
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = x as f64 + (sx as f64 + 0.5) / SS as f64 - ocx;
                    let py = y as f64 + (sy as f64 + 0.5) / SS as f64 - ocy;
                    // Gap left by the tear = scanner background (white).
                    let Some((bx, by)) = to_paper(px, py) else {
                        continue;
                    };
                    // Crumpling: inverse displacement by the deformation field.
                    let (bx, by) = (bx - dx, by - dy);
                    if bx >= 0.0 && by >= 0.0 && bmp.is_black(bx as u32, by as u32) {
                        black += 1;
                    }
                }
            }
            let frac = black as f64 / (SS * SS) as f64;
            let v = (sim.white as f64 - frac * range) * (1.0 + shade);
            out.set(x, y, v.round().clamp(0.0, 255.0) as u8);
        }
    }

    if sim.blur_radius > 0 {
        box_blur(&mut out, sim.blur_radius);
    }
    for spot in &sim.spots {
        draw_spot(&mut out, spot);
    }
    if sim.noise > 0 {
        add_noise(&mut out, sim.noise, sim.seed);
    }
    out
}

/// Box blur (separable: first horizontally, then vertically).
fn box_blur(img: &mut Gray, r: u32) {
    let (w, h) = (img.width as i64, img.height as i64);
    let r = r as i64;
    let mut tmp = img.clone();
    for y in 0..h {
        for x in 0..w {
            let s: u32 = (-r..=r).map(|d| img.get(x + d, y) as u32).sum();
            tmp.set(x as u32, y as u32, (s / (2 * r as u32 + 1)) as u8);
        }
    }
    for y in 0..h {
        for x in 0..w {
            let s: u32 = (-r..=r).map(|d| tmp.get(x, y + d) as u32).sum();
            img.set(x as u32, y as u32, (s / (2 * r as u32 + 1)) as u8);
        }
    }
}

fn draw_spot(img: &mut Gray, s: &Spot) {
    let cx = s.x * img.width as f64;
    let cy = s.y * img.height as f64;
    let r = s.radius * img.width.min(img.height) as f64;
    let (x0, x1) = (
        (cx - r).max(0.0) as u32,
        ((cx + r) as u32).min(img.width - 1),
    );
    let (y0, y1) = (
        (cy - r).max(0.0) as u32,
        ((cy + r) as u32).min(img.height - 1),
    );
    for y in y0..=y1 {
        for x in x0..=x1 {
            if (x as f64 - cx).powi(2) + (y as f64 - cy).powi(2) <= r * r {
                img.set(x, y, s.value);
            }
        }
    }
}

/// Adds uniform noise ±amp (deterministic given the seed).
fn add_noise(img: &mut Gray, amp: u8, seed: u64) {
    let mut state = seed | 1;
    for v in img.data.iter_mut() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        let n = (state % (2 * amp as u64 + 1)) as i32 - amp as i32;
        *v = (*v as i32 + n).clamp(0, 255) as u8;
    }
}
