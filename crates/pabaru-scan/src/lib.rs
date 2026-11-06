//! # pabaru-scan
//!
//! Decoding of scanned pages.
//!
//! Pipeline ([`decode_page`]):
//! 1. **With frame** – [`frame`] finds the frame corners, [`grid`] the cell
//!    count and the initial grid model.
//! 2. [`lattice`] – every grid node (line intersection) is refined locally,
//!    spreading from the most confident one to its neighbours. This copes
//!    with crumpled paper.
//! 3. [`cells`] – every cell is read bilinearly between its 4 corners.
//! 4. Orientation – all 8 rotations/mirrorings are tried and the one in which
//!    the most blocks pass (RS + CRC) is chosen.
//! 5. **Without frame** – when the first path reads less than 30 % of cells
//!    (damaged or torn-off frame, page fragments), all large dark regions are
//!    found and in each the grid is estimated from its regularity
//!    ([`seed`]). The results are merged.
//!
//! Failed cells are only marked. Missing blocks are recovered by the outer
//! error-correcting code in `pabaru-core`.

pub mod cells;
pub mod frame;
pub mod gray;
pub mod grid;
pub mod homography;
pub mod lattice;
pub mod pdf_input;
pub mod seed;
pub mod simulate;

use std::path::Path;

use pabaru_format::{Block, DecodedBlock};
use thiserror::Error;

pub use gray::Gray;

