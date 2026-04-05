# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

FastQC is a Java quality control application for high-throughput sequencing data (FastQ/BAM files). It runs a set of analysis modules on sequence files and produces HTML reports. Supports both interactive GUI (Swing) and CLI/pipeline modes.

## Build Commands

Uses Apache Ant with Java 11. Dependencies are bundled JARs (root directory + `lib/`).

```bash
ant build                          # Compile the project
ant clean build                    # Clean and rebuild
ant clean build unit-test          # Run unit tests
ant clean build integration-test   # Run integration tests
ant FastQCApplication              # Run the GUI application
```

UI tests require an X11 display: `xvfb-run -a ant clean build ui-test`

CI runs: `ant clean build unit-test integration-test`

## Architecture

**Entry point:** `uk.ac.babraham.FastQC.FastQCApplication` - parses CLI args via `FastQCConfig`, launches either GUI mode or `OfflineRunner` for CLI mode.

**Core pipeline flow:**
1. `Sequence.SequenceFactory` creates a `SequenceFile` reader (FastQ, BAM, etc.)
2. `Analysis.AnalysisRunner` (GUI) or `Analysis.OfflineRunner` (CLI) iterates sequences
3. Each `Sequence` is passed to all active `QCModule` implementations via `processSequence()`
4. Modules accumulate statistics, then `Report.HTMLReportArchive` generates the output

**Key abstractions:**
- `Modules.QCModule` - interface all analysis modules implement (`processSequence()`, `raisesError()`, `raisesWarning()`, `makeReport()`)
- `Modules.ModuleFactory` - instantiates the set of active modules
- `Modules.ModuleConfig` - reads pass/warn/fail thresholds from `Configuration/limits.txt`
- `Sequence.SequenceFile` - abstraction over input formats
- `Sequence.QualityEncoding.PhredEncoding` - auto-detects Phred offset (33 vs 64)

**QC Modules** (in `Modules/`): BasicStats, PerBaseQualityScores, PerTileQualityScores, PerSequenceQualityScores, PerBaseSequenceContent, PerSequenceGCContent, NContent, SequenceLengthDistribution, DuplicationLevel, OverRepresentedSeqs, AdapterContent, KmerContent.

**Configuration files** in `Configuration/`: `adapter_list.txt`, `contaminant_list.txt`, `limits.txt` (module thresholds).

## Source Layout

All Java source lives directly in the repo (no `src/` directory) under the package path:
- `uk/ac/babraham/FastQC/` - main application code
- `org/apache/commons/math3/` - vendored math utilities
- `net/sourceforge/iharder/base64/` - vendored Base64 utility

Compiled output goes to `bin/`. No Maven/Gradle - pure Ant.

## Testing

JUnit 5 (Jupiter). Tests live under `test/` with separate `unit/`, `integration/`, and `ui/` directories. Compiled test output goes to `test/bin/`, reports to `test/reports/`.

Integration tests use **approval testing** (ApprovalTests library) - expected outputs are stored as `.approved.*` files alongside tests. When a test fails, a `.received.*` file is generated for comparison. To update approved output, replace the `.approved` file with the `.received` file.

Test data files are in `test/data/` with test case definitions in `test/data/TestCases.java`.

## Launcher Scripts

- `fastqc` - Perl wrapper script for Linux/macOS (sets up classpath, JVM args)
- `run_fastqc.bat` - Windows batch launcher

## Rust Rewrite

`fastqc-rs/` contains a pure Rust CLI rewrite that produces byte-identical text output to the Java version. See `fastqc-rs/CLAUDE.md` for details. Build with `cd fastqc-rs && cargo build --release`.
