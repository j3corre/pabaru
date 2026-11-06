//! Robustness against crumpled and torn paper.
//!
//! A page is rendered, the simulator "crumples" it (smooth waviness + sharp
//! shaded creases) or tears it, and the decoder must read enough blocks.

use pabaru_format::{BLOCK_LEN, Block, BlockHeader, PAYLOAD_LEN};
use pabaru_layout::{DotGrid, Paper, PrintParams};
use pabaru_render::{Bitmap, rasterize};
use pabaru_scan::simulate::{Crumple, ScanSim, Tear, simulate, simulate_torn_pieces};
use pabaru_scan::{Gray, decode_page};

const ID: u32 = 0xBEEF;

/// A 120 × 100 mm page with the given pixels per dot (600 dpi printer).
fn page(px_per_dot: u32) -> (Bitmap, u32) {
    let params = PrintParams {
        px_per_dot,
        paper: Paper::Custom {
            width_mm: 120.0,
            height_mm: 100.0,
        },
        margin_mm: 6.0,
        header_mm: 2.0,
        ..Default::default()
    };
    let geom = params.geometry();
    let blocks: Vec<[u8; BLOCK_LEN]> = (0..geom.cells())
        .map(|i| {
            let mut payload = [0u8; PAYLOAD_LEN];
            for (j, b) in payload.iter_mut().enumerate() {
                *b = (i as usize * 13 + j * 3) as u8;
            }
            Block {
                header: BlockHeader {
                    backup_id: ID,
                    index: i,
                },
                payload,
            }
            .encode()
        })
        .collect();
    let grid = DotGrid::build(geom.cols, geom.rows, &blocks);
    (rasterize(&geom, &grid, params.dot_fill_pct), geom.cells())
}

/// Reads the images and returns the number of distinct correctly read blocks.
/// Every block read must be correct (no silent errors).
fn read(images: &[Gray], total: u32) -> usize {
    let mut seen = std::collections::HashSet::new();
    for img in images {
        let Ok(scan) = decode_page(img) else { continue };
        for c in scan.cells.iter().filter_map(|c| c.block.as_ref()) {
            let h = c.block.header;
            assert_eq!(h.backup_id, ID);
            assert!(h.index < total);
            assert_eq!(c.block.payload[1], (h.index as usize * 13 + 3) as u8);
            seen.insert(h.index);
        }
    }
    seen.len()
}

fn crumpled(ppd: u32, c: Crumple, seed: u64) -> (usize, u32) {
    let (bmp, total) = page(ppd);
    let sim = ScanSim {
        crumple: Some(c),
        angle_deg: 1.2,
        noise: 10,
        seed,
        ..Default::default()
    };
    (read(&[simulate(&bmp, &sim)], total), total)
}

/// Diagnostics: success-rate table (run in release).
#[test]
#[ignore]
fn diagnostics_matrix() {
    for ppd in [3u32, 6, 12] {
        for (name, c) in [("medium", Crumple::MEDIUM), ("strong", Crumple::STRONG)] {
            let r: Vec<String> = (1..=3)
                .map(|seed| {
                    let (ok, total) = crumpled(ppd, c, seed);
                    format!("{ok}/{total}")
                })
                .collect();
            eprintln!("{} dpi, {name}: {}", 600 / ppd, r.join("  "));
        }
    }
    let (bmp, total) = page(6);
    let sim = ScanSim {
        tear: Some(Tear::default()),
        ..Default::default()
    };
    eprintln!(
        "tear (one image): {}/{total}",
        read(&[simulate(&bmp, &sim)], total)
    );
    eprintln!(
        "tear (two images): {}/{total}",
        read(&simulate_torn_pieces(&bmp, &sim), total)
    );
}

/// Saves sample images (PABARU_SNAPSHOTS=directory).
#[test]
#[ignore]
fn sample_images() {
    let Ok(dir) = std::env::var("PABARU_SNAPSHOTS") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let save = |g: &Gray, name: &str| {
        image::GrayImage::from_raw(g.width, g.height, g.data.clone())
            .unwrap()
            .save(dir.join(name))
            .unwrap();
    };
    let (bmp, _) = page(6);
    let sim = ScanSim {
        crumple: Some(Crumple::MEDIUM),
        angle_deg: 1.2,
        noise: 10,
        ..Default::default()
    };
    save(&simulate(&bmp, &sim), "crumpled-100dpi.png");
    let sim = ScanSim {
        tear: Some(Tear::default()),
        ..Default::default()
    };
    save(&simulate(&bmp, &sim), "torn.png");
}

