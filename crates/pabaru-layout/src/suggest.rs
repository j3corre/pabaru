//! Parameter optimizer – proposes a backup layout.
//!
//! The user drops a folder and we need to pick the dot density, redundancy
//! and resilience so that the backup uses as little paper as possible while
//! still being reliably readable with their scanner. Three profiles are offered:
//!
//! | Profile     | Redundancy | Max. density | Loss of a whole page |
//! |-------------|-----------:|-------------:|----------------------|
//! | Compact     | 10 %       | 300 dpi      | no                   |
//! | Balanced    | 25 %       | 200 dpi      | no                   |
//! | Archival    | 35 %       | 150 dpi      | yes                  |
//!
//! For each profile we try every dot density the printer can produce
//! (an integer number of printer pixels per dot) that meets the constraints:
//!
//! * the scan must have at least 3× the resolution of the dot density,
//! * the density must not exceed the profile's and the printer's maximum (ink bleeds).
//!
//! We pick the variant with the fewest pages; on a tie, the one with sparser
//! (and therefore more reliable) dots.

use serde::{Deserialize, Serialize};

use crate::geometry::{Paper, PrintParams};
use crate::paginate::{max_shards_per_page, plan_parity};

/// Maximum number of shards in one outer code (see `pabaru-ecc`).
pub const MAX_TOTAL_SHARDS: u32 = 65_535;
/// Minimum ratio of scan resolution to dot density.
pub const SCAN_RATIO: u32 = 3;

/// Printer type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Printer {
    /// 600 dpi laser printer (the most common).
    Laser600,
    /// 1200 dpi laser printer.
    Laser1200,
    /// Inkjet printer – dots bleed, hence sparser and smaller dots.
    Inkjet,
}

impl Printer {
    pub const ALL: [Printer; 3] = [Printer::Laser600, Printer::Laser1200, Printer::Inkjet];

    /// Physical printer resolution.
    pub fn dpi(self) -> u32 {
        match self {
            Printer::Laser1200 => 1200,
            _ => 600,
        }
    }

    /// Possible numbers of printer pixels per dot (from the densest to the coarsest).
    ///
    /// Coarse dots (down to 50 dpi = 0.5 mm dot) are much more resistant to
    /// crumpling, folds and an imprecise scan. Small backups that come nowhere
    /// near filling a page therefore get them automatically (see [`propose`]).
    fn px_per_dot_candidates(self) -> &'static [u32] {
        match self {
            Printer::Laser600 => &[2, 3, 4, 6, 8, 10, 12],
            Printer::Laser1200 => &[4, 5, 6, 8, 12, 16, 20, 24],
            Printer::Inkjet => &[3, 4, 6, 8, 10, 12],
        }
    }

    /// Dot size as a percentage of the pitch.
    fn dot_fill_pct(self) -> u8 {
        match self {
            Printer::Inkjet => 60,
            _ => 70,
        }
    }

    /// Highest reasonable dot density for this printer type.
    fn max_dot_dpi(self) -> u32 {
        match self {
            Printer::Inkjet => 200,
            _ => 300,
        }
    }

    /// User-facing label.
    pub fn label(self) -> &'static str {
        match self {
            Printer::Laser600 => "Laser 600 dpi",
            Printer::Laser1200 => "Laser 1200 dpi",
            Printer::Inkjet => "Inkjet",
        }
    }
}

/// Resilience profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Profile {
    Compact,
    Balanced,
    Archival,
}

impl Profile {
    pub const ALL: [Profile; 3] = [Profile::Compact, Profile::Balanced, Profile::Archival];

