//! # pabaru-ecc
//!
//! Error-correcting codes (ECC) for Pabaru.
//!
//! We use two levels of protection:
//!
//! 1. **Inner code** ([`inner`]) – Reed-Solomon over the field GF(256).
//!    It protects one block (200 bytes) and can correct up to 30 *erroneous*
//!    bytes without knowing in advance which ones they are. Typically these
//!    are a few misread dots on a scan.
//! 2. **Outer code** ([`outer`]) – an erasure code across blocks. If a block
//!    cannot be read at all (e.g. a coffee stain), we know *which* one is
//!    missing and reconstruct it from the parity blocks.

pub mod gf256;
pub mod inner;
pub mod outer;

use thiserror::Error;

/// Errors the error-correcting code can return.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum EccError {
    /// The block contains more errors than the inner code can correct.
    #[error("too many errors in block, cannot correct")]
    TooManyErrors,
    /// The input has the wrong length.
    #[error("invalid input length: expected {expected}, got {got}")]
    InvalidLength { expected: usize, got: usize },
    /// Too many blocks are missing for the outer code to recover them.
    #[error("too many blocks missing: {missing} missing, at most {recoverable} recoverable")]
    NotEnoughShards { missing: usize, recoverable: usize },
    /// Error from the reed-solomon-simd library (e.g. unsupported shard count).
    #[error("outer code error: {0}")]
    Outer(String),
}
