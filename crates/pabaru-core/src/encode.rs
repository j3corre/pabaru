//! Encoding: files and folders → pages to print.

use std::fs;
use std::path::{Path, PathBuf};

use pabaru_crypto::KdfConfig;
use pabaru_ecc::outer;
use pabaru_format::{BLOCK_LEN, Block, BlockHeader, Flags, KdfParams, PAYLOAD_LEN, Superblock};
use pabaru_layout::paginate::max_shards_per_page;
use pabaru_layout::{CellContent, DotGrid, PageGeometry, PrintParams, paginate};
use pabaru_render::pdf::{PdfDocument, PdfPage};
use pabaru_render::{Bitmap, rasterize};
use rand::RngCore;

use crate::{CoreError, short_hash};
use pabaru_layout::{SUPERBLOCKS_PER_PAGE, plan_parity};

/// Backup settings.
#[derive(Debug, Clone)]
pub struct EncodeOptions {
    pub params: PrintParams,
    /// Minimum redundancy as a percentage of the number of data blocks
    /// (e.g. 20 = at least 20 parity blocks per 100 data blocks).
    pub redundancy_pct: u32,
    /// Guarantee recovery even after losing one whole page.
    pub survive_page_loss: bool,
    /// Encryption password (None = no encryption).
    pub password: Option<String>,
    pub kdf: KdfConfig,
    /// zstd compression level (1–22).
    pub zstd_level: i32,
    /// Optional user-chosen backup label, stored in the superblock (empty = none).
    pub name: String,
}

impl EncodeOptions {
    /// Settings based on an optimizer proposal.
    pub fn from_proposal(p: &pabaru_layout::LayoutProposal) -> Self {
        Self {
            params: p.params,
            redundancy_pct: p.redundancy_pct,
            survive_page_loss: p.survive_page_loss,
            ..Default::default()
        }
    }
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            params: PrintParams::default(),
            redundancy_pct: 20,
            survive_page_loss: false,
            password: None,
            kdf: KdfConfig::default(),
            zstd_level: 19,
            name: String::new(),
        }
    }
}

/// Summary information about a backup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupInfo {
    pub backup_id: u32,
    pub name: String,
    pub file_count: usize,
    /// Size of the files before compression.
    pub raw_bytes: u64,
    /// Size of the stored stream (after compression and optional encryption).
    pub stream_len: u64,
    pub data_shards: u32,
    pub parity_shards: u32,
    pub pages: u32,
    pub cells_per_page: u32,
    pub encrypted: bool,
    /// Highest number of shards on a single page.
    pub max_shards_per_page: u32,
}

impl BackupInfo {
    /// Does the backup survive the loss of any one whole page?
    pub fn survives_page_loss(&self) -> bool {
        self.pages > 1 && self.parity_shards >= self.max_shards_per_page
    }
}

/// A finished backup ready for rendering. Blocks are already encoded;
/// page bitmaps are rendered only on demand (saves memory).
#[derive(Debug, Clone)]
pub struct EncodedBackup {
    pub info: BackupInfo,
    pub params: PrintParams,
    pub geometry: PageGeometry,
    /// Encoded blocks of each page (in grid row order).
    pages: Vec<Vec<[u8; BLOCK_LEN]>>,
}

/// Prepared (packed and compressed) data – the first, slower step of
/// encoding. The GUI runs it once after a folder is dropped and then only
/// tries different parameters without compressing again.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub inputs: Vec<PathBuf>,
    packed: pabaru_archive::Packed,
}

impl Prepared {
    pub fn file_count(&self) -> usize {
        self.packed.manifest.files.len()
    }
    /// Size of the files before compression.
    pub fn raw_bytes(&self) -> u64 {
        self.packed.raw_bytes
    }
    /// Size after compression.
    pub fn packed_len(&self) -> u64 {
        self.packed.stream.len() as u64
    }
    /// Length of the stored stream (encryption adds a nonce and a tag).
    pub fn stream_len(&self, encrypted: bool) -> u64 {
        self.packed_len()
            + if encrypted {
                pabaru_crypto::OVERHEAD as u64
            } else {
                0
            }
    }
    /// Default backup name (for display only).
    pub fn default_name(&self) -> String {
        default_name(&self.inputs)
    }
}

/// Maximum size of the input data before compression.
pub const MAX_INPUT_BYTES: u64 = 256_000_000;

