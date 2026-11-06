//! Data block – the basic unit printed on paper.

use std::sync::OnceLock;

use pabaru_ecc::inner::{DATA_LEN, ReedSolomon};

use crate::{BLOCK_LEN, CRC_LEN, FormatError, HEADER_LEN, PAYLOAD_LEN, SUPERBLOCK_INDEX, whiten};

/// Shared RS encoder (the generator only needs to be computed once).
fn rs() -> &'static ReedSolomon {
    static RS: OnceLock<ReedSolomon> = OnceLock::new();
    RS.get_or_init(ReedSolomon::v1)
}

/// Block header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    /// Random backup identifier. Prevents mixing blocks from different backups.
    pub backup_id: u32,
    /// Global shard index: `0..k` data, `k..k+m` parity,
    /// [`SUPERBLOCK_INDEX`] = superblock.
    pub index: u32,
}

impl BlockHeader {
    /// Is this the superblock?
    pub fn is_superblock(&self) -> bool {
        self.index == SUPERBLOCK_INDEX
    }
}

/// Logical block: header + 128 B of data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub header: BlockHeader,
    pub payload: [u8; PAYLOAD_LEN],
}

/// Result of decoding a block from a scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedBlock {
    pub block: Block,
    /// How many bytes the inner RS had to correct (for the quality map).
    pub corrected_bytes: usize,
}

impl Block {
    /// Converts the block into 200 bytes ready for printing
    /// (header, payload, CRC, RS parity, whitening).
    pub fn encode(&self) -> [u8; BLOCK_LEN] {
        let mut data = [0u8; DATA_LEN];
        data[0..4].copy_from_slice(&self.header.backup_id.to_le_bytes());
        data[4..8].copy_from_slice(&self.header.index.to_le_bytes());
        data[HEADER_LEN..HEADER_LEN + PAYLOAD_LEN].copy_from_slice(&self.payload);
        let crc = crc32c::crc32c(&data[..HEADER_LEN + PAYLOAD_LEN]);
        data[HEADER_LEN + PAYLOAD_LEN..].copy_from_slice(&crc.to_le_bytes());

        let cw = rs().encode(&data);
        let mut out = [0u8; BLOCK_LEN];
        out.copy_from_slice(&cw);
        whiten(&mut out);
        out
    }

    /// Decodes 200 bytes read from a scan. Corrects errors and verifies the CRC.
    pub fn decode(raw: &[u8; BLOCK_LEN]) -> Result<DecodedBlock, FormatError> {
        Self::decode_with_erasures(raw, &[])
    }

    /// Like [`Block::decode`], but with positions of unreliable bytes (erasures).
    /// RS can then correct more damaged bytes (2 · errors + erasures ≤ 60).
    /// The CRC remains a safeguard against a wrong "correction".
    pub fn decode_with_erasures(
        raw: &[u8; BLOCK_LEN],
        erasures: &[usize],
    ) -> Result<DecodedBlock, FormatError> {
        let mut cw = *raw;
        whiten(&mut cw);
        let corrected_bytes = if erasures.is_empty() {
            rs().correct(&mut cw)?
        } else {
            rs().correct_with_erasures(&mut cw, erasures)?
        };

        let body = &cw[..HEADER_LEN + PAYLOAD_LEN];
        let stored_crc = u32::from_le_bytes(
            cw[HEADER_LEN + PAYLOAD_LEN..HEADER_LEN + PAYLOAD_LEN + CRC_LEN]
                .try_into()
                .expect("CRC length is fixed"),
        );
        // The CRC is a second safeguard: it catches rare cases where RS "corrects"
        // a damaged block into a different valid word.
        if crc32c::crc32c(body) != stored_crc {
            return Err(FormatError::CrcMismatch);
        }

        let header = BlockHeader {
            backup_id: u32::from_le_bytes(cw[0..4].try_into().unwrap()),
            index: u32::from_le_bytes(cw[4..8].try_into().unwrap()),
        };
        let mut payload = [0u8; PAYLOAD_LEN];
        payload.copy_from_slice(&cw[HEADER_LEN..HEADER_LEN + PAYLOAD_LEN]);
        Ok(DecodedBlock {
            block: Block { header, payload },
            corrected_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn round_trip_with_errors(
            id in any::<u32>(),
            index in any::<u32>(),
            payload in proptest::array::uniform32(any::<u8>()),
            errors in proptest::collection::btree_map(0usize..BLOCK_LEN, 1u8..=255, 0..=30),
        ) {
            // Build the 128 B payload from 4× 32 B (proptest cannot generate a 128-array directly).
            let mut p = [0u8; PAYLOAD_LEN];
            for (i, chunk) in p.chunks_mut(32).enumerate() {
                chunk.copy_from_slice(&payload);
                chunk[0] ^= i as u8;
            }
            let block = Block { header: BlockHeader { backup_id: id, index }, payload: p };
            let mut raw = block.encode();
            for (&pos, &v) in &errors { raw[pos] ^= v; }
            let dec = Block::decode(&raw).unwrap();
            prop_assert_eq!(dec.block, block);
            prop_assert_eq!(dec.corrected_bytes, errors.len());
        }
    }

    #[test]
    fn zeros_are_not_zeros_on_paper() {
        let block = Block {
            header: BlockHeader {
                backup_id: 0,
                index: 0,
            },
            payload: [0; PAYLOAD_LEN],
        };
        let raw = block.encode();
        let zero_bytes = raw.iter().filter(|&&b| b == 0).count();
        assert!(
            zero_bytes < 20,
            "whitening does not work: {zero_bytes} zero bytes"
        );
    }

    #[test]
    fn destroyed_block_is_rejected() {
        let raw = [0x77u8; BLOCK_LEN];
        assert!(Block::decode(&raw).is_err());
    }
}
