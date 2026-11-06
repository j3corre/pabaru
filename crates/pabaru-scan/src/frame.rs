//! Finding the outer grid frame on a scan.
//!
//! Procedure:
//! 1. Downscale the image (for speed – A4 at 600 dpi has 35 million pixels).
//! 2. Binarise it with Otsu's threshold.
//! 3. Find connected dark regions. The frame is connected to all grid lines,
//!    so it forms the largest region. Regions touching the image border are
//!    ignored (the scanner's black border, lid shadow…).
//! 4. Frame corners = the region's extreme points along the diagonals
//!    (minimum/maximum of `x + y` and `x − y`). For a convex quadrilateral
//!    rotated by less than 45° these are exactly its vertices.
//! 5. Refine the corners at full resolution.

use crate::gray::Gray;

/// Frame corners in scan pixels (outer edges of the frame).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Corners {
    pub tl: (f64, f64),
    pub tr: (f64, f64),
    pub br: (f64, f64),
    pub bl: (f64, f64),
}

/// Result of the frame search.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    pub corners: Corners,
    /// Black/white threshold determined by Otsu's method.
    pub threshold: u8,
}

/// Statistics of one connected region.
#[derive(Debug, Clone, Copy)]
struct Component {
    count: u64,
    min_x: u32,
    min_y: u32,
    max_x: u32,
    max_y: u32,
    /// Extreme points: (min x+y), (max x−y), (max x+y), (min x−y).
    ext: [(u32, u32); 4],
}

/// Finds the frame. Returns `None` if the image contains none.
pub fn find_frame(img: &Gray) -> Option<Frame> {
    let factor = (img.width.max(img.height) / 1200).max(1);
    let small = img.downscale(factor);
    // One threshold for the downscaled image (region search) and a separate
    // one for full resolution: in the downscaled image the data is blurred
    // into grey, so its threshold is shifted towards light and would be
    // useless for individual dots.
    let small_threshold = small.otsu_threshold();
    let threshold = img.otsu_threshold();

    let best = largest_inner_component(&small, small_threshold)?;
    // A region this small cannot be the frame.
    if best.max_x - best.min_x < 20 || best.max_y - best.min_y < 20 {
        return None;
    }

    // Refine the corners at full resolution.
    let refine = |p: (u32, u32), dir: usize| -> (f64, f64) {
        let cx = (p.0 * factor + factor / 2) as i64;
        let cy = (p.1 * factor + factor / 2) as i64;
        let r = (2 * factor + 2) as i64;
        // Extremity score for the given direction (larger = more extreme).
        let score = |x: i64, y: i64| match dir {
            0 => -(x + y), // TL
            1 => x - y,    // TR
            2 => x + y,    // BR
            _ => y - x,    // BL
        };
        let mut best: Option<(i64, i64, i64)> = None;
        for y in cy - r..=cy + r {
            for x in cx - r..=cx + r {
                if img.get(x, y) <= threshold {
                    let s = score(x, y);
                    if best.is_none_or(|b| s > b.2) {
                        best = Some((x, y, s));
                    }
                }
            }
        }
        let (x, y, _) = best.unwrap_or((cx, cy, 0));
        // Outer edge of the pixel according to the corner direction.
        let (x, y) = (x as f64, y as f64);
        match dir {
            0 => (x, y),
            1 => (x + 1.0, y),
            2 => (x + 1.0, y + 1.0),
            _ => (x, y + 1.0),
        }
    };

    Some(Frame {
        corners: Corners {
            tl: refine(best.ext[0], 0),
            tr: refine(best.ext[1], 1),
            br: refine(best.ext[2], 2),
            bl: refine(best.ext[3], 3),
        },
        threshold,
    })
}

/// Rectangular area at full resolution (bounding box of a connected dark region).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub min_x: u32,
    pub min_y: u32,
    pub max_x: u32,
    pub max_y: u32,
    /// Number of dark pixels (in the downscaled image) – for sorting by size.
    pub size: u64,
}

