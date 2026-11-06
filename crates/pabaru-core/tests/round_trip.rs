//! Integration tests of the whole pipeline: files → pages → (simulated) scan → files.

use std::fs;
use std::path::{Path, PathBuf};

use pabaru_core::{
    CoreError, Decoder, Destination, EncodeOptions, KdfConfig, Paper, PrintParams, ScanSim,
    VerifyOptions, encode, verify,
};

/// Small page so that the tests run fast.
fn small_params() -> PrintParams {
    PrintParams {
        paper: Paper::Custom {
            width_mm: 70.0,
            height_mm: 60.0,
        },
        margin_mm: 4.0,
        header_mm: 2.0,
        ..Default::default()
    }
}

/// Fast Argon2 parameters for tests.
const FAST_KDF: KdfConfig = KdfConfig {
    mem_kib: 256,
    iterations: 1,
    parallelism: 1,
};

/// Creates a test folder with a few files. Random (incompressible)
/// data makes the backup span several pages.
fn make_input(root: &Path, random_bytes: usize) -> PathBuf {
    let dir = root.join("documents");
    fs::create_dir_all(dir.join("sub")).unwrap();
    // Text with Czech diacritics – checks that UTF-8 content survives the round trip.
    fs::write(
        dir.join("readme.txt"),
        "Záloha na papír – test.\n".repeat(20),
    )
    .unwrap();
    let mut state = 0x1234_5678u32;
    let data: Vec<u8> = (0..random_bytes)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state as u8
        })
        .collect();
    fs::write(dir.join("sub/data.bin"), data).unwrap();
    dir
}

fn opts() -> EncodeOptions {
    EncodeOptions {
        params: small_params(),
        kdf: FAST_KDF,
        ..Default::default()
    }
}

#[test]
fn round_trip_without_simulation() {
    let src = tempfile::tempdir().unwrap();
    let input = make_input(src.path(), 20_000);
    let backup = encode(std::slice::from_ref(&input), &opts()).unwrap();
    assert!(
        backup.page_count() >= 2,
        "the test must cover several pages"
    );

    let mut dec = Decoder::new();
    // Pages in reverse order – the order must not matter.
    for i in (0..backup.page_count()).rev() {
        let bmp = backup.render_page(i);
        let scan = pabaru_core::ScanSim::default();
        let img = pabaru_scan_sim(&bmp, &scan);
        dec.add_image(&img).unwrap();
    }
    let p = dec.progress().unwrap();
    assert!(p.can_restore());
    // The input folder name must not be stored on paper.
    assert_eq!(p.name, "");

    let out = tempfile::tempdir().unwrap();
    let report = dec.restore(None, Destination::Folder(out.path())).unwrap();
    assert_eq!(report.unpack.verified_files, 2);
    assert_eq!(
        fs::read(out.path().join("documents/sub/data.bin")).unwrap(),
        fs::read(input.join("sub/data.bin")).unwrap()
    );
}

/// Helper – scan simulation from `pabaru-scan`.
fn pabaru_scan_sim(bmp: &pabaru_render::Bitmap, sim: &ScanSim) -> pabaru_core::Gray {
    pabaru_scan::simulate::simulate(bmp, sim)
}

#[test]
fn encryption_and_wrong_password() {
    let src = tempfile::tempdir().unwrap();
    let input = make_input(src.path(), 500);
    let o = EncodeOptions {
        password: Some("secret".into()),
        ..opts()
    };
    let backup = encode(&[input], &o).unwrap();
    assert!(backup.info.encrypted);

    let mut dec = Decoder::new();
    for i in 0..backup.page_count() {
        dec.add_image(&pabaru_scan_sim(
            &backup.render_page(i),
            &ScanSim::default(),
        ))
        .unwrap();
    }
    assert!(dec.needs_password());
    let out = tempfile::tempdir().unwrap();
    assert!(matches!(
        dec.restore(None, Destination::Folder(out.path())),
        Err(CoreError::PasswordRequired)
    ));
    assert!(matches!(
        dec.restore(Some("wrong"), Destination::Folder(out.path())),
        Err(CoreError::Crypto(_))
    ));
    let ok = dec
        .restore(Some("secret"), Destination::Folder(out.path()))
        .unwrap();
    assert_eq!(ok.unpack.verified_files, 2);
}

