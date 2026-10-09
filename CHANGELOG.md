# Changelog

## v1.1.0

> [!NOTE]
> Tracking: FastQC [v0.13.0](https://github.com/s-andrews/FastQC/releases/tag/v0.13.0)

### Upstream v0.13.0

Output matches Java FastQC v0.13.0 ([#11](https://github.com/ewels/FastQC-Rust/pull/11)). Many of its changes came from this project: see [upstream contributions](https://ewels.github.io/FastQC-Rust/about/upstream/).

- **Phred+33 is the default**; encoding autodetection is gone. Use `--phred64` for legacy files.
- **Basic Statistics** gains `Mean Length` and `Median Length`, and drops `Sequences flagged as poor quality`.
- **`--min_length` now filters reads** instead of padding plots. New `--max_length` does the same for long reads.
- **SVG graphs are the default** in the HTML report. Use `--png` for PNG.

### Faster

- **Multi-threaded by default.** `-t/--threads` is now a total budget for the whole run (default: available CPUs, up to 6), so a single `.fastq.gz` uses several cores. `-t 1` keeps everything, gzip decoding included, on one thread. ([#7](https://github.com/ewels/FastQC-Rust/pull/7), [#8](https://github.com/ewels/FastQC-Rust/pull/8))
- **Parallel gzip decompression** via [rapidgzip](https://crates.io/crates/rapidgzip-core). Binaries are now pure Rust and fully static.
- **Quicker analysis** ([#13](https://github.com/ewels/FastQC-Rust/pull/13)): about 2x faster on a 7.9 GB WES file and 3x on long reads, with much lower memory on long reads. See the [benchmarks](https://ewels.github.io/FastQC-Rust/about/performance/).

Output stays byte-identical whatever the thread count.

### Live progress display

Per-file progress bars with a live Basic Statistics table replace the `Approx N% complete` lines ([#9](https://github.com/ewels/FastQC-Rust/pull/9)). Logs and pipes get plain one-line-per-file output; override with `FASTQC_PROGRESS=always|never`.

### Bug fixes

- BAM/SAM input always uses Phred+33 ([#10](https://github.com/ewels/FastQC-Rust/pull/10))
- Sequence Duplication Levels are deterministic and match Java on large inputs ([#12](https://github.com/ewels/FastQC-Rust/pull/12))
- Per-base charts widen for long reads and `--nogroup`, as in Java
- `--template` no longer uses `-t`, which clashed with `--threads`
- Quality characters of 128 or above no longer crash the run

### Breaking changes for library users

`FastQCConfig::threads` is now `Option<usize>`, `FastQCConfig` has a new `decompress_threads` field, and `SequenceFileGroup::new` takes file openers. The `native-zlib` feature is gone. See the [library docs](https://ewels.github.io/FastQC-Rust/library/).

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
