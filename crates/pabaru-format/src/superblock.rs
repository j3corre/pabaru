//! Superblock – backup metadata present on every page.
//!
//! Thanks to the superblock the decoder knows how many data and parity blocks
//! there are, how long the stream is, whether it is encrypted, etc. It is printed
//! on every page, so reading any one of them is enough.
//!
//! Layout of the 128 bytes (little-endian):
//!
//! | offset | length | field |
//! |-------:|-------:|-------|
//! | 0  | 4  | magic `PBRU` |
//! | 4  | 1  | format version |
//! | 5  | 1  | flags ([`Flags`]) |
//! | 6  | 8  | length of the stored stream |
//! | 14 | 4  | number of data shards k |
//! | 18 | 4  | number of parity shards m |
//! | 22 | 2  | page index (from 0) |
//! | 24 | 2  | page count |
//! | 26 | 2  | block columns per page |
//! | 28 | 2  | block rows per page |
//! | 30 | 16 | BLAKE3 hash of the stream (truncated) |
//! | 46 | 16 | KDF salt |
//! | 62 | 4  | Argon2 memory (KiB) |
//! | 66 | 4  | Argon2 iterations |
//! | 70 | 1  | Argon2 parallelism |
//! | 71 | 1  | name length N |
//! | 72 | N  | backup name (UTF-8, max. 56 B) |

use crate::{FORMAT_VERSION, FormatError, PAYLOAD_LEN};

const MAGIC: &[u8; 4] = b"PBRU";
const NAME_OFFSET: usize = 72;
/// Maximum length of the backup name in bytes.
pub const MAX_NAME_LEN: usize = PAYLOAD_LEN - NAME_OFFSET;

/// Backup flags (bit field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags {
    /// The stream is zstd-compressed.
    pub compressed: bool,
    /// The stream is encrypted (XChaCha20-Poly1305).
    pub encrypted: bool,
}

impl Flags {
    fn to_byte(self) -> u8 {
        (self.compressed as u8) | ((self.encrypted as u8) << 1)
    }
    fn from_byte(b: u8) -> Self {
        Self {
            compressed: b & 1 != 0,
            encrypted: b & 2 != 0,
        }
    }
}

/// Password key-derivation parameters (Argon2id). The password is never printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KdfParams {
    pub salt: [u8; 16],
    pub mem_kib: u32,
    pub iterations: u32,
    pub parallelism: u8,
}

/// Backup metadata.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Superblock {
    pub flags: Flags,
    pub stream_len: u64,
    pub data_shards: u32,
    pub parity_shards: u32,
    pub page_index: u16,
    pub page_count: u16,
    pub cols: u16,
    pub rows: u16,
    pub stream_hash: [u8; 16],
    pub kdf: KdfParams,
    /// Backup name; truncated to [`MAX_NAME_LEN`] bytes when written.
    pub name: String,
}

impl Superblock {
    /// Serializes the superblock into a block payload.
    pub fn to_payload(&self) -> [u8; PAYLOAD_LEN] {
        let mut p = [0u8; PAYLOAD_LEN];
        p[0..4].copy_from_slice(MAGIC);
        p[4] = FORMAT_VERSION;
        p[5] = self.flags.to_byte();
        p[6..14].copy_from_slice(&self.stream_len.to_le_bytes());
        p[14..18].copy_from_slice(&self.data_shards.to_le_bytes());
        p[18..22].copy_from_slice(&self.parity_shards.to_le_bytes());
        p[22..24].copy_from_slice(&self.page_index.to_le_bytes());
        p[24..26].copy_from_slice(&self.page_count.to_le_bytes());
        p[26..28].copy_from_slice(&self.cols.to_le_bytes());
        p[28..30].copy_from_slice(&self.rows.to_le_bytes());
        p[30..46].copy_from_slice(&self.stream_hash);
        p[46..62].copy_from_slice(&self.kdf.salt);
        p[62..66].copy_from_slice(&self.kdf.mem_kib.to_le_bytes());
        p[66..70].copy_from_slice(&self.kdf.iterations.to_le_bytes());
        p[70] = self.kdf.parallelism;
        let name = truncate_utf8(&self.name, MAX_NAME_LEN);
        p[71] = name.len() as u8;
        p[NAME_OFFSET..NAME_OFFSET + name.len()].copy_from_slice(name.as_bytes());
        p
    }

    /// Parses the superblock from a payload.
    pub fn from_payload(p: &[u8; PAYLOAD_LEN]) -> Result<Self, FormatError> {
        if &p[0..4] != MAGIC {
            return Err(FormatError::InvalidSuperblock("missing PBRU magic"));
        }
        if p[4] != FORMAT_VERSION {
            return Err(FormatError::UnsupportedVersion(p[4]));
        }
        let u16_at = |o: usize| u16::from_le_bytes([p[o], p[o + 1]]);
        let u32_at = |o: usize| u32::from_le_bytes(p[o..o + 4].try_into().unwrap());
        let name_len = p[71] as usize;
        if name_len > MAX_NAME_LEN {
            return Err(FormatError::InvalidSuperblock("name too long"));
        }
        let name = std::str::from_utf8(&p[NAME_OFFSET..NAME_OFFSET + name_len])
            .map_err(|_| FormatError::InvalidSuperblock("name is not UTF-8"))?
            .to_string();
        Ok(Self {
            flags: Flags::from_byte(p[5]),
            stream_len: u64::from_le_bytes(p[6..14].try_into().unwrap()),
            data_shards: u32_at(14),
            parity_shards: u32_at(18),
            page_index: u16_at(22),
            page_count: u16_at(24),
            cols: u16_at(26),
            rows: u16_at(28),
            stream_hash: p[30..46].try_into().unwrap(),
            kdf: KdfParams {
                salt: p[46..62].try_into().unwrap(),
                mem_kib: u32_at(62),
                iterations: u32_at(66),
                parallelism: p[70],
            },
            name,
        })
    }
}

/// Truncates a string to at most `max` bytes without splitting a UTF-8 character.
fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Superblock {
        Superblock {
            flags: Flags {
                compressed: true,
                encrypted: true,
            },
            stream_len: 123_456_789,
            data_shards: 1000,
            parity_shards: 250,
            page_index: 2,
            page_count: 7,
            cols: 50,
            rows: 70,
            stream_hash: [9; 16],
            kdf: KdfParams {
                salt: [3; 16],
                mem_kib: 65536,
                iterations: 3,
                parallelism: 1,
            },
            name: "Fotky z dovolené".into(),
        }
    }

    #[test]
    fn round_trip() {
        let sb = sample();
        assert_eq!(Superblock::from_payload(&sb.to_payload()).unwrap(), sb);
    }

    #[test]
    fn long_name_is_truncated_at_char_boundary() {
        let mut sb = sample();
        sb.name = "ž".repeat(40); // 80 bytes
        let back = Superblock::from_payload(&sb.to_payload()).unwrap();
        assert_eq!(back.name, "ž".repeat(MAX_NAME_LEN / 2));
    }

    #[test]
    fn bad_magic() {
        assert!(Superblock::from_payload(&[0; PAYLOAD_LEN]).is_err());
    }
}