/// Scan decoding errors.
#[derive(Debug, Error)]
pub enum ScanError {
    #[error("cannot load image: {0}")]
    Image(#[from] image::ImageError),
    #[error("no grid frame found in the image")]
    NoFrame,
    #[error("failed to recognise the block grid")]
    NoGrid,
    #[error("PDF read error: {0}")]
    Pdf(String),
}

/// Result of reading a single cell.
#[derive(Debug, Clone)]
pub struct CellResult {
    /// Cell position on the scan (for the quality map) within region `region`.
    pub col: u32,
    pub row: u32,
    /// Image region (0 = primary; others = torn-off pieces).
    pub region: u32,
    /// Decoded block, or `None` if it could not be corrected.
    pub block: Option<DecodedBlock>,
}

/// Result of reading a whole page.
#[derive(Debug, Clone)]
pub struct PageScan {
    pub cols: u32,
    pub rows: u32,
    /// Chosen orientation (0–7, see [`cells::orient`]).
    pub orientation: u8,
    pub cells: Vec<CellResult>,
}

impl PageScan {
    /// Number of successfully read blocks.
    pub fn ok_count(&self) -> usize {
        self.cells.iter().filter(|c| c.block.is_some()).count()
    }
}

/// Loads an image file (PNG, JPEG, TIFF, BMP) as grayscale.
pub fn load_image(path: &Path) -> Result<Gray, ScanError> {
    let img = image::open(path)?.to_luma8();
    Ok(Gray::from(&img))
}

/// Loads a scan file – an image (1 page) or a PDF (multiple pages).
pub fn load_images(path: &Path) -> Result<Vec<Gray>, ScanError> {
    let is_pdf = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"));
    if is_pdf {
        pdf_input::load_pdf(path)
    } else {
        Ok(vec![load_image(path)?])
    }
}

/// Result for one region (grid) of the image.
struct RegionScan {
    cols: u32,
    rows: u32,
    orientation: u8,
    cells: Vec<CellResult>,
}

impl RegionScan {
    fn ok(&self) -> usize {
        self.cells.iter().filter(|c| c.block.is_some()).count()
    }
}

/// Shifts (in dots) for retrying an unreadable cell.
const RETRY_SHIFTS: [(f64, f64); 4] = [(0.4, 0.0), (-0.4, 0.0), (0.0, 0.4), (0.0, -0.4)];

/// Erasure counts tried when decoding an unreadable cell.
const ERASURE_COUNTS: [usize; 4] = [16, 32, 44, 56];

/// Decodes a sampled cell: first normally, then with the least reliable
/// bytes marked as erasures (RS with erasures corrects up to 2× more bytes).
fn decode_sample(s: &cells::CellSamples, orientation: u8) -> Option<DecodedBlock> {
    let cw = s.codeword(orientation);
    if let Ok(b) = Block::decode(&cw) {
        return Some(b);
    }
    ERASURE_COUNTS.iter().find_map(|&n| {
        let erasures = s.unreliable_bytes(orientation, n);
        Block::decode_with_erasures(&cw, &erasures).ok()
    })
}

/// Retry chain for an unreadable cell with edges `q`
/// (every result is verified by RS + CRC, so extra attempts cannot hurt):
/// 1. adaptive reading – local search for the dot raster (wavy paper),
///    two anchor spacings,
/// 2. straight cell edges (without points along the lines),
/// 3. slightly shifted reading.
fn retry_chain(
    img: &Gray,
    q: &lattice::Patch,
    c: u32,
    r: u32,
    orientation: u8,
) -> Option<DecodedBlock> {
    cells::ANCHOR_STEPS
        .iter()
        .find_map(|&step| {
            let s2 = cells::sample_patch_adaptive(img, q, c, r, step);
            decode_sample(&s2, orientation)
        })
        .or_else(|| {
            let s2 = cells::sample_patch(img, &q.straight(), c, r, (0.0, 0.0));
            decode_sample(&s2, orientation)
        })
        .or_else(|| {
            RETRY_SHIFTS.iter().find_map(|&shift| {
                let s2 = cells::sample_patch(img, q, c, r, shift);
                decode_sample(&s2, orientation)
            })
        })
}

/// Decodes the cells of a refined lattice.
/// `initial` = initial lattice from the global model (frame path only);
/// used as the last fallback – it is very accurate on an undamaged page.
/// `alt` = the same lattice with the boundary measured differently (fallback).
fn decode_lattice(
    img: &Gray,
    lat: &lattice::Lattice,
    initial: Option<&lattice::Lattice>,
    alt: Option<&lattice::Lattice>,
    frame: bool,
) -> RegionScan {
    // Points along the grid lines → cell edges (handles curvature inside a cell).
    let mode = frame.then_some(lattice::BoundaryMode::Exact);
    let edges = lattice::refine_edges(img, lat, mode);
    // Fallback boundary geometry ("step" detector) – only for cells at the frame.
    let alt_edges = alt.map(|a| {
        (
            a,
            lattice::refine_edges(img, a, Some(lattice::BoundaryMode::Step)),
        )
    });
    let quads: Vec<(u32, u32, lattice::Patch)> = (0..lat.rows)
        .flat_map(|r| (0..lat.cols).map(move |c| (c, r)))
        .filter_map(|(c, r)| edges.patch(lat, c, r).map(|q| (c, r, q)))
        .collect();
    let samples: Vec<cells::CellSamples> = quads
        .iter()
        .map(|(c, r, q)| cells::sample_patch(img, q, *c, *r, (0.0, 0.0)))
        .collect();

    // Orientation: try cells (at most 64) until we have 3 successes.
    let mut votes = [0usize; 8];
    let step = (samples.len() / 64).max(1);
    for s in samples.iter().step_by(step) {
        for t in 0..8u8 {
            if Block::decode(&s.codeword(t)).is_ok() {
                votes[t as usize] += 1;
            }
        }
        if votes.iter().sum::<usize>() >= 3 {
            break;
        }
    }
    let orientation = (0..8u8).max_by_key(|t| votes[*t as usize]).unwrap_or(0);
    let found_any = votes.iter().any(|v| *v > 0);

    let cells = quads
        .iter()
        .zip(&samples)
        .map(|((c, r, q), s)| {
            let mut block = decode_sample(s, orientation);
            if block.is_none() && found_any {
                block = retry_chain(img, q, *c, *r, orientation);
            }
            // Cell at the frame: run the whole retry chain again with the fallback
            // boundary geometry ("step" detector – follows a wavy edge better).
            let at_frame = *c == 0 || *r == 0 || *c + 1 == lat.cols || *r + 1 == lat.rows;
            if block.is_none()
                && found_any
                && at_frame
                && let Some((alat, aedges)) = &alt_edges
                && let Some(aq) = aedges.patch(alat, *c, *r)
            {
                block = decode_sample(
                    &cells::sample_patch(img, &aq, *c, *r, (0.0, 0.0)),
                    orientation,
                )
                .or_else(|| retry_chain(img, &aq, *c, *r, orientation));
            }
            // Last attempt: corners from the global model (without local refinement).
            if block.is_none()
                && found_any
                && let Some(init) = initial
                && let Some(corners) = init.cell_corners(*c, *r)
            {
                // Grid of shifts ±1 dot in steps of 0.5 (like the original decoder).
                let straight = lattice::Patch::from_corners(corners);
                block = (-2..=2)
                    .flat_map(|i| (-2..=2).map(move |j| (i as f64 * 0.5, j as f64 * 0.5)))
                    .find_map(|shift| {
                        let s2 = cells::sample_patch(img, &straight, *c, *r, shift);
                        decode_sample(&s2, orientation)
                    });
            }
            CellResult {
                col: *c,
                row: *r,
                region: 0,
                block,
            }
        })
        .collect();
    RegionScan {
        cols: lat.cols,
        rows: lat.rows,
        orientation,
        cells,
    }
}

/// Frame path: global model as the initial estimate + local refinement.
fn decode_with_frame(img: &Gray) -> Option<RegionScan> {
    let frame = frame::find_frame(img)?;
    let model = grid::detect_grid(img, &frame)?;
    let initial = lattice::Lattice::from_model(&model);
    let mut tracked = initial.clone();
    lattice::track(img, &mut tracked);
    // Two boundary variants: exact (primary) and robust (fallback).
    let mut exact = tracked.clone();
    lattice::refine_frame_boundary(img, &mut exact, lattice::BoundaryMode::Exact);
    let mut robust = tracked;
    lattice::refine_frame_boundary(img, &mut robust, lattice::BoundaryMode::Step);
    Some(decode_lattice(
        img,
        &exact,
        Some(&initial),
        Some(&robust),
        true,
    ))
}

/// Frameless path: each large dark region separately.
fn decode_regions(img: &Gray) -> Vec<RegionScan> {
    let threshold = img.otsu_threshold();
    frame::dark_regions(img, 4)
        .iter()
        .filter_map(|r| seed::lattice_for(img, r, threshold))
        .map(|mut lat| {
            lattice::track(img, &mut lat);
            decode_lattice(img, &lat, None, None, false)
        })
        .collect()
}

/// Decodes one scanned page (even a crumpled or torn one).
pub fn decode_page(img: &Gray) -> Result<PageScan, ScanError> {
    let mut regions: Vec<RegionScan> = Vec::new();
    let mut good_enough = false;
    if let Some(r) = decode_with_frame(img) {
        // The frameless path is slow and only helps with a destroyed frame or a
        // torn-off piece – it runs when the frame path reads less than 30 %.
        good_enough = !r.cells.is_empty() && r.ok() * 10 >= r.cells.len() * 3;
        regions.push(r);
    }
    if !good_enough {
        regions.extend(decode_regions(img));
    }
    if regions.is_empty() {
        return Err(ScanError::NoFrame);
    }
    // Primary region = the one with the most blocks read (for the quality map).
    regions.sort_by_key(|r| std::cmp::Reverse(r.ok()));

    // Merge: each data block (by index) only once – the one with the fewest corrections.
    let mut best: std::collections::HashMap<u32, (usize, usize, usize)> =
        std::collections::HashMap::new();
    for (ri, r) in regions.iter().enumerate() {
        for (ci, c) in r.cells.iter().enumerate() {
            let Some(b) = &c.block else { continue };
            if b.block.header.is_superblock() {
                continue;
            }
            let entry = best
                .entry(b.block.header.index)
                .or_insert((ri, ci, b.corrected_bytes));
            if b.corrected_bytes < entry.2 {
                *entry = (ri, ci, b.corrected_bytes);
            }
        }
    }
    let primary = &regions[0];
    let mut cells = Vec::new();
    for (ri, r) in regions.iter().enumerate() {
        for (ci, c) in r.cells.iter().enumerate() {
            let keep = match &c.block {
                // Unreadable cells are shown only from the primary region (quality map).
                None => ri == 0,
                Some(b) if b.block.header.is_superblock() => true,
                Some(b) => best
                    .get(&b.block.header.index)
                    .is_some_and(|&(bri, bci, _)| (bri, bci) == (ri, ci)),
            };
            if keep {
                cells.push(CellResult {
                    region: ri as u32,
                    ..c.clone()
                });
            }
        }
    }
    Ok(PageScan {
        cols: primary.cols,
        rows: primary.rows,
        orientation: primary.orientation,
        cells,
    })
}
