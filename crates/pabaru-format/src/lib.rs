//! # pabaru-format
//!
//! Pabaru binary format version 1. This crate deals with neither images nor
//! files, only with conversion between logical structures and the bytes that get printed.
//!
//! ## Block structure (200 bytes = 40×40 dots)
//!
//! ```text
//! +----------------+-----------------+-----------+------------------+
//! | header 8 B     | payload 128 B   | CRC32C 4B | RS parity 60 B   |
//! +----------------+-----------------+-----------+------------------+
//! |<------------- 140 B (data for RS) --------->|
//! ```
//!
//! Finally the whole 200-byte word is XORed with a fixed pseudorandom
//! sequence ([`whiten`]) so that no large white/black areas appear on paper.

mod block;
pub mod dots;
mod superblock;
mod whitening;

pub use block::{Block, BlockHeader, DecodedBlock};
pub use superblock::{Flags, KdfParams, Superblock};
pub use whitening::whiten;

use thiserror::Error;

/// Format version written by this crate.
pub const FORMAT_VERSION: u8 = 1;
/// Size of the block's data area in dots (side of the square).
pub const BLOCK_DOTS: usize = 40;
/// Length of the block header.
pub const HEADER_LEN: usize = 8;
/// Length of the payload in a block.
pub const PAYLOAD_LEN: usize = pabaru_ecc::outer::SHARD_LEN;
/// Length of the CRC.
pub const CRC_LEN: usize = 4;
/// Length of the whole encoded block (= BLOCK_DOTS² / 8).
pub const BLOCK_LEN: usize = pabaru_ecc::inner::CODEWORD_LEN;
/// Special block index marking the superblock.
pub const SUPERBLOCK_INDEX: u32 = u32::MAX;

// Compile-time consistency check of the constants.
const _: () = assert!(HEADER_LEN + PAYLOAD_LEN + CRC_LEN == pabaru_ecc::inner::DATA_LEN);
const _: () = assert!(BLOCK_DOTS * BLOCK_DOTS / 8 == BLOCK_LEN);

/// Errors while reading the format.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum FormatError {
    #[error("block cannot be corrected: {0}")]
    Ecc(#[from] pabaru_ecc::EccError),
    #[error("CRC checksum mismatch")]
    CrcMismatch,
    #[error("invalid superblock: {0}")]
    InvalidSuperblock(&'static str),
    #[error("unsupported format version {0}")]
    UnsupportedVersion(u8),
}
