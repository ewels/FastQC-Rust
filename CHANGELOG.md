# Changelog

## v1.0.2dev

> [!NOTE]
> Tracking: FastQC [v0.13.0](https://github.com/s-andrews/FastQC/releases/tag/v0.13.0)

### Upstream v0.13.0 changes

Output matches Java FastQC v0.13.0. Many of these changes came from this project: see [upstream contributions](https://ewels.github.io/FastQC-Rust/about/upstream/).

- **Phred+33 is the default.** Encoding autodetection is gone. Use the new `--phred64` flag for legacy Phred+64 files. A warning is printed if the data has no quality below Q31, and `--phred64` fails the file if a quality character is below 64.
- **Basic Statistics** has new `Mean Length` and `Median Length` rows. The `Sequences flagged as poor quality` row is gone, and `Total Sequences` now includes CASAVA-filtered reads.
- **`--min_length` is now a read filter**: shorter reads are discarded before analysis. It no longer pads per-base plots. The new `--max_length` discards longer reads.
- **SVG graphs are the default** in the HTML report. Use `--png` for PNG. `--svg` is still accepted and does nothing. SVG files are now always written to the zip, in the same compact format as Java (polylines, merged rectangles, 2px lines).
- **Overrepresented sequences** percentages are rounded to 2 decimal places, as in Java v0.13.0.
- Empty input (for example, every read removed by `--min_length`) gives the same output as Java.
- A warning is printed if the adapter sequences have different lengths.

### Bug fixes

- Use Sanger / Illumina 1.9 encoding for BAM/SAM input instead of inferring it from the lowest quality character ([#6](https://github.com/ewels/FastQC-Rust/issues/6), [#10](https://github.com/ewels/FastQC-Rust/pull/10)). BAM/SAM quality is Phred+33 by specification. Java FastQC v0.13.0 now defaults to Phred+33 for all input, so the only divergence left is that `--phred64` has no effect on BAM/SAM input. The matching Java PR ([s-andrews/FastQC#210](https://github.com/s-andrews/FastQC/pull/210)) was closed as superseded by v0.13.0.
- Per-base charts (quality, sequence content, N content, adapter, Kmer, length distribution, per-tile) are now `max(800, groups × 15)` px wide, as in Java. Before, they were always 800px, which squashed long-read and `--nogroup` plots.
- Adapter Content shows "Can't analyse adapters as read length is too short", as in Java, when no read is longer than the longest adapter. Overrepresented sequences shows "No overrepresented sequences" when there are none.
- `--template` no longer uses the `-t` short flag, which clashed with `--threads`.
- Fixed the nightly upstream check, which failed because the `upstream-update` label did not exist.
- Sequence Duplication Levels values are now deterministic and match Java to the last digit. They were summed in Rust's randomly seeded `HashMap` order, so the last 2–3 digits of `#Total Deduplicated Percentage` and the level percentages could change between runs on large inputs; they are now summed in Java's `HashMap` iteration order.

### Other

- New equivalence test cases for mixed read lengths, `--min_length`/`--max_length`, empty input and Phred+64 (26 cases in total).
- `generate_reference.sh` works with an unpacked upstream release zip. The new `update_svg_patches.py` regenerates the SVG patches.
- New docs page: [upstream contributions](https://ewels.github.io/FastQC-Rust/about/upstream/).

## v1.0.1

> [!NOTE]
> Tracking: FastQC [v0.12.1](https://github.com/s-andrews/FastQC/releases/tag/v0.12.1)

### Bug fixes

- Emit unrounded percentage in Overrepresented sequences for Java byte-identical output ([#2](https://github.com/ewels/FastQC-Rust/pull/2))

### Other

- Added docs for the Rust library
- New, slightly less minimal, test FastQ file
- Equivalence reports should now be attached to releases as an asset

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