/// Packs and compresses the inputs.
pub fn prepare(inputs: &[PathBuf], zstd_level: i32) -> Result<Prepared, CoreError> {
    // Memory guard: files are read into RAM and must fit into ~8 MB after
    // compression anyway. Huge folders are rejected before we start loading them.
    let summary = pabaru_archive::summarize(inputs)?;
    if summary.total_bytes > MAX_INPUT_BYTES {
        return Err(CoreError::InvalidParams(format!(
            "the data has {} MB, at most {} MB (before compression) can be backed up to paper",
            summary.total_bytes / 1_000_000,
            MAX_INPUT_BYTES / 1_000_000
        )));
    }
    Ok(Prepared {
        inputs: inputs.to_vec(),
        packed: pabaru_archive::pack(inputs, zstd_level)?,
    })
}

/// Encodes files and folders into a backup (= [`prepare`] + [`encode_prepared`]).
pub fn encode(inputs: &[PathBuf], opts: &EncodeOptions) -> Result<EncodedBackup, CoreError> {
    encode_prepared(&prepare(inputs, opts.zstd_level)?, opts)
}

/// Encodes prepared data with the given parameters.
pub fn encode_prepared(
    prepared: &Prepared,
    opts: &EncodeOptions,
) -> Result<EncodedBackup, CoreError> {
    encode_packed_with_id(&prepared.packed, opts, rand::rngs::OsRng.next_u32())
}

/// Encodes an already packed stream with a caller-chosen `backup_id`.
///
/// Not part of the stable API: it exists so that the test vectors in
/// `docs/SPEC.md` can be reproduced bit for bit. Without a password the output
/// is fully deterministic; with a password the salt and nonce are still random.
/// Normal callers use [`encode_prepared`], which picks a random `backup_id`.
#[doc(hidden)]
pub fn encode_packed_with_id(
    packed: &pabaru_archive::Packed,
    opts: &EncodeOptions,
    backup_id: u32,
) -> Result<EncodedBackup, CoreError> {
    let geometry = opts.params.geometry();
    let cells = geometry.cells();
    if cells <= SUPERBLOCKS_PER_PAGE {
        return Err(CoreError::InvalidParams(
            "not enough blocks fit on a page (check paper, margins and DPI)".into(),
        ));
    }
    if opts.params.px_per_dot == 0 {
        return Err(CoreError::InvalidParams(
            "px_per_dot must be at least 1".into(),
        ));
    }

    // 2) Optional encryption.
    let mut kdf = KdfParams::default();
    let stream = match &opts.password {
        Some(pw) => {
            kdf = KdfParams {
                salt: pabaru_crypto::random_salt(),
                mem_kib: opts.kdf.mem_kib,
                iterations: opts.kdf.iterations,
                parallelism: opts.kdf.parallelism,
            };
            pabaru_crypto::encrypt(&packed.stream, pw, &kdf.salt, opts.kdf)?
        }
        None => packed.stream.clone(),
    };

    // 3) Split into shards and compute parity.
    let k = stream.len().div_ceil(PAYLOAD_LEN).max(1) as u32;
    let (m, page_count) = plan_parity(k, cells, opts.redundancy_pct, opts.survive_page_loss);
    let total = (k + m) as usize;
    if total > outer::MAX_TOTAL_SHARDS {
        return Err(CoreError::TooLarge {
            shards: total,
            max: outer::MAX_TOTAL_SHARDS,
        });
    }
    let data: Vec<Vec<u8>> = (0..k as usize)
        .map(|i| {
            let mut shard = vec![0u8; PAYLOAD_LEN];
            let start = i * PAYLOAD_LEN;
            let end = (start + PAYLOAD_LEN).min(stream.len());
            if start < end {
                shard[..end - start].copy_from_slice(&stream[start..end]);
            }
            shard
        })
        .collect();
    let parity = outer::encode(&data, m as usize)?;

    // 4) Metadata.
    // Privacy: the name derived from input files is NOT stored on paper.
    // Only an explicit, user-chosen label (`opts.name`) goes into the superblock.
    let name = opts.name.clone();
    let superblock = |page_index: u16| Superblock {
        flags: Flags {
            compressed: true,
            encrypted: opts.password.is_some(),
        },
        stream_len: stream.len() as u64,
        data_shards: k,
        parity_shards: m,
        page_index,
        page_count: page_count as u16,
        cols: geometry.cols as u16,
        rows: geometry.rows as u16,
        stream_hash: short_hash(&stream),
        kdf,
        name: name.clone(),
    };

    // 5) Distribute blocks onto pages.
    let layout = paginate(k + m, cells, SUPERBLOCKS_PER_PAGE, backup_id as u64);
    debug_assert_eq!(layout.len() as u32, page_count);
    let pages = layout
        .iter()
        .enumerate()
        .map(|(p, page)| {
            let sb_block = Block {
                header: BlockHeader {
                    backup_id,
                    index: pabaru_format::SUPERBLOCK_INDEX,
                },
                payload: superblock(p as u16).to_payload(),
            }
            .encode();
            page.iter()
                .map(|cell| match cell {
                    CellContent::Superblock => sb_block,
                    CellContent::Shard(i) => {
                        let i = *i as usize;
                        let src = if i < k as usize {
                            &data[i]
                        } else {
                            &parity[i - k as usize]
                        };
                        let mut payload = [0u8; PAYLOAD_LEN];
                        payload.copy_from_slice(src);
                        Block {
                            header: BlockHeader {
                                backup_id,
                                index: i as u32,
                            },
                            payload,
                        }
                        .encode()
                    }
                })
                .collect()
        })
        .collect();

    let info = BackupInfo {
        backup_id,
        name,
        file_count: packed.manifest.files.len(),
        raw_bytes: packed.raw_bytes,
        stream_len: stream.len() as u64,
        data_shards: k,
        parity_shards: m,
        pages: page_count,
        cells_per_page: cells,
        encrypted: opts.password.is_some(),
        max_shards_per_page: max_shards_per_page(k + m, page_count),
    };
    Ok(EncodedBackup {
        info,
        params: opts.params,
        geometry,
        pages,
    })
}