    /// User-facing label.
    pub fn label(self) -> &'static str {
        match self {
            Profile::Compact => "Compact",
            Profile::Balanced => "Balanced",
            Profile::Archival => "Archival",
        }
    }

    /// User-facing one-line description.
    pub fn description(self) -> &'static str {
        match self {
            Profile::Compact => "Least paper. Needs good print and scan quality.",
            Profile::Balanced => "A sensible trade-off between page count and resilience.",
            Profile::Archival => "For long-term storage. Survives the loss of a whole page.",
        }
    }

    fn redundancy_pct(self) -> u32 {
        match self {
            Profile::Compact => 10,
            Profile::Balanced => 25,
            Profile::Archival => 35,
        }
    }

    fn max_dot_dpi(self) -> u32 {
        match self {
            Profile::Compact => 300,
            Profile::Balanced => 200,
            Profile::Archival => 150,
        }
    }

    fn survive_page_loss(self) -> bool {
        matches!(self, Profile::Archival)
    }
}

/// Optimizer input.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SuggestInput {
    /// Length of the stored stream in bytes (after compression and encryption).
    pub stream_len: u64,
    pub printer: Printer,
    /// Optical resolution of the scanner that will be available.
    pub scanner_dpi: u32,
    pub paper: Paper,
    pub margin_mm: f32,
}

impl Default for SuggestInput {
    fn default() -> Self {
        Self {
            stream_len: 0,
            printer: Printer::Laser600,
            scanner_dpi: 600,
            paper: Paper::A4,
            margin_mm: 10.0,
        }
    }
}

/// A single layout proposal.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LayoutProposal {
    pub profile: Profile,
    pub params: PrintParams,
    pub redundancy_pct: u32,
    pub survive_page_loss: bool,
    pub data_shards: u32,
    pub parity_shards: u32,
    pub pages: u32,
    pub cells_per_page: u32,
    pub dot_dpi: u32,
    /// Minimum scan resolution.
    pub min_scan_dpi: u32,
    /// How many payload bytes fit on one page.
    pub bytes_per_page: u64,
    /// Dot pitch in millimetres (for the user: "0.5 mm dot").
    pub dot_mm: f32,
}

impl LayoutProposal {
    /// Fraction of blocks that may be lost (spread across all pages).
    pub fn loss_tolerance(&self) -> f64 {
        self.parity_shards as f64 / (self.data_shards + self.parity_shards) as f64
    }

    /// Does it survive the loss of any one whole page?
    pub fn survives_page_loss(&self) -> bool {
        self.pages > 1
            && self.parity_shards
                >= max_shards_per_page(self.data_shards + self.parity_shards, self.pages)
    }
}

/// Proposes a layout for one profile. `None` if nothing fits
/// (weak scanner, small paper, backup too large).
pub fn propose(input: &SuggestInput, profile: Profile) -> Option<LayoutProposal> {
    let k = (input.stream_len.div_ceil(pabaru_format::PAYLOAD_LEN as u64)).max(1);
    if k > MAX_TOTAL_SHARDS as u64 {
        return None;
    }
    let k = k as u32;
    let printer = input.printer;
    let max_dot = profile.max_dot_dpi().min(printer.max_dot_dpi());

    let mut best: Option<LayoutProposal> = None;
    for &ppd in printer.px_per_dot_candidates() {
        let dot_dpi = printer.dpi() / ppd;
        if dot_dpi > max_dot || dot_dpi * SCAN_RATIO > input.scanner_dpi {
            continue;
        }
        let params = PrintParams {
            printer_dpi: printer.dpi(),
            px_per_dot: ppd,
            dot_fill_pct: printer.dot_fill_pct(),
            paper: input.paper,
            margin_mm: input.margin_mm,
            ..Default::default()
        };
        let cells = params.geometry().cells();
        if cells <= crate::SUPERBLOCKS_PER_PAGE + 1 {
            continue;
        }
        let (m, pages) = plan_parity(
            k,
            cells,
            profile.redundancy_pct(),
            profile.survive_page_loss(),
        );
        if k + m > MAX_TOTAL_SHARDS {
            continue;
        }
        let candidate = LayoutProposal {
            profile,
            params,
            redundancy_pct: profile.redundancy_pct(),
            survive_page_loss: profile.survive_page_loss(),
            data_shards: k,
            parity_shards: m,
            pages,
            cells_per_page: cells,
            dot_dpi,
            min_scan_dpi: dot_dpi * SCAN_RATIO,
            bytes_per_page: (cells - crate::SUPERBLOCKS_PER_PAGE) as u64
                * pabaru_format::PAYLOAD_LEN as u64,
            dot_mm: 25.4 / dot_dpi as f32,
        };
        // Fewer pages wins; on a tie, sparser dots (candidates go from the densest).
        if best.is_none_or(|b| candidate.pages <= b.pages) {
            best = Some(candidate);
        }
    }
    best
}

