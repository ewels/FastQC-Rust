# Changelog

## v1.0.0

> [!NOTE]
> Tracking: FastQC [v0.12.1](https://github.com/s-andrews/FastQC/releases/tag/v0.12.1)

Initial Rust rewrite.

### Comparison to upstream

- `fastqc_data.txt` and `summary.txt` are byte-identical to FastQC v0.12.1
    - Only known exception: **Adapter Content** trims trailing empty rows when `--min_length` is set. Upstream PR: [#187](https://github.com/s-andrews/FastQC/pull/187).
- **PNG charts** rendered via [resvg](https://github.com/linebender/resvg) + [tiny-skia](https://github.com/linebender/tiny-skia) instead of Java2D. Antialiasing differs, producing ~1–2% pixel differences.
- **SVG charts** use bundled Liberation Sans instead of system Arial, so text positions shift by a few pixels.
- **HTML report** is identical once embedded chart images are stripped.
- No "interactive mode" (upstream launched an interactive Java GUI if run without any arguments)

See the [equivalence test suite](https://ewels.github.io/FastQC-Rust/about/equivalence/) for details.

### Additional features

- **`--template modern`** — alternative HTML report with inline SVG charts, responsive sidebar, CSS-only help accordions, and Material Design status icons. ~13% of the classic template's size when gzipped. Upstream PR: [#161](https://github.com/s-andrews/FastQC/pull/161).
- **Bundled [Liberation Sans](https://github.com/liberationfonts/liberation-fonts) font** — chart rendering has no system font dependency. Upstream PR: [#185](https://github.com/s-andrews/FastQC/pull/185).
- **Static single-file binary** — no JVM required. Prebuilt releases for Linux (x86_64/aarch64, musl), macOS (x86_64/arm64), and Windows.
- **Published as a Rust crate** — [`fastqc-rust`](https://crates.io/crates/fastqc-rust) for use in the Rust bioinformatics ecosystem.
