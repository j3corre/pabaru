//! # pabaru-crypto
//!
//! Optional password encryption of a backup.
//!
//! * **Argon2id** derives a 256-bit key from the password. It is deliberately
//!   slow and memory-hard so that brute-force password guessing is expensive.
//! * **XChaCha20-Poly1305** encrypts the data and also protects it against
//!   modification (AEAD). A wrong password or corrupted data is reliably detected.
//!
//! Output of [`encrypt`]: `nonce (24 B) || ciphertext || tag (16 B)`.
//! The salt and Argon2 parameters are stored in the superblock, the password never.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::RngCore;
use rand::rngs::OsRng;
use thiserror::Error;

/// Length of the XChaCha20 nonce.
pub const NONCE_LEN: usize = 24;
/// Length of the Poly1305 authentication tag.
pub const TAG_LEN: usize = 16;
/// How many bytes encryption adds to the data.
pub const OVERHEAD: usize = NONCE_LEN + TAG_LEN;

/// Encryption errors.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("invalid Argon2 parameters: {0}")]
    Kdf(String),
    #[error("wrong password or corrupted data")]
    Decrypt,
    #[error("encrypted data is too short")]
    TooShort,
}

/// Argon2id parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdfConfig {
    /// Memory in KiB.
    pub mem_kib: u32,
    /// Number of passes.
    pub iterations: u32,
    /// Number of threads.
    pub parallelism: u8,
}

impl Default for KdfConfig {
    /// Defaults: 64 MiB, 3 passes, 1 thread (OWASP recommendation).
    fn default() -> Self {
        Self {
            mem_kib: 64 * 1024,
            iterations: 3,
            parallelism: 1,
        }
    }
}

/// Generates a random salt.
pub fn random_salt() -> [u8; 16] {
    let mut s = [0u8; 16];
    OsRng.fill_bytes(&mut s);
    s
}

/// Derives a 256-bit key from the password.
fn derive_key(password: &str, salt: &[u8; 16], cfg: KdfConfig) -> Result<[u8; 32], CryptoError> {
    let params = Params::new(
        cfg.mem_kib,
        cfg.iterations,
        cfg.parallelism as u32,
        Some(32),
    )
    .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut key = [0u8; 32];
    argon
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;
    Ok(key)
}

/// Encrypts data. Returns `nonce || ciphertext || tag`.
pub fn encrypt(
    plain: &[u8],
    password: &str,
    salt: &[u8; 16],
    cfg: KdfConfig,
) -> Result<Vec<u8>, CryptoError> {
    let key = derive_key(password, salt, cfg)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let mut nonce = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce);
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce), plain)
        .map_err(|_| CryptoError::Decrypt)?;
    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypts data produced by [`encrypt`].
pub fn decrypt(
    blob: &[u8],
    password: &str,
    salt: &[u8; 16],
    cfg: KdfConfig,
) -> Result<Vec<u8>, CryptoError> {
    if blob.len() < OVERHEAD {
        return Err(CryptoError::TooShort);
    }
    let key = derive_key(password, salt, cfg)?;
    let cipher = XChaCha20Poly1305::new((&key).into());
    let (nonce, ct) = blob.split_at(NONCE_LEN);
    cipher
        .decrypt(XNonce::from_slice(nonce), ct)
        .map_err(|_| CryptoError::Decrypt)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fast parameters for tests (the defaults would slow tests down needlessly).
    const FAST: KdfConfig = KdfConfig {
        mem_kib: 256,
        iterations: 1,
        parallelism: 1,
    };

    #[test]
    fn round_trip() {
        let salt = random_salt();
        let blob = encrypt(b"top secret", "password", &salt, FAST).unwrap();
        assert_eq!(blob.len(), 10 + OVERHEAD);
        assert_eq!(
            decrypt(&blob, "password", &salt, FAST).unwrap(),
            b"top secret"
        );
    }

    #[test]
    fn wrong_password() {
        let salt = random_salt();
        let blob = encrypt(b"x", "password", &salt, FAST).unwrap();
        assert_eq!(
            decrypt(&blob, "other", &salt, FAST),
            Err(CryptoError::Decrypt)
        );
    }

    #[test]
    fn corrupted_data() {
        let salt = random_salt();
        let mut blob = encrypt(b"data", "password", &salt, FAST).unwrap();
        blob[30] ^= 1;
        assert_eq!(
            decrypt(&blob, "password", &salt, FAST),
            Err(CryptoError::Decrypt)
        );
    }
}
