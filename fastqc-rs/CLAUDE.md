# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

fastqc-rs is a pure Rust rewrite of FastQC, a bioinformatics QC tool for high-throughput sequencing data. It produces byte-identical text output (`fastqc_data.txt`, `summary.txt`) to the Java version. CLI only — no GUI.

By default, links system zlib for faster gzip decompression (available on all Linux/macOS). Build with `--no-default-features` for a fully static pure-Rust binary (WASM, Windows, or environments without libz).

## Build Commands

```bash
cargo build --release              # Build optimized binary (uses system zlib by default)
cargo build --release --no-default-features  # Pure Rust, no C deps, fully static
cargo test                         # Run all tests (94 total)
cargo clippy --all-targets         # Lint — must produce zero warnings
cargo audit                        # Security audit of dependencies
```

Run a single test:
```bash
cargo test test_name               # By test function name
cargo test --test integration      # By test file (integration.rs)
cargo test --test create_test_fast5 test_fast5_single_read  # Specific test in specific file
```

Cross-compile (requires `cargo-zigbuild` and `zig`):
```bash
cargo zigbuild --release --target x86_64-unknown-linux-musl
cargo zigbuild --release --target aarch64-unknown-linux-musl
cargo zigbuild --release --target x86_64-apple-darwin
cargo zigbuild --release --target x86_64-pc-windows-gnu
```

## Comparing output against Java FastQC

```bash
# Build Java version (from repo root, not fastqc-rs/)
cd .. && ant build && cd fastqc-rs

# Run both, diff text output (skip version line)
diff <(tail -n +2 /tmp/rust_out/sample_fastqc/fastqc_data.txt) \
     <(tail -n +2 /tmp/java_out/sample_fastqc/fastqc_data.txt)
```

Text output must be byte-identical except for the version header line and minor float-precision differences in the Duplication Levels module (13th+ decimal place, caused by HashMap iteration order).

## Architecture

**Pipeline flow:** CLI args → `runner::run()` → open `SequenceFile` → feed each `Sequence` through all `QCModule`s → `finalize()` modules → generate reports (text, HTML, zip).

**Key abstractions:**
- `sequence::SequenceFile` trait — implemented by `FastQFile`, `BAMFile`, `Fast5File`
- `modules::QCModule` trait — implemented by all 12 analysis modules
- `config::FastQCConfig` — all CLI options; `config::Limits` — warn/error thresholds from `limits.txt`
- `config::LimitsExt` trait — convenience methods (`threshold()`, `is_ignored()`, `is_module_enabled()`) on the `Limits` HashMap

**Module ordering matters.** `create_modules()` in `modules/mod.rs` instantiates modules in the exact order they appear in the report. DuplicationLevel and OverRepresentedSeqs share data via `Arc<Mutex<OverRepresentedData>>` — DuplicationLevel appears before OverRepresentedSeqs in the report but reads from its data.

**Chart rendering:** Modules generate SVG strings via `report::charts::*`, which are converted to PNG via `resvg`+`tiny-skia` with a bundled Liberation Sans font (no system font dependency). Rects and lines use `shape-rendering="crispEdges"` for pixel-sharp rendering; data polylines use default antialiasing.

**Report generation:** `report::text` writes `fastqc_data.txt` and `summary.txt`. `report::html` generates the HTML report with base64-embedded PNG charts. `report::archive` creates the zip file. HTML generation happens once and is passed to the archive to avoid redundant SVG→PNG rendering.

## `// JAVA COMPAT` comments

These mark places where code does something non-idiomatic specifically to match Java's exact numeric output — integer division instead of float, Java's `Double.toString()` formatting quirks, integer arithmetic for percentile thresholds, etc. These could be simplified once byte-identical output is no longer required. There are ~24 of these, mostly in `utils/format.rs` and `utils/quality_count.rs`.

Regular comments (without the prefix) describe the algorithms and their correspondence to the Java source but don't indicate compatibility workarounds.

## Testing approach

- **Unit tests:** Inline in each module file. Cover utilities, format functions, base grouping, config parsing.
- **Integration tests** (`tests/integration.rs`): Run the full pipeline on `minimal.fastq` and `complex.fastq`, diff against approved output files in `tests/approved/`.
- **Fast5 tests** (`tests/create_test_fast5.rs`): Test HDF5 reading with synthetic Fast5 files in `tests/data/`.
- **Approved files:** `tests/approved/FileContentsTest_{minimal,complex}_fastqc_data.approved.txt` — these are the ground truth from Java FastQC. If module algorithms change, these must be updated.

## Configuration files

Embedded at compile time from `assets/` via `include_str!`/`include_bytes!`:
- `limits.txt` — module warn/error/ignore thresholds
- `adapter_list.txt`, `contaminant_list.txt` — sequence lists
- `icons/` — PNG icons for reports
- `fonts/LiberationSans-{Regular,Bold}.ttf` — bundled font for chart rendering
- `header_template.html` — CSS for HTML reports
