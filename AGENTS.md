# AGENTS.md

## Cursor Cloud specific instructions

This is a single-binary Rust CLI project with no runtime services. See `CLAUDE.md` for full build/test/lint commands and architecture details.

### Quick reference

- **Build:** `cargo build --release` (links system zlib by default)
- **Lint:** `cargo clippy --all-targets` (must produce zero warnings)
- **Test:** `cargo test` (runs 99 unit + integration + Fast5 tests)
- **Run:** `./target/release/fastqc -o <outdir> <input.fastq>` (output dir must exist)
- **Equivalence tests:** `uv run tests/equivalence/compare.py --binary ./target/release/fastqc`

### Environment notes

- Rust MSRV is **1.88.0** (defined in `Cargo.toml`). The update script keeps Rust stable current via `rustup update stable`.
- System `zlib1g-dev` is required for the default `native-zlib` feature. It is pre-installed in the VM image. If missing, build with `--no-default-features` for a pure-Rust fallback.
- `uv` is required for equivalence tests (Python scripts use PEP 723 inline deps). The update script installs it if absent.
- `source $HOME/.local/bin/env` is needed in new shells to put `uv` on `PATH`.
- There is no `rust-toolchain.toml`; the project uses whatever stable toolchain `rustup` provides as long as it meets the MSRV.
- The release profile uses `lto = "fat"` and `codegen-units = 1`, so release builds are slow (~60s). Use debug builds (`cargo build`) for iteration; reserve `--release` for final testing and equivalence tests.
