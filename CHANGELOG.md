# Changelog

## Unreleased

### Changes

- **Live progress display.** The `Approx N% complete for <file>` lines inherited
  from Java FastQC are replaced by a rich terminal display: a version banner,
  then one progress bar per input file (in command-line order) showing that
  file's own progress, read count and elapsed time. Runs of more than 10 files
  collapse to a single bar counting completed files. A live Basic Statistics
  table is drawn underneath whenever the terminal is wide enough for every
  column to be readable, with a column per file and a row per measure from the
  top of the report — cells start as `-` and fill in as the analysis proceeds,
  ending on exactly the values written to the report (both are rendered from the
  same counters). Each column heading is coloured to match its file's bar:
  accent while it runs, green once analysed, red if it failed. Built on
  [indicatif](https://crates.io/crates/indicatif). The display is used only for
  an interactive stderr: when stderr is a pipe or a log file, or `TERM` is
  `dumb`/unset, it degrades to one plain line per file at start and finish so
  pipeline logs stay readable, and `--quiet` still silences everything but
  errors. The name and version are printed before the run does anything else,
  so everything it goes on to say — including complaints about the input files
  themselves — appears beneath them in the order it happened.
- **`FASTQC_PROGRESS=auto|always|never`** overrides the display auto-detection
  in either direction. `always` draws the bars even when stderr is redirected —
  for recording a demo, or a consumer that re-renders the stream — sizing itself
  from `COLUMNS`/`LINES`; `never` always takes the plain path. Colour is a
  separate, independent switch and follows the usual environment conventions:
  `NO_COLOR` and `CLICOLOR=0` disable it, `CLICOLOR_FORCE=1` forces it on even
  for a pipe. Because the two are independent, colour off still draws the bars
  and table, and the plain fallback still colours its lines when colour is
  forced on, which is what a CI log viewer wants. `--quiet` beats both.
- **Warnings and errors scroll above the display** as ordinary terminal output,
  rather than being written into the middle of the bars and erased by the next
  frame, which is what happened to warnings raised during analysis (a bad
  quality character, too many tiles, an unreadable nanopore read). The log can
  then grow without bound, as the log of a long run must, and a blank line
  separates it from the bars when there is anything to separate. The clamping
  warning for out-of-range quality characters is also emitted once per run
  rather than once per base — deduplicated centrally, so it is genuinely once
  per run rather than once per process however many files are read at a time.
- **A closing summary**: `Complete. Analysed N files in mm:ss`, counting the
  files that were analysed successfully and widening to `hh:mm:ss` past an hour.
  It is the last line of the redrawn region, so it always appears below the bars
  and the table; `--quiet` suppresses it along with everything else.
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

### Bug fixes

- `--template` no longer claims the short flag `-t`, which is `--threads` (as in
  Java FastQC). Two arguments sharing a short name makes clap abort at startup —
  release builds skip that assertion so it only showed up in debug builds, but
  `-t` was ambiguous either way. Use the long `--template` form.

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
