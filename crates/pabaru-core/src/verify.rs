//! Backup verification without a printer: encoding → simulated print and scan →
//! decoding → restore into a temporary folder with hash checks.

use std::path::PathBuf;

use pabaru_archive::Destination;
use pabaru_scan::simulate::{ScanSim, simulate};

use crate::{BackupInfo, CoreError, Decoder, EncodeOptions, encode};

/// Verification settings.
#[derive(Debug, Clone, Default)]
pub struct VerifyOptions {
    /// Parameters of the simulated scan.
    pub sim: ScanSim,
    /// Pages (0-based) that get "lost" – they are not scanned at all.
    pub drop_pages: Vec<usize>,
}

/// Verification result.
#[derive(Debug, Clone)]
pub struct VerifyReport {
    pub info: BackupInfo,
    pub pages_scanned: usize,
    pub blocks_ok: usize,
    pub blocks_failed: usize,
    /// How many data shards the outer code had to reconstruct.
    pub recovered_shards: usize,
    /// Number of restored files with a matching hash.
    pub verified_files: usize,
}

/// Runs the full verification. Returns an error if the restore fails.
pub fn verify(
    inputs: &[PathBuf],
    opts: &EncodeOptions,
    vopts: &VerifyOptions,
) -> Result<VerifyReport, CoreError> {
    let backup = encode(inputs, opts)?;
    let mut decoder = Decoder::new();
    let (mut pages_scanned, mut blocks_ok, mut blocks_failed) = (0, 0, 0);

    for i in 0..backup.page_count() {
        if vopts.drop_pages.contains(&i) {
            continue;
        }
        let bmp = backup.render_page(i);
        // Each page gets a different noise seed.
        let sim = ScanSim {
            seed: vopts.sim.seed.wrapping_add(i as u64 * 7919),
            // The simulator needs the real printer DPI (mm → pixel conversion).
            printer_dpi: opts.params.printer_dpi as f64,
            ..vopts.sim.clone()
        };
        let scan = simulate(&bmp, &sim);
        let report = decoder.add_image(&scan)?;
        pages_scanned += 1;
        blocks_ok += report.ok_blocks;
        blocks_failed += report.failed_blocks;
    }

    let dir = tempfile::tempdir()?;
    let restore = decoder.restore(opts.password.as_deref(), Destination::Folder(dir.path()))?;
    Ok(VerifyReport {
        info: backup.info,
        pages_scanned,
        blocks_ok,
        blocks_failed,
        recovered_shards: restore.recovered_shards,
        verified_files: restore.unpack.verified_files,
    })
}
