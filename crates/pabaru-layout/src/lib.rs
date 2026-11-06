//! # pabaru-layout
//!
//! A purely computational crate (no images or files):
//!
//! * [`geometry`] – how many blocks fit on the paper with the given print parameters,
//! * [`paginate`] – which blocks go on which page and at which position,
//! * [`grid`] – the "dot map" of a page's data area (what will be black).
//!
//! ## Structure of the data area (in dots)
//!
//! ```text
//!  quiet (white margin, 4 dots)
//!  ┌──────────────────────────────┐  <- frame 3 dots (grid line + 2 extra dots)
//!  │┌─────┬─────┬─────┐           │
//!  ││block│block│block│  ...      │  <- cell = 1 line + 40×40 data dots (pitch 41)
//!  │├─────┼─────┼─────┤           │
//! ```
//!
//! The grid lines are solid, so the decoder can easily find them in a scan
//! and use them to locate each block precisely.

pub mod geometry;
pub mod grid;
pub mod paginate;
pub mod suggest;

pub use geometry::{PageGeometry, Paper, PrintParams};
pub use grid::DotGrid;
pub use paginate::{CellContent, paginate, plan_parity};
pub use suggest::{LayoutProposal, Printer, Profile, SuggestInput, propose, suggest};

/// Cell pitch in dots: 1 grid line + 40 data dots.
pub const CELL_DOTS: u32 = pabaru_format::BLOCK_DOTS as u32 + 1;
/// How many dots thicker the outer frame is than a normal grid line (outwards on each side).
pub const FRAME_EXTRA: u32 = 2;
/// Number of superblock copies on each page (at least). A page cannot be
/// decoded without a superblock, hence 4 copies at random positions
/// (crumpling, folds and stains destroy only some of them).
pub const SUPERBLOCKS_PER_PAGE: u32 = 4;
/// Width of the white margin around the frame, in dots.
pub const QUIET: u32 = 4;
