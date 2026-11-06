# Pabaru architecture

The printed format itself is specified in [SPEC.md](SPEC.md). This document describes how the code is organised.

## Modules (Cargo workspace)

```
          pabaru-cli        pabaru-gui (egui)
                \              /
                 \            /
                  pabaru-core  (encode/decode pipeline, the only public API)
   ┌──────┬──────┬───┴───┬────────┬────────┬───────┐
archive crypto  ecc   format   layout   render   scan
```

| Crate | Responsibility |
|-------|----------------|
| `pabaru-format` | Binary format: superblock, data block, CRC32C, whitening. No I/O. Depends on `pabaru-ecc` (inner RS). |
| `pabaru-ecc` | Inner RS over GF(256) (corrects bad bytes within a block) and the outer erasure code (`reed-solomon-simd`) across blocks. |
| `pabaru-archive` | Files and folders → tar + zstd, manifest with BLAKE3, restoring the folder structure. |
| `pabaru-crypto` | Argon2id + XChaCha20-Poly1305 (optional encryption). |
| `pabaru-layout` | Page geometry, block placement, parameter optimizer (`suggest`). |
| `pabaru-render` | Rasterising a page into a 1-bit bitmap, PNG and PDF export at an exact DPI. |
| `pabaru-scan` | Scan decoding: thresholding, frame and grid detection, homography, bit sampling. |
| `pabaru-core` | Ties the modules into the `encode` / `decode` / `verify` pipeline. |
| `pabaru-cli` | Command-line interface (clap). |
| `pabaru-gui` | Desktop GUI (eframe/egui) with drag & drop and a view-model. |

Dependencies point in one direction only, and only `pabaru-gui` depends on egui.

## Encoding pipeline
1. **archive**: tar + zstd.
2. **crypto**: optional AEAD encryption.
3. **Splitting** into 128-byte payloads and computing parity payloads with the outer erasure code.
4. **format**: each payload gets a header and CRC, then goes through the inner RS and whitening.
5. **layout**: blocks are placed on pages (pseudo-randomly, from a seed); the superblock is replicated on every page.
6. **render**: drawing to PNG or PDF.

Decoding runs in reverse. Missing or uncorrectable blocks are treated as *erasures* by the outer code.

## Block geometry (fixed for format v1)
- Data area of 40×40 dots = 1600 bits = 200 bytes, bordered by 1-dot grid lines shared with the neighbouring cells.
- Content: 8-byte header + 128-byte payload + 4-byte CRC32C = 140 bytes, plus 60 bytes of RS parity.
- The inner RS(200,140) corrects up to 30 bad bytes per block (up to 60 when the positions are known as erasures).
- Whitening: the whole 200-byte word is XORed with a fixed xorshift32 sequence (seed `0x50414252`). The seed is part of the specification and must not change.
- The superblock has index `0xFFFFFFFF` and contains the magic `PBRU`, version, flags, k/m, page, grid, BLAKE3 hash of the stream, Argon2 parameters and an optional name (see `crates/pabaru-format/src/superblock.rs`).

## Page layout (pabaru-layout)
- A cell has a pitch of 41 dots: 1 solid grid line + 40 data dots.
- The grid is surrounded by a thick frame (3 dots) and a white quiet zone (4 dots).
- The solid grid lines let the decoder find and precisely locate blocks.
- Every page carries at least 4 copies of the superblock (2 in backups made by 0.1); free cells are filled with extra parity.
- Shards are distributed evenly (at most 1 difference per page) and their positions are shuffled pseudo-randomly.
- Bytes map to dots row by row, MSB first, black = 1 (`pabaru-format/src/dots.rs`).
- The decoder finds the page orientation by trying all 8 symmetries of the square (4 rotations × mirroring) and keeping the one where RS/CRC succeed. The grid is square, so no special orientation mark is needed.
- Grid lines are drawn with the same thickness and at the same positions as data dots: a 2 px dot in a 3 px pitch cannot be centred, and the decoder aligns dots to the lines.

## Export (pabaru-render)
- A 1-bit bitmap of the whole page at the printer resolution; data dots are shrunk to `dot_fill_pct`.
- PNG with DPI in `pHYs`, and a small custom PDF writer (image at 1:1, header in Helvetica, ASCII only).

## Scan decoding (pabaru-scan)
1. **With the frame:** frame → number of cells → initial grid nodes from a homography.
2. **Local node tracking** (`lattice::track`): best-first from the most confident node; a neighbour is predicted as node + local grid vector, then refined by a 1D search across the line ("ridge"). Boundary nodes (`refine_frame_boundary`) are extrapolated from the interior and refined using the columns of data dots; a "step" detector on the inner edge of the frame is kept as a fallback geometry.
3. **Points along lines** (`refine_edges`, 7 per edge): cross profile of darkness (±8 dots along the line), maximum + centroid in a ±0.6-dot window.
4. **Cell = Coons patch** built from the polyline edges (`Patch::point`).
5. **Chain of attempts** per cell (each result is verified by RS + CRC32C):
   1. normal read,
   2. adaptive read – a field of offsets at anchors every 4 and 3 dots; energy = variance of samples in a 7×7 window (the right phase gives clearly black/white samples); two passes (chain from the edge + symmetric),
   3. straight cell edges,
   4. small shifts of ±0.4 dot,
   5. corners from the global model with a ±1-dot grid of offsets.
   Every read is also tried with **erasures** of the 16/32/44/56 least confident bytes (confidence of a dot = distance from the threshold).
