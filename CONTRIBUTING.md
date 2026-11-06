# Contributing to Pabaru

Thanks for your interest! Bug reports, test scans and pull requests are welcome.

## Reporting problems
- For a scan that fails to restore, please attach (if the content isn't private) the scan, the original PDF, and the output of `pabaru inspect <scan>`. Mention the printer, scanner, scan resolution and paper.
- Never share scans of backups containing private data. A test backup of dummy files is just as useful.

## Development
```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
On Linux the GUI needs: `libxkbcommon-dev libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev libxcb-xfixes0-dev`.

## Rules
- **The printed format v1 is frozen.** Any change that affects what is printed or how it is decoded must keep old backups readable, update [docs/SPEC.md](docs/SPEC.md) and the test vectors in `crates/pabaru-core/tests/spec_vectors.rs`.
- Code, comments and commit messages in English. GUI texts are bilingual: `tr("English", "Česky")` in `crates/pabaru-gui`.
- New dependencies must have a permissive license (checked by `cargo deny check licenses`, see `deny.toml`).

## License
By contributing you agree that your contributions are dual licensed under MIT OR Apache-2.0, like the rest of the project.
