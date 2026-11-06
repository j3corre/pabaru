//! Distribution of blocks (shards) onto pages.
//!
//! Each page gets:
//! * at least `superblocks_per_page` copies of the superblock (so that every
//!   page can be identified on its own),
//! * an even share of the shards (page counts differ by at most 1 –
//!   this matters for the "survives the loss of a whole page" invariant),
//! * the remaining free cells are filled with more superblock copies.
//!
//! Positions on the page are pseudo-randomly shuffled so that local damage
//! (a stain, a fold) does not hit a contiguous run of shards. The decoder does
//! not need to know the order – the shard index is in every block's header.

use serde::{Deserialize, Serialize};

/// Content of one grid cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellContent {
    /// A copy of the superblock.
    Superblock,
    /// A shard with a global index (`0..k` data, `k..k+m` parity).
    Shard(u32),
}

/// How many pages are needed for `total_shards` shards.
pub fn pages_needed(total_shards: u32, cells_per_page: u32, superblocks_per_page: u32) -> u32 {
    let cap = cells_per_page.saturating_sub(superblocks_per_page);
    assert!(cap > 0, "no shard fits on a page");
    total_shards.div_ceil(cap).max(1)
}

/// Highest number of shards on a single page (for the resilience calculation).
pub fn max_shards_per_page(total_shards: u32, pages: u32) -> u32 {
    total_shards.div_ceil(pages)
}

/// Computes the number of parity shards.
///
/// 1. At least `redundancy_pct` % of `k`.
/// 2. Free cells on the last page are filled with parity ("for free").
/// 3. If the backup must survive the loss of a page, there must be at least
///    as much parity as one page carries shards – adding a page if needed.
///
/// Returns `(m, number of pages)`.
pub fn plan_parity(
    k: u32,
    cells_per_page: u32,
    redundancy_pct: u32,
    survive_page_loss: bool,
) -> (u32, u32) {
    let cap = cells_per_page - crate::SUPERBLOCKS_PER_PAGE;
    let mut m = (k as u64 * redundancy_pct as u64).div_ceil(100) as u32;
    loop {
        let pages = pages_needed(k + m, cells_per_page, crate::SUPERBLOCKS_PER_PAGE);
        // Fill all free cells with parity.
        m = pages * cap - k;
        let per_page = max_shards_per_page(k + m, pages);
        if survive_page_loss && m < per_page {
            // Not enough – grow the parity so that another page is added.
            m = pages * cap - k + 1;
            continue;
        }
        return (m, pages);
    }
}

/// Distributes shards onto pages. Returns, for each page, the content of all
/// cells in row order (length = `cells_per_page`).
pub fn paginate(
    total_shards: u32,
    cells_per_page: u32,
    superblocks_per_page: u32,
    seed: u64,
) -> Vec<Vec<CellContent>> {
    let pages = pages_needed(total_shards, cells_per_page, superblocks_per_page);
    let base = total_shards / pages;
    let extra = total_shards % pages; // the first `extra` pages get 1 more shard

    let mut out = Vec::with_capacity(pages as usize);
    let mut next = 0u32;
    for p in 0..pages {
        let count = base + u32::from(p < extra);
        let mut cells: Vec<CellContent> = (next..next + count).map(CellContent::Shard).collect();
        next += count;
        cells.resize(cells_per_page as usize, CellContent::Superblock);
        shuffle(
            &mut cells,
            seed ^ (p as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15),
        );
        out.push(cells);
    }
    out
}

/// Fisher-Yates shuffle with a simple deterministic generator (splitmix64).
fn shuffle<T>(items: &mut [T], seed: u64) {
    let mut state = seed;
    let mut next = || {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    for i in (1..items.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        items.swap(i, j);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn parity_fills_last_page() {
        // 100 blocks per page (minus superblocks leaves `cap` for shards), 150 data, 10 % parity.
        let (m, pages) = plan_parity(150, 100, 10, false);
        assert_eq!(pages, 2);
        let cap = 100 - crate::SUPERBLOCKS_PER_PAGE;
        assert_eq!(150 + m, 2 * cap);
    }

    #[test]
    fn survives_page_loss() {
        for k in [1u32, 50, 98, 99, 150, 500, 1000] {
            let (m, pages) = plan_parity(k, 100, 5, true);
            let per_page = max_shards_per_page(k + m, pages);
            assert!(m >= per_page, "k={k}: m={m} < {per_page}");
            assert!(pages >= 2);
        }
    }

    proptest! {
        #[test]
        fn every_shard_exactly_once(total in 0u32..5000, cells in 3u32..2000, seed in any::<u64>()) {
            let pages = paginate(total, cells, 2, seed);
            let mut seen = vec![false; total as usize];
            for page in &pages {
                prop_assert_eq!(page.len(), cells as usize);
                let sb = page.iter().filter(|c| **c == CellContent::Superblock).count();
                prop_assert!(sb >= 2);
                for c in page {
                    if let CellContent::Shard(i) = c {
                        prop_assert!(!seen[*i as usize]);
                        seen[*i as usize] = true;
                    }
                }
            }
            prop_assert!(seen.iter().all(|&s| s));
            // Evenness: no page has more than max_shards_per_page.
            let max = max_shards_per_page(total, pages.len() as u32) as usize;
            for page in &pages {
                prop_assert!(page.iter().filter(|c| matches!(c, CellContent::Shard(_))).count() <= max);
            }
        }
    }
}
