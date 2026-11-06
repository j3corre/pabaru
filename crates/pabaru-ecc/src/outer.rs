//! Outer code: erasure Reed-Solomon across blocks.
//!
//! The data is split into `k` data shards (payloads) of equal length and `m`
//! parity shards are computed. All data can be recovered from any `k` received
//! shards (data or parity). So up to `m` shards may be lost.
//!
//! The implementation uses the `reed-solomon-simd` library (GF(2^16), fast thanks to SIMD).

use std::collections::BTreeMap;

use crate::EccError;

/// Length of one shard (block payload) in format v1.
pub const SHARD_LEN: usize = 128;

/// Maximum total number of shards in one code (GF(2^16) limit).
pub const MAX_TOTAL_SHARDS: usize = 65_535;

/// Computes `parity_count` parity shards for the data shards.
///
/// All shards must have the same (even) length.
pub fn encode(data: &[Vec<u8>], parity_count: usize) -> Result<Vec<Vec<u8>>, EccError> {
    if parity_count == 0 || data.is_empty() {
        return Ok(Vec::new());
    }
    check_lengths(data.iter().map(|s| s.as_slice()))?;
    reed_solomon_simd::encode(data.len(), parity_count, data)
        .map_err(|e| EccError::Outer(e.to_string()))
}

/// Recovers all data shards.
///
/// * `data_count` – number of data shards `k`,
/// * `parity_count` – number of parity shards `m`,
/// * `received` – received shards keyed by global index:
///   indices `0..k` are data, `k..k+m` parity.
///
/// Returns all `k` data shards in the correct order.
pub fn decode(
    data_count: usize,
    parity_count: usize,
    received: &BTreeMap<usize, Vec<u8>>,
) -> Result<Vec<Vec<u8>>, EccError> {
    let missing_data = (0..data_count)
        .filter(|i| !received.contains_key(i))
        .count();
    // Fast path: we have all data, parity is not needed.
    if missing_data == 0 {
        return Ok((0..data_count).map(|i| received[&i].clone()).collect());
    }
    let available = received
        .keys()
        .filter(|&&i| i < data_count + parity_count)
        .count();
    if parity_count == 0 || available < data_count {
        return Err(EccError::NotEnoughShards {
            missing: data_count + parity_count - available,
            recoverable: parity_count,
        });
    }
    check_lengths(received.values().map(|s| s.as_slice()))?;

    let originals = received
        .iter()
        .filter(|(i, _)| **i < data_count)
        .map(|(i, s)| (*i, s.as_slice()));
    let recovery = received
        .iter()
        .filter(|(i, _)| **i >= data_count && **i < data_count + parity_count)
        .map(|(i, s)| (*i - data_count, s.as_slice()));

    let restored = reed_solomon_simd::decode(data_count, parity_count, originals, recovery)
        .map_err(|e| EccError::Outer(e.to_string()))?;

    let mut out = Vec::with_capacity(data_count);
    for i in 0..data_count {
        match received.get(&i) {
            Some(s) => out.push(s.clone()),
            None => out.push(
                restored
                    .get(&i)
                    .cloned()
                    .ok_or_else(|| EccError::Outer(format!("shard {i} was not recovered")))?,
            ),
        }
    }
    Ok(out)
}

/// Checks that all shards have the same non-zero even length.
fn check_lengths<'a>(mut shards: impl Iterator<Item = &'a [u8]>) -> Result<(), EccError> {
    let Some(first) = shards.next() else {
        return Ok(());
    };
    let len = first.len();
    if len == 0 || len % 2 != 0 {
        return Err(EccError::InvalidLength {
            expected: SHARD_LEN,
            got: len,
        });
    }
    for s in shards {
        if s.len() != len {
            return Err(EccError::InvalidLength {
                expected: len,
                got: s.len(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_data(k: usize) -> Vec<Vec<u8>> {
        (0..k)
            .map(|i| (0..SHARD_LEN).map(|j| (i * 31 + j * 7) as u8).collect())
            .collect()
    }

    #[test]
    fn recovers_lost_shards() {
        let data = make_data(20);
        let parity = encode(&data, 5).unwrap();
        assert_eq!(parity.len(), 5);

        let mut received = BTreeMap::new();
        for (i, s) in data.iter().enumerate() {
            // Drop 5 data shards.
            if ![0, 3, 7, 11, 19].contains(&i) {
                received.insert(i, s.clone());
            }
        }
        for (i, s) in parity.iter().enumerate() {
            received.insert(20 + i, s.clone());
        }
        assert_eq!(decode(20, 5, &received).unwrap(), data);
    }

    #[test]
    fn too_many_losses() {
        let data = make_data(10);
        let parity = encode(&data, 2).unwrap();
        let mut received = BTreeMap::new();
        for (i, shard) in data.iter().enumerate().skip(3) {
            received.insert(i, shard.clone());
        }
        received.insert(10, parity[0].clone());
        assert!(matches!(
            decode(10, 2, &received),
            Err(EccError::NotEnoughShards { .. })
        ));
    }

    #[test]
    fn no_parity_and_no_losses() {
        let data = make_data(3);
        assert!(encode(&data, 0).unwrap().is_empty());
        let received: BTreeMap<_, _> = data.iter().cloned().enumerate().collect();
        assert_eq!(decode(3, 0, &received).unwrap(), data);
    }
}