/// Overlay of the detected grid on the scan (PABARU_SCAN, PABARU_SNAPSHOTS).
#[test]
#[ignore]
fn debug_overlay() {
    let (Ok(path), Ok(out)) = (
        std::env::var("PABARU_SCAN"),
        std::env::var("PABARU_SNAPSHOTS"),
    ) else {
        return;
    };
    let img = &pabaru_scan::load_images(std::path::Path::new(&path)).unwrap()[0];
    let f = pabaru_scan::frame::find_frame(img).unwrap();
    let m = pabaru_scan::grid::detect_grid(img, &f).unwrap();
    let mut lat = pabaru_scan::lattice::Lattice::from_model(&m);
    let init = lat.clone();
    pabaru_scan::lattice::track(img, &mut lat);
    pabaru_scan::lattice::refine_frame_boundary(
        img,
        &mut lat,
        pabaru_scan::lattice::BoundaryMode::Exact,
    );
    let edges = pabaru_scan::lattice::refine_edges(
        img,
        &lat,
        Some(pabaru_scan::lattice::BoundaryMode::Exact),
    );
    let mut rgb = image::RgbImage::from_fn(img.width, img.height, |x, y| {
        let v = img.data[(y * img.width + x) as usize];
        image::Rgb([v, v, v])
    });
    let mut dot = |p: (f64, f64), c: [u8; 3], r: i64| {
        for dy in -r..=r {
            for dx in -r..=r {
                let (x, y) = (p.0 as i64 + dx, p.1 as i64 + dy);
                if x >= 0 && y >= 0 && (x as u32) < rgb.width() && (y as u32) < rgb.height() {
                    rgb.put_pixel(x as u32, y as u32, image::Rgb(c));
                }
            }
        }
    };
    for n in init.nodes.iter().flatten() {
        dot(*n, [0, 160, 0], 4);
    }
    for r in 0..lat.rows {
        for c in 0..lat.cols {
            let p = edges.patch(&lat, c, r).unwrap();
            for q in p.top.iter().chain(&p.left) {
                dot(*q, [0, 0, 255], 3);
            }
        }
    }
    for (c, r) in [(0u32, 0u32), (1, 0), (0, 1), (1, 1)] {
        let p = edges.patch(&lat, c, r).unwrap();
        for y in 0..40 {
            for x in 0..40 {
                let q = p.point((x as f64 + 1.0) / 41.0, (y as f64 + 1.0) / 41.0);
                dot(q, [255, 140, 0], 1);
            }
        }
    }
    for n in lat.nodes.iter().flatten() {
        dot(*n, [255, 0, 0], 5);
    }
    let name = std::path::Path::new(&path)
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .to_string();
    let small = image::imageops::resize(
        &rgb,
        img.width / 3,
        img.height / 3,
        image::imageops::FilterType::Triangle,
    );
    small
        .save(std::path::Path::new(&out).join(format!("{name}-overlay.png")))
        .unwrap();
    // Detail of the top-left corner at full resolution.
    let crop = image::imageops::crop_imm(
        &rgb,
        (f.corners.tl.0 as u32).saturating_sub(50),
        (f.corners.tl.1 as u32).saturating_sub(50),
        1400,
        1000,
    )
    .to_image();
    crop.save(std::path::Path::new(&out).join(format!("{name}-detail.png")))
        .unwrap();
}

// ---------------------------------------------------------------------------
// Regular tests (thresholds taken from diagnostics, with a margin).
// ---------------------------------------------------------------------------

#[test]
fn crumple_medium_100dpi() {
    let (ok, total) = crumpled(6, Crumple::MEDIUM, 1);
    assert!(ok * 10 >= total as usize * 6, "read only {ok}/{total}");
}

#[test]
fn tear_single_image() {
    let (bmp, total) = page(6);
    let sim = ScanSim {
        tear: Some(Tear::default()),
        ..Default::default()
    };
    let ok = read(&[simulate(&bmp, &sim)], total);
    assert!(ok * 10 >= total as usize * 6, "read only {ok}/{total}");
}

#[test]
fn tear_two_pieces_separately() {
    let (bmp, total) = page(6);
    let sim = ScanSim {
        tear: Some(Tear::default()),
        ..Default::default()
    };
    let ok = read(&simulate_torn_pieces(&bmp, &sim), total);
    assert!(ok * 10 >= total as usize * 6, "read only {ok}/{total}");
}

/// Debugging the frame boundary on a file (PABARU_SCAN).
#[test]
#[ignore]
fn debug_frame() {
    let Ok(path) = std::env::var("PABARU_SCAN") else {
        return;
    };
    let img = &pabaru_scan::load_images(std::path::Path::new(&path)).unwrap()[0];
    let f = pabaru_scan::frame::find_frame(img).unwrap();
    let m = pabaru_scan::grid::detect_grid(img, &f).unwrap();
    let init = pabaru_scan::lattice::Lattice::from_model(&m);
    let mut lat = init.clone();
    pabaru_scan::lattice::track(img, &mut lat);
    let before = lat.clone();
    pabaru_scan::lattice::refine_frame_boundary(
        img,
        &mut lat,
        pabaru_scan::lattice::BoundaryMode::Exact,
    );
    for (i, j) in [
        (0u32, 3u32),
        (0, 6),
        (3, 0),
        (5, 0),
        (lat.cols, 4),
        (4, lat.rows),
    ] {
        let k = lat.idx(i, j);
        let (a, b, t) = (
            before.nodes[k].unwrap(),
            lat.nodes[k].unwrap(),
            init.nodes[k].unwrap(),
        );
        eprintln!(
            "node {i},{j}: model {:?} → after boundary {:?} (shift {:+.1},{:+.1} px)",
            t,
            b,
            b.0 - t.0,
            b.1 - t.1
        );
        let _ = a;
    }
}