/// Finds large dark regions – the whole page, or individual torn-off pieces.
/// Returns at most `max` regions, largest first; those smaller than 5 % of
/// the largest are skipped.
pub fn dark_regions(img: &Gray, max: usize) -> Vec<Region> {
    let factor = (img.width.max(img.height) / 1200).max(1);
    let small = img.downscale(factor);
    let threshold = small.otsu_threshold();
    let mut comps = inner_components(&small, threshold);
    comps.sort_by_key(|c| std::cmp::Reverse(c.count));
    let largest = comps.first().map(|c| c.count).unwrap_or(0);
    comps
        .iter()
        .filter(|c| c.count * 20 >= largest && c.max_x - c.min_x >= 10 && c.max_y - c.min_y >= 10)
        .take(max)
        .map(|c| Region {
            min_x: c.min_x * factor,
            min_y: c.min_y * factor,
            max_x: ((c.max_x + 1) * factor).min(img.width),
            max_y: ((c.max_y + 1) * factor).min(img.height),
            size: c.count,
        })
        .collect()
}

/// Finds the largest connected dark region that does not touch the image border.
fn largest_inner_component(img: &Gray, threshold: u8) -> Option<Component> {
    inner_components(img, threshold)
        .into_iter()
        .max_by_key(|c| c.count)
}

/// All connected dark regions that do not touch the image border.
fn inner_components(img: &Gray, threshold: u8) -> Vec<Component> {
    let (w, h) = (img.width, img.height);
    let dark = |x: u32, y: u32| img.data[(y * w + x) as usize] <= threshold;
    let mut visited = vec![false; (w * h) as usize];
    let mut stack: Vec<(u32, u32)> = Vec::new();
    let mut out: Vec<Component> = Vec::new();

    for sy in 0..h {
        for sx in 0..w {
            let i = (sy * w + sx) as usize;
            if visited[i] || !dark(sx, sy) {
                continue;
            }
            // Depth-first search (8-connectivity) without recursion.
            visited[i] = true;
            stack.push((sx, sy));
            let mut c = Component {
                count: 0,
                min_x: sx,
                min_y: sy,
                max_x: sx,
                max_y: sy,
                ext: [(sx, sy); 4],
            };
            while let Some((x, y)) = stack.pop() {
                c.count += 1;
                c.min_x = c.min_x.min(x);
                c.min_y = c.min_y.min(y);
                c.max_x = c.max_x.max(x);
                c.max_y = c.max_y.max(y);
                let (xi, yi) = (x as i64, y as i64);
                let s = |p: (u32, u32)| (p.0 as i64, p.1 as i64);
                if xi + yi < s(c.ext[0]).0 + s(c.ext[0]).1 {
                    c.ext[0] = (x, y);
                }
                if xi - yi > s(c.ext[1]).0 - s(c.ext[1]).1 {
                    c.ext[1] = (x, y);
                }
                if xi + yi > s(c.ext[2]).0 + s(c.ext[2]).1 {
                    c.ext[2] = (x, y);
                }
                if yi - xi > s(c.ext[3]).1 - s(c.ext[3]).0 {
                    c.ext[3] = (x, y);
                }
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let (nx, ny) = (xi + dx, yi + dy);
                        if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                            continue;
                        }
                        let j = (ny as u32 * w + nx as u32) as usize;
                        if !visited[j] && dark(nx as u32, ny as u32) {
                            visited[j] = true;
                            stack.push((nx as u32, ny as u32));
                        }
                    }
                }
            }
            let touches_edge = c.min_x == 0 || c.min_y == 0 || c.max_x == w - 1 || c.max_y == h - 1;
            // Tiny regions (noise, header letters) are of no interest.
            if !touches_edge && c.count >= 50 {
                out.push(c);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_rectangle_and_ignores_border() {
        let mut g = Gray::new(200, 150, 230);
        // Scanner's black border (touches the edge) – must be ignored.
        for x in 0..200 {
            for y in 0..5 {
                g.set(x, y, 0);
            }
        }
        // Frame 3 px thick from (30,40) to (170,120) (exclusive).
        for y in 40..120 {
            for x in 30..170 {
                if !(33..167).contains(&x) || !(43..117).contains(&y) {
                    g.set(x, y, 10);
                }
            }
        }
        let f = find_frame(&g).unwrap();
        assert_eq!(f.corners.tl, (30.0, 40.0));
        assert_eq!(f.corners.tr, (170.0, 40.0));
        assert_eq!(f.corners.br, (170.0, 120.0));
        assert_eq!(f.corners.bl, (30.0, 120.0));
    }

    #[test]
    fn empty_image() {
        assert!(find_frame(&Gray::new(100, 100, 255)).is_none());
    }
}
