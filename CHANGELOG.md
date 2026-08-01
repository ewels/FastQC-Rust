# Changelog

## Unreleased

### Changes

- **Parallel analysis pipeline** for a single file. `-t/--threads` is now a total
  thread budget spread across files first and then within each file: a reader
  batches records while worker threads each run a disjoint subset of the QC
  modules over every sequence. Work is split by module rather than by data, so
  each module still sees the whole stream in file order on one thread and the
  output stays **byte-identical** to the single-threaded runner (`-t 1` is
  unchanged). Modules are balanced across workers by estimated cost so the few
  expensive ones don't cluster. Combined with parallel gzip decompression, a
  single large `.fastq.gz` now benefits from extra threads instead of being
  pinned to one core. Builds on the upstream Java three-stage pipeline
  ([s-andrews/FastQC#197](https://github.com/s-andrews/FastQC/pull/197)).
  A single file scales until the heaviest single module dominates (~4x for the
  default modules); the order-dependent modules (overrepresented sequences,
  per-sequence GC) can't be split without changing output, so beyond that extra
  cores are best spent on more files at once, which scales linearly.
- **Parallel gzip decompression is now the default** (and only) gzip reader,
  backed by [`rapidgzip-core`](https://crates.io/crates/rapidgzip-core).
  `.fastq.gz` is decompressed on a pool of background threads and overlapped
  with the analysis, giving a meaningful end-to-end speedup on large gzipped
  inputs (~1.5× end-to-end, up to ~4× on decompression alone in local tests)
  while producing byte-identical output. New `--decompress-threads N` option
  (default `0` = auto).
- **Removed the flate2/system-zlib gzip path**, the `rapidgzip`/`native-zlib`
  Cargo features, and the `FASTQC_GZIP_BACKEND` switch. The binary is now pure
  Rust (zlib-rs) with no C toolchain or system-library dependency, so builds are
  fully static by default. As a side effect, BAM/BGZF and Fast5 decompression
  (via `noodles`/`hdf5-pure`) now use the pure-Rust `miniz_oxide` backend rather
  than system zlib.

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
