# Changelog

## v0.12.1-0

Initial Rust rewrite, tracking upstream Java FastQC [v0.12.1](https://github.com/s-andrews/FastQC/releases/tag/v0.12.1).

`fastqc_data.txt` and `summary.txt` are byte-identical to the Java version across the [equivalence test suite](https://ewels.github.io/FastQC-Rust/about/equivalence/).

### Additional features

- **`--template modern`** — alternative HTML report with inline SVG charts, responsive sidebar, CSS-only help accordions, and Material Design status icons. ~13% of the classic template's size when gzipped. Ported from [upstream PR #161](https://github.com/s-andrews/FastQC/pull/161).
- **Static single-file binary** — no JVM required. Prebuilt releases for Linux (x86_64/aarch64, musl), macOS (x86_64/arm64), and Windows.
- **Bundled [Liberation Sans](https://github.com/liberationfonts/liberation-fonts) font** — chart rendering has no system font dependency. Also [ported upstream in PR #185](https://github.com/s-andrews/FastQC/pull/185).
- **Published as a Rust crate** — [`fastqc-rust`](https://crates.io/crates/fastqc-rust) for use in the Rust bioinformatics ecosystem.
- **Adapter Content trims trailing empty rows when `--min_length` is set** — also [ported upstream in PR #187](https://github.com/s-andrews/FastQC/pull/187).

### Known differences

Text output is byte-identical; differences are all in chart rendering:

- **PNG charts** rendered via [resvg](https://github.com/linebender/resvg) + [tiny-skia](https://github.com/linebender/tiny-skia) instead of Java2D. Antialiasing differs, producing ~1–2% pixel differences.
- **SVG charts** use bundled Liberation Sans instead of system Arial, so text positions shift by a few pixels.
- **HTML report** is structurally identical once embedded chart images are stripped.

None of this should affect analysis results.