6. **Without the frame** (when less than 30 % of blocks were read using the frame, `seed.rs`): each large dark region (a torn piece) is handled separately – angle from the "peakiness" of the projection, dot pitch from autocorrelation, lines from a ridge search in a central 900 px window; results are merged by block index.
7. **Orientation:** the 8 symmetries are tried on a few cells and the one with the most RS+CRC successes wins.
8. **Simulator** (`simulate`): produces a "scan" from a bitmap with rotation, scaling, stretching, blur, noise, stains, crumpling and tears. Used by tests and `pabaru verify`.

## Optimizer (pabaru-layout::suggest)
- Input: stream length, printer (`Laser600`, `Laser1200`, `Inkjet`), scanner resolution, paper, margin.
- Profiles: Compact (10 %, max 300 dpi), Balanced (25 %, max 200 dpi), Archival (35 %, max 150 dpi, survives losing a page).
- Dot densities = printer DPI / integer. Constraints: scan ≥ 3× dot density; inkjet ≤ 200 dpi and 60 % fill.
- The smallest page count wins; on a tie, coarser dots (down to 50 dpi, i.e. 0.5 mm dots) are preferred because they survive crumpling. Invariants are checked by proptest.

## Scans from PDF (pabaru-scan::pdf_input)
- From each page the largest embedded image is taken (`lopdf`): DCTDecode (JPEG), FlateDecode/uncompressed, 1 or 8 bits, grey/RGB, `/Decode [1 0]`. JBIG2, CCITT and JPEG2000 are not supported.

## Pipeline (pabaru-core)
- `prepare(inputs, level) -> Prepared` (packing + compression, done once) and `encode_prepared(&Prepared, &EncodeOptions)`, so the GUI can try parameters without recompressing.
- `encode(inputs, &EncodeOptions) -> EncodedBackup` packs, encrypts, splits into 128-byte shards, computes parity and places the blocks. Pages are rendered on demand (`render_page`, `write_pdf`, `write_pngs`), so the whole backup is never held in memory as bitmaps.
- `plan_parity(k, cells, pct, survive)`:
  1. parity of at least `pct` %,
  2. free cells on the last page are filled with parity,
  3. to survive losing a page, `m ≥ max shards on one page` (a page is added if needed).
- `Decoder` collects blocks from any scans and groups them by `backup_id`, so blocks of different backups never mix. `progress()` reports read and needed blocks and missing pages. `restore(password, Destination)` restores files and verifies hashes.
- `verify()` runs encode → simulated print and scan → decode → restore into a temporary folder.

## CLI (pabaru-cli)
- A thin layer over `pabaru-core`: `encode`, `decode`, `inspect`, `verify`, `plan`.
- `decode` without `-o` restores into the folder of the first scan.
- The password is entered interactively (`rpassword`) or via `--password-env VARIABLE`.

## GUI (pabaru-gui)
Layers:
- `model.rs`: state and logic without egui (`BackupModel`, `RestoreModel`, `Task<T>`), covered by unit tests.
- `worker.rs`: long-running tasks (data preparation, export, scan decoding, restore) in threads; results come back over a channel, redraw via `request_repaint`. An input generation counter discards stale results.
- `views/`: drawing of the screens. Side effects are returned as `Action`s (dialogs, tasks) and executed by `app.rs`.
- `widgets.rs` + `theme.rs`: cards, drop zone, chips, painted icons and a palette for light and dark themes.
- `dialogs.rs`: native dialogs (`rfd`) behind the `Dialogs` trait, replaced by a fake in tests.
- `i18n.rs`: English + Czech. The language comes from the system (`sys-locale`, override `PABARU_LANG`); texts sit right at the call site as `tr("English", "Česky")`. Tests run in English; `tests/gui_cs.rs` (a separate process) checks Czech.
- Restore destination: `DestChoice::NextToScans` (default, folder of the first scan) or `DestChoice::Folder`.

Tests: `egui_kittest` simulates drag & drop (`RawInput::dropped_files`), clicks by AccessKit labels and checks export and restore.

## Archive and paths (pabaru-archive)
- Several files and folders can be packed at once. Names in the archive are relative to their common ancestor, so folders with the same name don't overwrite each other.
- The manifest only contains root names inside the archive, **not** the original absolute paths (privacy). Restore always goes into a chosen folder (`Destination::Folder`).
- Extraction rejects dangerous paths (`..`, absolute paths).

## Resilience
- The outer code is global across all blocks (up to 65,535 shards ≈ 8 MB). Larger backups are not supported yet (interleaved groups are planned).
- **Archival profile invariant:** the number of parity blocks ≥ the number of blocks on one page, so the backup survives losing a whole page.

## Testing
- Unit tests and proptest round-trips for every crate.
- Scan degradation tests (rotation, noise, blur, stains, page loss, crumpling, tearing) and a test that a generated page decodes with zero corrections.
- CLI integration tests via `assert_cmd`.
- GUI via `egui_kittest`.
- Spec test vectors (`crates/pabaru-core/tests/spec_vectors.rs`) keep [SPEC.md](SPEC.md) in sync with the code.
