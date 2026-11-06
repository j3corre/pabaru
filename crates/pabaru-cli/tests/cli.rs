//! Command-line integration tests (run the real `pabaru` binary).

use std::fs;

use assert_cmd::Command;
use predicates::str::contains;

/// Small paper so that the tests are fast.
const SMALL: [&str; 4] = ["--paper", "70x60", "--margin", "4"];

fn pabaru() -> Command {
    Command::cargo_bin("pabaru").unwrap()
}

fn make_input(root: &std::path::Path) -> std::path::PathBuf {
    let dir = root.join("data");
    fs::create_dir_all(dir.join("sub")).unwrap();
    // Text with Czech diacritics – checks that UTF-8 content survives the round trip.
    fs::write(dir.join("a.txt"), "Ahoj z papíru!\n".repeat(50)).unwrap();
    let noise: Vec<u8> = (0..4000u32)
        .map(|i| (i.wrapping_mul(2654435761) >> 13) as u8)
        .collect();
    fs::write(dir.join("sub/b.bin"), noise).unwrap();
    dir
}

#[test]
fn encode_png_and_decode() {
    let tmp = tempfile::tempdir().unwrap();
    let input = make_input(tmp.path());
    let pages = tmp.path().join("pages");

    pabaru()
        .arg("encode")
        .arg(&input)
        .arg("-o")
        .arg(&pages)
        .args(SMALL)
        .assert()
        .success()
        .stdout(contains("Pages:"))
        // Input names are not stored, so the backup has no name.
        .stdout(contains("(unnamed)"));

    let mut scans: Vec<_> = fs::read_dir(&pages)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    scans.sort();
    assert!(!scans.is_empty());

    let out = tmp.path().join("restored");
    pabaru()
        .arg("decode")
        .args(&scans)
        .arg("-o")
        .arg(&out)
        .assert()
        .success()
        .stdout(contains("Restored and verified 2 files"));
    assert_eq!(
        fs::read(out.join("data/sub/b.bin")).unwrap(),
        fs::read(input.join("sub/b.bin")).unwrap()
    );

    // inspect prints the quality map
    pabaru()
        .arg("inspect")
        .arg(&scans[0])
        .assert()
        .success()
        .stdout(contains("Quality map"));
}

#[test]
fn encode_pdf() {
    let tmp = tempfile::tempdir().unwrap();
    let input = make_input(tmp.path());
    let pdf = tmp.path().join("backup.pdf");
    pabaru()
        .arg("encode")
        .arg(&input)
        .arg("-o")
        .arg(&pdf)
        .args(SMALL)
        .assert()
        .success();
    assert!(fs::read(&pdf).unwrap().starts_with(b"%PDF"));
}

#[test]
fn encryption_via_env_variable() {
    let tmp = tempfile::tempdir().unwrap();
    let input = make_input(tmp.path());
    let pages = tmp.path().join("pages");
    pabaru()
        .env("PABARU_PW", "password123")
        .arg("encode")
        .arg(&input)
        .arg("-o")
        .arg(&pages)
        .args(["--password-env", "PABARU_PW"])
        .args(SMALL)
        .assert()
        .success()
        .stdout(contains("encrypted"))
        .stdout(contains("the password is not printed on paper"));

    let scans: Vec<_> = fs::read_dir(&pages)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    let out = tmp.path().join("restored");
    pabaru()
        .env("PABARU_PW", "wrong")
        .arg("decode")
        .args(&scans)
        .arg("-o")
        .arg(&out)
        .args(["--password-env", "PABARU_PW"])
        .assert()
        .failure()
        .stdout(contains("The backup is encrypted"))
        .stderr(contains("wrong password"));
    pabaru()
        .env("PABARU_PW", "password123")
        .arg("decode")
        .args(&scans)
        .arg("-o")
        .arg(&out)
        .args(["--password-env", "PABARU_PW"])
        .assert()
        .success();
}

#[test]
fn decode_without_output_restores_next_to_scans() {
    let tmp = tempfile::tempdir().unwrap();
    let input = make_input(tmp.path());
    let pages = tmp.path().join("pages");
    pabaru()
        .arg("encode")
        .arg(&input)
        .arg("-o")
        .arg(&pages)
        .args(SMALL)
        .assert()
        .success();
    let scans: Vec<_> = fs::read_dir(&pages)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    pabaru().arg("decode").args(&scans).assert().success();
    // The root folder is recreated inside the scans' folder.
    let name = input.file_name().unwrap();
    assert!(pages.join(name).join("a.txt").exists());
}

#[test]
fn verify_with_page_loss() {
    let tmp = tempfile::tempdir().unwrap();
    let input = make_input(tmp.path());
    pabaru()
        .arg("verify")
        .arg(&input)
        .args(SMALL)
        .args(["--survive-page-loss", "--drop-page", "1"])
        .assert()
        .success()
        .stdout(contains("Result:      OK"));
}

#[test]
fn invalid_dpi() {
    pabaru()
        .args(["encode", ".", "-o", "x.pdf", "--dot-dpi", "250"])
        .assert()
        .failure()
        .stderr(contains("must divide"));
}

#[test]
fn plan_prints_three_profiles() {
    let tmp = tempfile::tempdir().unwrap();
    let input = make_input(tmp.path());
    pabaru()
        .arg("plan")
        .arg(&input)
        .assert()
        .success()
        .stdout(contains("Compact"))
        .stdout(contains("Balanced"))
        .stdout(contains("Archival"));

    let out = pabaru()
        .arg("plan")
        .arg(&input)
        .args(["--json", "--scanner-dpi", "1200", "--printer", "laser1200"])
        .output()
        .unwrap();
    assert!(out.status.success());
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(json.as_array().unwrap().len(), 3);
    assert_eq!(json[0]["profile"], "Compact");
}

#[test]
fn encode_with_profile_and_decode_from_pdf() {
    let tmp = tempfile::tempdir().unwrap();
    let input = make_input(tmp.path());
    let pdf = tmp.path().join("backup.pdf");
    pabaru()
        .arg("encode")
        .arg(&input)
        .arg("-o")
        .arg(&pdf)
        .args(["--profile", "archival", "--paper", "70x60", "--margin", "4"])
        .assert()
        .success()
        .stdout(contains("survives the loss of a whole page"));

    // Restore directly from the PDF (page images are extracted from it).
    let out = tmp.path().join("restored");
    pabaru()
        .arg("decode")
        .arg(&pdf)
        .arg("-o")
        .arg(&out)
        .assert()
        .success()
        .stdout(contains("[2]"))
        .stdout(contains("Restored and verified 2 files"));
}
