//! Debugging tool: compares a real scan with the original PDF.
//!
//! PABARU_TRUTH = generated PDF (original), PABARU_SCAN = scan of the same backup.
//! For every cell of the scan it finds the closest correct block and prints the
//! number of wrong bits and a map of where inside the cell the errors occur.

use std::path::Path;

use pabaru_format::{BLOCK_DOTS, BLOCK_LEN, Block, dots};
use pabaru_scan::{cells, frame, grid, lattice};

/// Correct codeword (200 B) of every block of the original.
fn truth(path: &str) -> Vec<[u8; BLOCK_LEN]> {
    let img = &pabaru_scan::load_images(Path::new(path)).unwrap()[0];
    let scan = pabaru_scan::decode_page(img).unwrap();
    scan.cells
        .iter()
        .filter_map(|c| c.block.as_ref())
        .map(|b| b.block.encode())
        .collect()
}

#[test]
#[ignore]
fn compare_with_original() {
    let (Ok(orig), Ok(scan_path)) = (std::env::var("PABARU_TRUTH"), std::env::var("PABARU_SCAN"))
    else {
        return;
    };
    let truth = truth(&orig);
    eprintln!("original: {} blocks", truth.len());
    let img = &pabaru_scan::load_images(Path::new(&scan_path)).unwrap()[0];
    let f = frame::find_frame(img).unwrap();
    let m = grid::detect_grid(img, &f).unwrap();
    let mut lat = lattice::Lattice::from_model(&m);
    lattice::track(img, &mut lat);
    lattice::refine_frame_boundary(img, &mut lat, pabaru_scan::lattice::BoundaryMode::Exact);
    let edges = lattice::refine_edges(img, &lat, Some(pabaru_scan::lattice::BoundaryMode::Exact));

    // Error frequency by dot position within the cell (8×8 grid of 5×5-dot areas).
    let mut heat = [[0usize; 8]; 8];
    let mut per_cell = Vec::new();
    for r in 0..lat.rows {
        for c in 0..lat.cols {
            let Some(p) = edges.patch(&lat, c, r) else {
                continue;
            };
            let s = if std::env::var_os("PABARU_PLAIN").is_some() {
                cells::sample_patch(img, &p, c, r, (0.0, 0.0))
            } else {
                cells::sample_patch_adaptive(img, &p, c, r, 4)
            };
            // Best match over all orientations and blocks.
            let mut best = (usize::MAX, 0u8, 0usize);
            for t in 0..8u8 {
                let cw = s.codeword(t);
                for (i, tr) in truth.iter().enumerate() {
                    let bits: usize = cw
                        .iter()
                        .zip(tr)
                        .map(|(a, b)| (a ^ b).count_ones() as usize)
                        .sum();
                    if bits < best.0 {
                        best = (bits, t, i);
                    }
                }
            }
            let (bits, t, i) = best;
            let cw = s.codeword(t);
            let bytes = cw.iter().zip(&truth[i]).filter(|(a, b)| a != b).count();
            let ok = Block::decode(&cw).is_ok();
            per_cell.push((c, r, bits, bytes, ok));
            // Errors by position (in the original's orientation); optionally only column PABARU_COLUMN.
            let in_col =
                std::env::var("PABARU_COLUMN").map_or(true, |v| v.parse::<u32>().ok() == Some(c));
            for y in 0..BLOCK_DOTS {
                for x in 0..BLOCK_DOTS {
                    if in_col && dots::dot_at(&cw, x, y) != dots::dot_at(&truth[i], x, y) {
                        heat[y / 5][x / 5] += 1;
                    }
                }
            }
        }
    }
    eprintln!("cell: wrong bits / wrong bytes (RS corrects ≤ 30 bytes)");
    for r in 0..lat.rows {
        let line: String = per_cell
            .iter()
            .filter(|x| x.1 == r)
            .map(|x| format!("{:4}/{:3}{} ", x.2, x.3, if x.4 { " " } else { "!" }))
            .collect();
        eprintln!("  {line}");
    }
    eprintln!("error map inside a cell (sum over all cells, 8×8 areas):");
    for row in heat {
        eprintln!(
            "  {}",
            row.iter().map(|v| format!("{v:5}")).collect::<String>()
        );
    }
}

/// Actual displacements inside selected cells (relative to the original).
#[test]
#[ignore]
fn actual_displacements() {
    let (Ok(orig), Ok(scan_path)) = (std::env::var("PABARU_TRUTH"), std::env::var("PABARU_SCAN"))
    else {
        return;
    };
    let truth = truth(&orig);
    let img = &pabaru_scan::load_images(Path::new(&scan_path)).unwrap()[0];
    let f = frame::find_frame(img).unwrap();
    let m = grid::detect_grid(img, &f).unwrap();
    let mut lat = lattice::Lattice::from_model(&m);
    lattice::track(img, &mut lat);
    lattice::refine_frame_boundary(img, &mut lat, pabaru_scan::lattice::BoundaryMode::Exact);
    let edges = lattice::refine_edges(img, &lat, Some(pabaru_scan::lattice::BoundaryMode::Exact));
    for (c, r) in [(2u32, 1u32), (3, 1), (0, 2), (2, 8)] {
        let p = edges.patch(&lat, c, r).unwrap();
        // Block identification: best match of the adaptive reading.
        let s = cells::sample_patch_adaptive(img, &p, c, r, 4);
        let mut best = (usize::MAX, 0usize);
        for (i, tr) in truth.iter().enumerate() {
            let cw = s.codeword(0);
            let bits: usize = cw
                .iter()
                .zip(tr)
                .map(|(a, b)| (a ^ b).count_ones() as usize)
                .sum();
            if bits < best.0 {
                best = (bits, i);
            }
        }
        let tr = &truth[best.1];
        let cell = 41.0;
        let k = p.corners();
        let u = ((k[1].0 - k[0].0) / cell, (k[1].1 - k[0].1) / cell);
        let v = ((k[2].0 - k[0].0) / cell, (k[2].1 - k[0].1) / cell);
        eprintln!(
            "cell {c},{r} ({} errors): actual anchor shift [u v] in dots",
            best.0
        );
        for ay in (2usize..40).step_by(6) {
            let mut line = String::new();
            for ax in (2usize..40).step_by(6) {
                let mut bestoff = (f64::NEG_INFINITY, 0.0, 0.0);
                for ia in -15..=15 {
                    for ib in -15..=15 {
                        let (a, b) = (ia as f64 * 0.1, ib as f64 * 0.1);
                        let mut sc = 0.0;
                        for y in ay.saturating_sub(3)..(ay + 4).min(40) {
                            for x in ax.saturating_sub(3)..(ax + 4).min(40) {
                                let q = p.point((x as f64 + 1.0) / cell, (y as f64 + 1.0) / cell);
                                let q = (q.0 + u.0 * a + v.0 * b, q.1 + u.1 * a + v.1 * b);
                                let d = 255.0 - img.sample(q.0, q.1);
                                let bit = dots::dot_at(tr, x, y);
                                sc += if bit { d } else { -d };
                            }
                        }
                        if sc > bestoff.0 {
                            bestoff = (sc, a, b);
                        }
                    }
                }
                line.push_str(&format!(" [{:+.1} {:+.1}]", bestoff.1, bestoff.2));
            }
            eprintln!("  {line}");
        }
    }
}
