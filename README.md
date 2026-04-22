# FastQC (Rust)

An **unofficial** Rust rewrite of [FastQC](https://github.com/s-andrews/FastQC), the sequencing QC tool by Simon Andrews at the Babraham Institute.

> [!WARNING]
>
> **You should probably use the [official Java version](https://github.com/s-andrews/FastQC), not this one.**
>
> This rewrite is primarily a development vehicle for [porting improvements back to the canonical tool](https://ewels.github.io/FastQC-Rust/about/strategy/), as well as being a Rust crate for folks building in that ecosystem - not a FastQC replacement. For regular use, install the official version from [Babraham](https://www.bioinformatics.babraham.ac.uk/projects/fastqc/) or [GitHub](https://github.com/s-andrews/FastQC).

![FastQC Screenshot](docs/public/images/fastqc.png)

## Why does this exist?

Two reasons, both secondary to the original tool:

- **Upstream contributions** — a sandbox for prototyping improvements (performance, bug fixes, UI) that get [ported back to Java FastQC as PRs](https://ewels.github.io/FastQC-Rust/about/strategy/). The goal is to make the canonical tool better, not replace it.
- **Rust crate** — published as [`fastqc-rust`](https://crates.io/crates/fastqc-rust) for developers building bioinformatics tooling in the Rust ecosystem. `fastqc_data.txt` and `summary.txt` are byte-identical to the Java version — see the [equivalence report](https://ewels.github.io/FastQC-Rust/about/equivalence/).

Currently tracking upstream Java FastQC version — see [`UPSTREAM.toml`](UPSTREAM.toml) for details.

## Installation

### From source

```bash
cargo install fastqc-rust
```

### From a release binary

Download prebuilt binaries from the [Releases](https://github.com/ewels/FastQC-Rust/releases) page.

### Building from source

```bash
git clone https://github.com/ewels/FastQC-Rust.git
cd FastQC-Rust
cargo build --release
# Binary at ./target/release/fastqc
```

## Usage

```bash
# Analyze a FASTQ file
fastqc sample.fastq.gz

# Specify output directory
fastqc -o results/ sample.fastq.gz

# Analyze multiple files in parallel
fastqc -t 4 *.fastq.gz

# BAM input
fastqc --format bam aligned.bam

# See all options
fastqc --help
```

### Key options

| Flag | Description |
|------|-------------|
| `-o, --outdir DIR` | Output directory (must exist) |
| `-f, --format FORMAT` | Force format: `bam`, `sam`, `bam_mapped`, `sam_mapped`, `fastq` |
| `-t, --threads N` | Number of parallel file processing threads |
| `--casava` | CASAVA 1.8+ mode (exclude filtered reads) |
| `--nano` | Nanopore Fast5 mode |
| `--nogroup` | Disable base grouping for reads > 50bp |
| `--expgroup` | Use exponential base groups |
| `-k, --kmers N` | Kmer size 2-10 (default: 7) |
| `--min_length N` | Minimum sequence length to report |
| `--extract` | Unzip output after creation |

## Equivalence testing

This project maintains strict equivalence with the upstream Java FastQC. CI runs automated comparison of text output and chart images against stored Java reference data.

```bash
# Run equivalence tests locally (requires uv)
cargo build --release
uv run tests/equivalence/compare.py --binary ./target/release/fastqc
```

This generates an interactive HTML report with text diffs and side-by-side image comparison. See `tests/equivalence/` for details.

## Upstream tracking

`UPSTREAM.toml` pins the Java FastQC version this rewrite tracks. A nightly CI job checks for new upstream releases and automatically creates a GitHub issue when one is found.

## License

GPL-3.0 — see [LICENSE](LICENSE).

## Acknowledgments

FastQC was originally written by Simon Andrews at the [Babraham Institute](https://www.bioinformatics.babraham.ac.uk/). This Rust rewrite aims to be a faithful, high-performance reimplementation.
