//! Decoding: page scans → restored files.
//!
//! [`Decoder`] collects blocks from any number of scans in any order
//! (even from several copies of the same backup). As soon as it has at least
//! `k` distinct shards, the backup can be restored.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use pabaru_archive::{Destination, UnpackReport};
use pabaru_crypto::KdfConfig;
use pabaru_ecc::outer;
use pabaru_format::{PAYLOAD_LEN, Superblock};
use pabaru_scan::{Gray, PageScan, decode_page};

use crate::{CoreError, short_hash};

/// Blocks of a single backup (keyed by `backup_id`).
#[derive(Debug, Default, Clone)]
struct Collected {
    superblock: Option<Superblock>,
    shards: BTreeMap<u32, [u8; PAYLOAD_LEN]>,
    pages_seen: BTreeSet<u16>,
}

/// Result of processing a single page.
#[derive(Debug, Clone)]
pub struct PageReport {
    pub backup_id: Option<u32>,
    /// Page number according to the superblock (0-based).
    pub page_index: Option<u16>,
    pub ok_blocks: usize,
    pub failed_blocks: usize,
    /// How many new (not yet read) shards the page contributed.
    pub new_shards: usize,
    /// Detailed scan result (quality map).
    pub scan: PageScan,
}

/// Progress of block collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Progress {
    pub backup_id: u32,
    pub name: String,
    /// Number of distinct shards read.
    pub have: usize,
    /// How many shards are needed (k).
    pub need: usize,
    pub total: usize,
    pub pages_seen: Vec<u16>,
    pub page_count: u16,
    pub encrypted: bool,
}

impl Progress {
    pub fn can_restore(&self) -> bool {
        self.have >= self.need
    }
    /// Pages that have not been scanned yet.
    pub fn missing_pages(&self) -> Vec<u16> {
        (0..self.page_count)
            .filter(|p| !self.pages_seen.contains(p))
            .collect()
    }
}

/// Restore result.
#[derive(Debug, Clone)]
pub struct RestoreReport {
    pub unpack: UnpackReport,
    /// How many shards the outer code had to reconstruct.
    pub recovered_shards: usize,
}

/// Collects blocks from scans.
#[derive(Debug, Default, Clone)]
pub struct Decoder {
    backups: HashMap<u32, Collected>,
}

impl Decoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Decodes a page image and adds its blocks.
    pub fn add_image(&mut self, img: &Gray) -> Result<PageReport, CoreError> {
        Ok(self.add_scan(decode_page(img)?))
    }

    /// Adds blocks from an already decoded scan.
    pub fn add_scan(&mut self, scan: PageScan) -> PageReport {
        let mut report = PageReport {
            backup_id: None,
            page_index: None,
            ok_blocks: 0,
            failed_blocks: 0,
            new_shards: 0,
            scan,
        };
        for cell in &report.scan.cells {
            let Some(decoded) = &cell.block else {
                report.failed_blocks += 1;
                continue;
            };
            report.ok_blocks += 1;
            let header = decoded.block.header;
            report.backup_id.get_or_insert(header.backup_id);
            let entry = self.backups.entry(header.backup_id).or_default();
            if header.is_superblock() {
                if let Ok(sb) = Superblock::from_payload(&decoded.block.payload) {
                    report.page_index = Some(sb.page_index);
                    entry.pages_seen.insert(sb.page_index);
                    entry.superblock.get_or_insert(sb);
                }
            } else if entry
                .shards
                .insert(header.index, decoded.block.payload)
                .is_none()
            {
                report.new_shards += 1;
            }
        }
        report
    }

    /// The primary backup: the one with a superblock and the most shards.
    fn primary(&self) -> Option<(u32, &Collected, &Superblock)> {
        self.backups
            .iter()
            .filter_map(|(id, c)| c.superblock.as_ref().map(|sb| (*id, c, sb)))
            .max_by_key(|(_, c, _)| c.shards.len())
    }

    /// Current progress (None until a superblock has been read).
    pub fn progress(&self) -> Option<Progress> {
        let (id, c, sb) = self.primary()?;
        let total = (sb.data_shards + sb.parity_shards) as usize;
        Some(Progress {
            backup_id: id,
            name: sb.name.clone(),
            have: c.shards.keys().filter(|&&i| (i as usize) < total).count(),
            need: sb.data_shards as usize,
            total,
            pages_seen: c.pages_seen.iter().copied().collect(),
            page_count: sb.page_count,
            encrypted: sb.flags.encrypted,
        })
    }

    /// Is the backup encrypted?
    pub fn needs_password(&self) -> bool {
        self.primary().is_some_and(|(_, _, sb)| sb.flags.encrypted)
    }

    /// Returns the superblock (metadata) of the primary backup.
    pub fn superblock(&self) -> Option<&Superblock> {
        self.primary().map(|(_, _, sb)| sb)
    }

    /// Reassembles the stored stream (after outer-code correction and decryption).
    fn stream(&self, password: Option<&str>) -> Result<(Vec<u8>, usize), CoreError> {
        let (_, c, sb) = self.primary().ok_or(CoreError::NoSuperblock)?;
        let (k, m) = (sb.data_shards as usize, sb.parity_shards as usize);

        let received: BTreeMap<usize, Vec<u8>> = c
            .shards
            .iter()
            .filter(|(i, _)| (**i as usize) < k + m)
            .map(|(i, p)| (*i as usize, p.to_vec()))
            .collect();
        if received.len() < k {
            return Err(CoreError::NotEnoughBlocks {
                have: received.len(),
                need: k,
            });
        }
        let missing_data = (0..k).filter(|i| !received.contains_key(i)).count();
        let data = outer::decode(k, m, &received)?;

        let mut stream: Vec<u8> = data.concat();
        stream.truncate(sb.stream_len as usize);
        if short_hash(&stream) != sb.stream_hash {
            return Err(CoreError::HashMismatch);
        }

        if sb.flags.encrypted {
            let pw = password.ok_or(CoreError::PasswordRequired)?;
            let cfg = KdfConfig {
                mem_kib: sb.kdf.mem_kib,
                iterations: sb.kdf.iterations,
                parallelism: sb.kdf.parallelism,
            };
            stream = pabaru_crypto::decrypt(&stream, pw, &sb.kdf.salt, cfg)?;
        }
        Ok((stream, missing_data))
    }

    /// Restores the files to the chosen destination. All file hashes are verified.
    pub fn restore(
        &self,
        password: Option<&str>,
        dest: Destination,
    ) -> Result<RestoreReport, CoreError> {
        let (stream, recovered_shards) = self.stream(password)?;
        let unpack = pabaru_archive::unpack_to(&stream, dest)?;
        Ok(RestoreReport {
            unpack,
            recovered_shards,
        })
    }

    /// Checks the password without writing to disk (decrypts the stream; the
    /// AEAD tag reliably detects a wrong password). Returns `CoreError::Crypto`
    /// on a wrong password.
    pub fn check_password(&self, password: &str) -> Result<(), CoreError> {
        self.stream(Some(password)).map(|_| ())
    }

    /// Reads the backup manifest (list of roots and files) without writing
    /// to disk – e.g. to show the user what will be restored.
    pub fn manifest(&self, password: Option<&str>) -> Result<pabaru_archive::Manifest, CoreError> {
        let (stream, _) = self.stream(password)?;
        Ok(pabaru_archive::read_manifest(&stream)?)
    }
}
