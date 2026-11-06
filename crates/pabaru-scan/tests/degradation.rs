//! Degradation tests: render a page, "print and scan" it with the simulator
//! using various defects and check that the blocks are read.

use pabaru_format::{BLOCK_LEN, Block, BlockHeader, PAYLOAD_LEN};
use pabaru_layout::{DotGrid, Paper, PrintParams};
use pabaru_render::{Bitmap, rasterize};
use pabaru_scan::decode_page;
use pabaru_scan::simulate::{ScanSim, Spot, simulate};

/// Prepares a test page. Returns the bitmap and the number of blocks.
fn test_page(px_per_dot: u32) -> (Bitmap, u32) {
    let params = PrintParams {
        px_per_dot,
        paper: Paper::Custom {
            width_mm: 70.0,
            height_mm: 60.0,
        },
        margin_mm: 4.0,
        header_mm: 2.0,
        ..Default::default()
    };
    let geom = params.geometry();
    let blocks: Vec<[u8; BLOCK_LEN]> = (0..geom.cells())
        .map(|i| {
            let mut payload = [0u8; PAYLOAD_LEN];
            for (j, b) in payload.iter_mut().enumerate() {
                *b = (i as usize * 31 + j * 7) as u8;
            }
            Block {
                header: BlockHeader {
                    backup_id: 0xC0FFEE,
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

/// Runs simulation and decoding; returns (read, total).
fn run(px_per_dot: u32, sim: ScanSim) -> (usize, u32) {
    let (bmp, cells) = test_page(px_per_dot);
    let scan = simulate(&bmp, &sim);
    let page = decode_page(&scan).expect("page must be found");
    // Every block read must be correct (no silent errors).
    let mut seen = std::collections::HashSet::new();
    for c in &page.cells {
        if let Some(b) = &c.block {
            let h = b.block.header;
            assert_eq!(h.backup_id, 0xC0FFEE);
            assert!(h.index < cells, "invalid index");
            assert!(seen.insert(h.index), "block read twice");
            assert_eq!(b.block.payload[1], (h.index as usize * 31 + 7) as u8);
        }
    }
    assert_eq!(page.cols * page.rows, cells, "grid recognised incorrectly");
    (page.ok_count(), cells)
}

fn assert_all(px_per_dot: u32, sim: ScanSim) {
    let (ok, total) = run(px_per_dot, sim.clone());
    assert_eq!(ok as u32, total, "read {ok}/{total} with {sim:?}");
}

#[test]
fn clean_scan() {
    assert_all(3, ScanSim::default());
}

#[test]
fn rotation() {
    for angle in [-5.0, -1.3, 0.7, 3.0, 5.0] {
        assert_all(
            3,
            ScanSim {
                angle_deg: angle,
                ..Default::default()
            },
        );
    }
}

#[test]
fn rotation_by_90_180_270() {
    for angle in [90.0, 180.0, 270.0, 181.5] {
        assert_all(
            3,
            ScanSim {
                angle_deg: angle,
                ..Default::default()
            },
        );
    }
}

#[test]
fn different_scan_resolution() {
    for scale in [1.5, 0.95, 2.0] {
        assert_all(
            3,
            ScanSim {
                scale,
                angle_deg: 1.0,
                ..Default::default()
            },
        );
    }
}

#[test]
fn blur_noise_and_stretch() {
    assert_all(
        3,
        ScanSim {
            angle_deg: -2.0,
            blur_radius: 1,
            noise: 25,
            stretch_y: 1.004,
            ..Default::default()
        },
    );
}

#[test]
fn spots_damage_only_some_blocks() {
    let (ok, total) = run(
        3,
        ScanSim {
            angle_deg: 1.0,
            spots: vec![
                Spot {
                    x: 0.4,
                    y: 0.5,
                    radius: 0.08,
                    value: 20,
                },
                Spot {
                    x: 0.7,
                    y: 0.3,
                    radius: 0.05,
                    value: 240,
                },
            ],
            ..Default::default()
        },
    );
    assert!(ok as u32 >= total * 2 / 3, "read only {ok}/{total}");
    assert!((ok as u32) < total, "a spot should destroy some block");
}

#[test]
fn compact_profile_2px_per_dot() {
    assert_all(
        2,
        ScanSim {
            angle_deg: 0.8,
            ..Default::default()
        },
    );
}

#[test]
fn compact_profile_scan_900dpi() {
    // 300 dpi dots scanned at 900 dpi = 3 px per dot.
    assert_all(
        2,
        ScanSim {
            scale: 1.5,
            angle_deg: -2.5,
            blur_radius: 1,
            noise: 15,
            ..Default::default()
        },
    );
}

/// Full A4 page with default parameters (600 dpi, dots at 200 dpi).
/// It is slow, so it is run only manually: `cargo test --release -- --ignored`.
#[test]
#[ignore]
fn full_a4_page() {
    let params = PrintParams::default();
    let geom = params.geometry();
    let blocks: Vec<[u8; BLOCK_LEN]> = (0..geom.cells())
        .map(|i| {
            Block {
                header: BlockHeader {
                    backup_id: 7,
                    index: i,
                },
                payload: [i as u8; PAYLOAD_LEN],
            }
            .encode()
        })
        .collect();
    let grid = DotGrid::build(geom.cols, geom.rows, &blocks);
    let bmp = rasterize(&geom, &grid, params.dot_fill_pct);
    let sim = ScanSim {
        angle_deg: 1.5,
        blur_radius: 1,
        noise: 20,
        ..Default::default()
    };
    let t0 = std::time::Instant::now();
    let scan = simulate(&bmp, &sim);
    let t1 = std::time::Instant::now();
    let page = decode_page(&scan).unwrap();
    eprintln!(
        "A4: {} blocks, simulation {:?}, decoding {:?}",
        geom.cells(),
        t1 - t0,
        t1.elapsed()
    );
    assert_eq!(page.ok_count() as u32, geom.cells());
}