/// Default backup name: name of the first input (+ the number of other inputs).
fn default_name(inputs: &[PathBuf]) -> String {
    let first = inputs
        .first()
        .and_then(|p| std::path::absolute(p).ok())
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "backup".into());
    match inputs.len() {
        0 | 1 => first,
        n => format!("{first} +{}", n - 1),
    }
}

impl EncodedBackup {
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Renders page `i` into a bitmap at printer resolution.
    pub fn render_page(&self, i: usize) -> Bitmap {
        let g = &self.geometry;
        let grid = DotGrid::build(g.cols, g.rows, &self.pages[i]);
        rasterize(g, &grid, self.params.dot_fill_pct)
    }

    /// Human-readable header of page `i`.
    pub fn header_lines(&self, i: usize) -> Vec<String> {
        let info = &self.info;
        let dot_dpi = self.params.dot_dpi().round() as u32;
        vec![
            // No backup name or file names here – the header must not reveal content.
            format!(
                "Pabaru paper backup | page {}/{} | ID {:08X}",
                i + 1,
                info.pages,
                info.backup_id
            ),
            format!(
                "Blocks: {} data + {} parity{} | dots {} dpi | print at 100% scale | scan at >= {} dpi in grayscale",
                info.data_shards,
                info.parity_shards,
                if info.encrypted { " | encrypted" } else { "" },
                dot_dpi,
                dot_dpi * 3
            ),
            "Restore: pabaru decode <scans> [-o <folder>]".into(),
        ]
    }

    /// Writes the whole backup as a PDF.
    pub fn write_pdf(&self, path: &Path) -> Result<(), CoreError> {
        let mut doc = PdfDocument::new(self.params.printer_dpi);
        for i in 0..self.page_count() {
            let bmp = self.render_page(i);
            doc.add_page(&PdfPage {
                bitmap: &bmp,
                header_lines: self.header_lines(i),
                margin_mm: self.params.margin_mm,
            })?;
        }
        fs::write(path, doc.finish())?;
        Ok(())
    }

    /// Writes each page as a PNG into folder `dir`
    /// (`<stem>-001.png`, `<stem>-002.png`, …). Returns the file paths.
    pub fn write_pngs(&self, dir: &Path, stem: &str) -> Result<Vec<PathBuf>, CoreError> {
        fs::create_dir_all(dir)?;
        let mut out = Vec::new();
        for i in 0..self.page_count() {
            let path = dir.join(format!("{stem}-{:03}.png", i + 1));
            let file = std::io::BufWriter::new(fs::File::create(&path)?);
            pabaru_render::png::write_png(file, &self.render_page(i), self.params.printer_dpi)?;
            out.push(path);
        }
        Ok(out)
    }
}