/// Proposes a layout for all profiles (skipping those that cannot be met).
pub fn suggest(input: &SuggestInput) -> Vec<LayoutProposal> {
    Profile::ALL
        .iter()
        .filter_map(|&p| propose(input, p))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn input(len: u64) -> SuggestInput {
        SuggestInput {
            stream_len: len,
            ..Default::default()
        }
    }

    #[test]
    fn three_proposals_for_typical_input() {
        let s = suggest(&input(500_000));
        assert_eq!(s.len(), 3);
        let (c, b, a) = (&s[0], &s[1], &s[2]);
        assert_eq!(c.profile, Profile::Compact);
        // Compact is the densest and has the fewest pages.
        assert!(c.dot_dpi >= b.dot_dpi && b.dot_dpi >= a.dot_dpi);
        assert!(c.pages <= b.pages && b.pages <= a.pages);
        assert!(a.survives_page_loss());
    }

    #[test]
    fn small_data_gets_coarse_dots() {
        // 7.2 KB – the situation from a real user test (page 14 % used).
        for p in suggest(&input(7_200)) {
            assert!(p.dot_dpi <= 75, "{:?}: {} dpi", p.profile, p.dot_dpi);
            assert!(p.dot_mm >= 0.33);
        }
        let balanced = propose(&input(7_200), Profile::Balanced).unwrap();
        assert_eq!(balanced.pages, 1);
        assert_eq!(balanced.dot_dpi, 50);
    }

    #[test]
    fn weak_scanner_limits_density() {
        let s = suggest(&SuggestInput {
            scanner_dpi: 450,
            ..input(100_000)
        });
        assert!(s.iter().all(|p| p.dot_dpi * 3 <= 450));
        assert!(!s.is_empty());
    }

    #[test]
    fn backup_too_large() {
        assert!(suggest(&input(100_000_000)).is_empty());
    }

    #[test]
    fn inkjet_has_sparser_dots() {
        let s = suggest(&SuggestInput {
            printer: Printer::Inkjet,
            scanner_dpi: 1200,
            ..input(50_000)
        });
        assert!(
            s.iter()
                .all(|p| p.dot_dpi <= 200 && p.params.dot_fill_pct == 60)
        );
    }

    proptest! {
        /// Invariants: every proposal meets the constraints and the data fits.
        #[test]
        fn proposals_meet_constraints(
            len in 1u64..3_000_000,
            scanner in prop::sample::select(vec![300u32, 600, 900, 1200, 2400]),
            printer in prop::sample::select(Printer::ALL.to_vec()),
        ) {
            let inp = SuggestInput { stream_len: len, printer, scanner_dpi: scanner, ..Default::default() };
            for p in suggest(&inp) {
                prop_assert!(p.dot_dpi * SCAN_RATIO <= scanner);
                prop_assert!(p.dot_dpi <= p.profile.max_dot_dpi());
                prop_assert_eq!(p.params.printer_dpi % p.params.px_per_dot, 0);
                prop_assert!(p.data_shards as u64 * 128 >= len);
                prop_assert!(p.parity_shards * 100 >= p.data_shards * p.redundancy_pct);
                let cap = p.cells_per_page - crate::SUPERBLOCKS_PER_PAGE;
                prop_assert!(p.data_shards + p.parity_shards <= p.pages * cap);
                if p.survive_page_loss {
                    prop_assert!(p.survives_page_loss());
                }
            }
        }
    }
}