#[test]
fn verify_with_page_loss_and_degradation() {
    let src = tempfile::tempdir().unwrap();
    let input = make_input(src.path(), 20_000);
    let o = EncodeOptions {
        survive_page_loss: true,
        ..opts()
    };
    let vopts = VerifyOptions {
        sim: ScanSim {
            angle_deg: 1.5,
            blur_radius: 1,
            noise: 15,
            ..Default::default()
        },
        drop_pages: vec![1],
    };
    let r = verify(&[input], &o, &vopts).unwrap();
    assert!(r.info.survives_page_loss());
    assert_eq!(r.pages_scanned as u32, r.info.pages - 1);
    assert!(
        r.recovered_shards > 0,
        "the lost page had to be reconstructed"
    );
    assert_eq!(r.verified_files, 2);
}

#[test]
fn page_loss_without_parity_reserve_fails() {
    let src = tempfile::tempdir().unwrap();
    let input = make_input(src.path(), 20_000);
    let o = EncodeOptions {
        redundancy_pct: 0,
        ..opts()
    };
    let vopts = VerifyOptions {
        drop_pages: vec![0],
        ..Default::default()
    };
    match verify(&[input], &o, &vopts) {
        Err(CoreError::NotEnoughBlocks { .. }) => {}
        other => panic!("expected a not-enough-blocks error, got {other:?}"),
    }
}

#[test]
fn pdf_and_png_export() {
    let src = tempfile::tempdir().unwrap();
    let input = make_input(src.path(), 3000);
    let backup = encode(&[input], &opts()).unwrap();
    let out = tempfile::tempdir().unwrap();

    // Privacy: the printed header must not reveal names of backed-up files.
    for i in 0..backup.page_count() {
        let text = backup.header_lines(i).join("\n");
        assert!(
            !text.contains("documents") && !text.contains("readme"),
            "{text}"
        );
        assert!(text.starts_with("Pabaru paper backup"), "{text}");
    }

    let pdf = out.path().join("backup.pdf");
    backup.write_pdf(&pdf).unwrap();
    let bytes = fs::read(&pdf).unwrap();
    assert!(bytes.starts_with(b"%PDF-1.4"));
    assert!(bytes.ends_with(b"%%EOF\n"));

    // The PNGs must load and decode directly (without simulation).
    let pngs = backup.write_pngs(out.path(), "backup").unwrap();
    assert_eq!(pngs.len(), backup.page_count());
    let mut dec = Decoder::new();
    for p in &pngs {
        dec.add_image(&pabaru_core::load_image(p).unwrap()).unwrap();
    }
    assert!(dec.progress().unwrap().can_restore());
}

#[test]
fn password_check_without_writing() {
    let src = tempfile::tempdir().unwrap();
    let input = make_input(src.path(), 300);
    // Passwords with diacritics ("correct password" / "wrong") check UTF-8 handling.
    let o = EncodeOptions {
        password: Some("správné heslo".into()),
        ..opts()
    };
    let backup = encode(&[input], &o).unwrap();
    let mut dec = Decoder::new();
    for i in 0..backup.page_count() {
        dec.add_image(&pabaru_scan_sim(
            &backup.render_page(i),
            &ScanSim::default(),
        ))
        .unwrap();
    }
    assert!(dec.progress().unwrap().encrypted);
    assert!(matches!(
        dec.check_password("špatné"),
        Err(CoreError::Crypto(_))
    ));
    assert!(dec.check_password("správné heslo").is_ok());
}

