//! # pabaru-core
//!
//! Ties all modules together into a single API used by both the CLI and the GUI:
//!
//! * [`encode`] – files/folders → pages to print (PDF/PNG),
//! * [`Decoder`] – scans → restored files,
//! * [`verify`] – end-to-end check: encoding → simulated print and scan →
//!   decoding → hash comparison.
//!
//! ```text
//! encode:  archive ─► crypto ─► shards ─► outer RS ─► blocks ─► pages ─► PDF/PNG
//! decode:  scan ─► pabaru-scan ─► blocks ─► outer RS ─► crypto ─► archive
//! ```

mod decode;
mod encode;
mod verify;

pub use decode::{Decoder, PageReport, Progress, RestoreReport};
pub use encode::{
    BackupInfo, EncodeOptions, EncodedBackup, Prepared, encode, encode_packed_with_id,
    encode_prepared, prepare,
};
pub use verify::{VerifyOptions, VerifyReport, verify};

// Re-exported types so that the CLI and GUI do not need to know the inner crates.
pub use pabaru_archive::{Destination, InputSummary, Manifest, summarize};
pub use pabaru_crypto::KdfConfig;
pub use pabaru_layout::{
    CELL_DOTS, FRAME_EXTRA, LayoutProposal, PageGeometry, Paper, PrintParams, Printer, Profile,
    QUIET, SUPERBLOCKS_PER_PAGE, SuggestInput, plan_parity, propose, suggest,
};
pub use pabaru_scan::simulate::{Crumple, ScanSim, Spot, Tear};
pub use pabaru_scan::{Gray, PageScan, decode_page, load_image, load_images};

use thiserror::Error;

/// Pipeline errors.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error(transparent)]
    Archive(#[from] pabaru_archive::ArchiveError),
    #[error(transparent)]
    Crypto(#[from] pabaru_crypto::CryptoError),
    #[error(transparent)]
    Ecc(#[from] pabaru_ecc::EccError),
    #[error(transparent)]
    Scan(#[from] pabaru_scan::ScanError),
    #[error(transparent)]
    Render(#[from] pabaru_render::RenderError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("invalid parameters: {0}")]
    InvalidParams(String),
    #[error("backup is too large: {shards} blocks, the maximum is {max}")]
    TooLarge { shards: usize, max: usize },
    #[error("no superblock (backup metadata) found in the scans")]
    NoSuperblock,
    #[error("not enough blocks: read {have}, need at least {need}")]
    NotEnoughBlocks { have: usize, need: usize },
    #[error("checksum of the restored data does not match")]
    HashMismatch,
    #[error("the backup is encrypted, a password is required")]
    PasswordRequired,
}

/// First 16 bytes of the BLAKE3 hash (stored in the superblock).
pub(crate) fn short_hash(data: &[u8]) -> [u8; 16] {
    let mut h = [0u8; 16];
    h.copy_from_slice(&blake3::hash(data).as_bytes()[..16]);
    h
}
