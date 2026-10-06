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
  A single file scales until the heaviest single module dominates (~2.4x on a
  7.9 GB WES file, flat from `-t 6`); the order-dependent modules (overrepresented sequences,
  per-sequence GC) can't be split without changing output, so beyond that extra
  cores are best spent on more files at once, which scales linearly.
- **`-t/--threads` is a ceiling on the whole run**, each file's decoder
  included. Give it and the run stays inside it — `-t 1` really does mean one
  analysis thread and one decoder, which is what a workflow engine passing
  `task.cpus` needs. Leave it out and the budget defaults to **the available
  CPUs, up to 6** — a plain `fastqc sample.fastq.gz` gets the parallel pipeline
  without being asked, but a big shared machine is not treated as idle just
  because it is big. (Java FastQC defaults to 1; output is byte-identical
  whatever the budget.) The thread budget honours cgroup quotas and CPU
  affinity, so a container or a scheduler-pinned job sees its own allowance,
  not the host's cores. Six is where a single file's returns flatten; at most 6
  analysis workers run per file, and any budget beyond that is left idle.
- **Bounded pipeline memory on long reads.** The analysis pipeline capped its
  in-flight batches by record count alone, which is a few MB of Illumina reads
  but gigabytes of nanopore or PacBio ones. Batches are now capped by bytes as
  well: peak RSS on a 10 kb-read FASTQ at `-t 8` drops from 552 MB to 66 MB, and
  short-read runs batch exactly as before.
- **rapidgzip is now the default** (and only) gzip reader, backed by
  [`rapidgzip-core`](https://crates.io/crates/rapidgzip-core). `.fastq.gz` is
  decoded on a background thread, overlapped with the analysis, with
  byte-identical output. One decoder keeps up with the full parallel pipeline;
  the new `--decompress-threads N` option (default `1`, not counted in
  `--threads`) decodes in parallel chunks, but on typical data that costs CPU
  and memory without speeding the run up.
- **Removed the flate2/system-zlib gzip path**, the `rapidgzip`/`native-zlib`
  Cargo features, and the `FASTQC_GZIP_BACKEND` switch. The binary is now pure
  Rust (zlib-rs) with no C toolchain or system-library dependency, so builds are
  fully static by default. BAM/BGZF and Fast5 decompression (via
  `noodles`/`hdf5-pure`) use flate2's pure-Rust zlib-rs backend instead of
  system zlib: ~10% slower on a BAM at `-t 1` on macOS, where the default
  `miniz_oxide` backend was ~30% slower. The `zip` dependency is likewise reduced to the `deflate`
  feature — the only compression method FastQC ever writes or reads — which
  drops `xz2`/`lzma-sys` and with it the last dynamically linked C library.

### Bug fixes

- **The progress bar for a `.fastq.gz` no longer runs ahead of the file.** It
  was driven by the compressed bytes the decoder had read, and rapidgzip's
  workers read far ahead of the parser: on four cores a 99 MB `.gz` opened at
  22% and hit 100% halfway through the run, and anything under about 20 MB was
  pinned at 100% from the first update. Progress now comes from the decompressed
  bytes handed to the analysis, against a total taken from the gzip trailer
  (exact for the single-member files `gzip` and `pigz` produce, including past
  4 GiB) or estimated from the achieved ratio for multi-member and BGZF input.
- **A file's elapsed time is its own.** Every bar's clock started when the
  display was built rather than when its file did, so anything queued behind
  another file counted the wait: a 2,000-read file reported 13.9s next to the
  400,000-read file it was waiting for, which reported 9.8s. Queued files now
  sit at zero, with an idle spinner, until they actually start.
- **A quality byte of 128 or more no longer panics the run.** `Per sequence
  quality scores` indexed its 128-slot tally with the mean quality directly, so
  a mis-encoded or non-ASCII quality line aborted the analysis (and, under the
  parallel pipeline, took a worker thread with it). It clamps and warns once,
  like the per-position tally next to it. Pre-existing, not new in this release.
- The live statistics table follows a terminal resized mid-run, rather than
  staying laid out for the width the run started at.
- Read counts just short of a unit read as `1.0M` rather than `1000.0k`.

- Use Sanger / Illumina 1.9 encoding for BAM/SAM input instead of inferring it from the lowest quality character ([#6](https://github.com/ewels/FastQC-Rust/issues/6), [#10](https://github.com/ewels/FastQC-Rust/pull/10)). BAM/SAM quality is Phred+33 by specification. Java FastQC v0.13.0 now defaults to Phred+33 for all input, so the only divergence left is that `--phred64` has no effect on BAM/SAM input. The matching Java PR ([s-andrews/FastQC#210](https://github.com/s-andrews/FastQC/pull/210)) was closed as superseded by v0.13.0.
- Per-base charts (quality, sequence content, N content, adapter, Kmer, length distribution, per-tile) are now `max(800, groups × 15)` px wide, as in Java. Before, they were always 800px, which squashed long-read and `--nogroup` plots.
- Adapter Content shows "Can't analyse adapters as read length is too short", as in Java, when no read is longer than the longest adapter. Overrepresented sequences shows "No overrepresented sequences" when there are none.
- `--template` no longer uses the `-t` short flag, which clashed with `--threads`.
- Fixed the nightly upstream check, which failed because the `upstream-update` label did not exist.

### Other

- New equivalence test cases for mixed read lengths, `--min_length`/`--max_length`, empty input and Phred+64 (26 cases in total).
- `generate_reference.sh` works with an unpacked upstream release zip. The new `update_svg_patches.py` regenerates the SVG patches.
- New docs page: [upstream contributions](https://ewels.github.io/FastQC-Rust/about/upstream/).
### Breaking changes for library users

- `FastQCConfig::threads` is now `Option<usize>`; `None` (the default) means
  "not specified", which is what lets an explicit budget bound decompression
  while an absent one does not. Pass `Some(n)` where you passed `n`.
- `BasicStats::format_length` is now the free function
  `modules::basic_stats::format_length`. The behaviour is unchanged.

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