/// Typical scanner app output: each page as a JPEG inside a PDF.
#[test]
fn restore_from_pdf_with_jpeg_scans() {
    use lopdf::{Document, Object, Stream, dictionary};

    let src = tempfile::tempdir().unwrap();
    let input = make_input(src.path(), 6000);
    let backup = encode(std::slice::from_ref(&input), &opts()).unwrap();

    // "Scan" of each page: simulation + JPEG quality 85.
    let mut doc = Document::with_version("1.5");
    let pages_id = doc.new_object_id();
    let mut kids = Vec::new();
    for i in 0..backup.page_count() {
        let sim = ScanSim {
            angle_deg: 0.8,
            noise: 8,
            ..Default::default()
        };
        let g = pabaru_scan_sim(&backup.render_page(i), &sim);
        let mut jpeg = Vec::new();
        let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85);
        image::ImageEncoder::write_image(
            enc,
            &g.data,
            g.width,
            g.height,
            image::ExtendedColorType::L8,
        )
        .unwrap();
        let img_id = doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject", "Subtype" => "Image",
                "Width" => g.width as i64, "Height" => g.height as i64,
                "ColorSpace" => "DeviceGray", "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            jpeg,
        ));
        let content_id = doc.add_object(Stream::new(
            dictionary! {},
            b"q 200 0 0 170 0 0 cm /Im0 Do Q".to_vec(),
        ));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => pages_id, "Contents" => content_id,
            "MediaBox" => vec![0.into(), 0.into(), 200.into(), 170.into()],
            "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => img_id } },
        });
        kids.push(Object::from(page_id));
    }
    let count = kids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog_id = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    doc.trailer.set("Root", catalog_id);
    let pdf = src.path().join("scan.pdf");
    doc.save(&pdf).unwrap();

    let mut dec = Decoder::new();
    for img in pabaru_core::load_images(&pdf).unwrap() {
        dec.add_image(&img).unwrap();
    }
    let out = tempfile::tempdir().unwrap();
    let r = dec.restore(None, Destination::Folder(out.path())).unwrap();
    assert_eq!(r.unpack.verified_files, 2);
}

/// Trustworthiness: a generated page (directly and via PDF) must be read
/// **without a single correction** – at all dot densities. Corrections on a clean
/// source would indicate a measurement error and would eat into the margin for real
/// paper damage.
#[test]
fn generated_page_without_a_single_correction() {
    let src = tempfile::tempdir().unwrap();
    let input = make_input(src.path(), 4000);
    // 600 dpi printer: 2/3/4/6/12 px per dot = 300/200/150/100/50 dpi.
    for px_per_dot in [2u32, 3, 4, 6, 12] {
        let o = EncodeOptions {
            params: PrintParams {
                px_per_dot,
                paper: Paper::Custom {
                    width_mm: 150.0,
                    height_mm: 180.0,
                },
                margin_mm: 6.0,
                header_mm: 4.0,
                ..Default::default()
            },
            kdf: FAST_KDF,
            ..Default::default()
        };
        let backup = encode(std::slice::from_ref(&input), &o).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let pdf = dir.path().join("z.pdf");
        backup.write_pdf(&pdf).unwrap();
        for img in pabaru_core::load_images(&pdf).unwrap() {
            let scan = pabaru_core::decode_page(&img).unwrap();
            let total = scan.cells.len();
            let corrected: Vec<usize> = scan
                .cells
                .iter()
                .filter_map(|c| c.block.as_ref().map(|b| b.corrected_bytes))
                .collect();
            assert_eq!(corrected.len(), total, "{px_per_dot} px/dot: unread blocks");
            assert!(
                corrected.iter().all(|&n| n == 0),
                "{px_per_dot} px/dot: corrections on a clean source: {:?}",
                scan.cells
                    .iter()
                    .filter_map(
                        |c| c.block.as_ref().filter(|b| b.corrected_bytes > 0).map(|b| (
                            c.col,
                            c.row,
                            b.corrected_bytes
                        ))
                    )
                    .collect::<Vec<_>>()
            );
        }
    }
}
