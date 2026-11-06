# Changelog

All notable changes to Pabaru. Format: [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [0.3.0] – 2026-11-06 – first public release
### Privacy
- The manifest stores only root names inside the archive, no original absolute paths. Restoring "to the original location" was removed. Older backups with the `original` field can still be read (the field is ignored).
- The PDF page header is in English and carries no content: "Pabaru paper backup | page 1/2 | ID …", block counts, `encrypted`, dpi. Neither the backup name nor file names are printed, and a name derived from the inputs is no longer stored in the superblock (only an explicit `--name`).

### Changed
- `pabaru decode` without `-o` restores into the folder of the first scan. The GUI offers "Next to scans" (default) or a chosen folder.
- Everything in English: code comments, CLI output, error messages, documentation. CLI profiles: Compact / Balanced / Archival.
- The GUI is bilingual (English/Czech), picked from the system language; override with `PABARU_LANG=en|cs`.
- Debug/test environment variables renamed: `PABARU_TRUTH`, `PABARU_SCAN`, `PABARU_COLUMN`, `PABARU_SNAPSHOTS`, `PABARU_SCREENSHOTS`.
- A freshly generated page now decodes with **zero** corrections at 300/200/150/100/50 dpi (better line detection at the frame edge, using the columns of data dots).

### Added
- Full format and decoding specification with test vectors ([docs/SPEC.md](docs/SPEC.md)) for independent implementations.
- Installation packages for macOS (.dmg), Windows (.msi, setup .exe) and Linux (.deb, .AppImage) built by GitHub Actions.

## [0.2.0] – 2026-11-05 – crumple and tear resistance
Prompted by a real test: a printed page could not be restored after being crumpled.

Results on real prints (scanner Konica Minolta, 61 blocks needed):

| Scan | 0.1 | 0.2 |
|---|---|---|
| clean | 96/96 | 96/96 |
| crumpled, 600 dpi | 0 | 67 ✓ |
| crumpled, 300 dpi | 0 | 64 ✓ |
| crumpled and torn | 0 | 56 |
| all crumpled scans combined | – | ✓ restored |

- **Coarser dots:** the optimizer goes down to 50 dpi (0.5 mm dots) when the data fit on the same number of pages.
- **Local grid tracking:** grid nodes and points along lines are found locally; a cell is a Coons patch built from its edges.
- **Adaptive cell reading:** a field of offsets inside each cell derived from sample variance – handles large cells on wavy paper.
- **RS with erasures:** uncertain bytes are passed as erasures → up to 60 damaged bytes per block can be fixed instead of 30.
- **Fallback attempts:** straight edges, shifts, corners from the global model (all verified by RS + CRC).
- **Frameless grid detection:** torn pieces (even scanned separately) are decoded when less than 30 % is read using the frame.
- **4 superblock copies** per page (previously 2).
- Crumple and tear **simulator**: `pabaru verify --crumple medium|strong --tear`.

Known limits: strong crumpling with fine dots (200 dpi) can't be read – hence coarse dots; decoding a crumpled A4 page takes ~4 s.

## [0.1.0] – 2026-11-05 – first working version
- Format v1: tar + zstd, BLAKE3 manifest, Argon2id + XChaCha20-Poly1305, inner RS(200,140) over GF(256), outer erasure code across blocks and pages, whitening, 8 scan orientations.
- `pabaru-core` pipeline and CLI (`encode`, `decode`, `inspect`, `verify`, `plan`), layout optimizer with Compact/Balanced/Archival profiles, scans from PDF.
- Desktop GUI (egui) with drag & drop, layout proposals, preview, PDF/PNG export and restore; tested with `egui_kittest`.
- Scan decoder (frame, grid, homography, sampling) and a print/scan simulator for tests.

[0.3.0]: https://github.com/j3corre/pabaru/releases/tag/v0.3.0
