# AGENTS.md

## Cursor Cloud specific instructions

This is the original Java FastQC application (master branch). It is a Swing-based bioinformatics QC tool built with Apache Ant.

### Quick reference

- **Build:** `ant clean build` (compiles to `bin/`)
- **Unit tests:** `ant clean build unit-test`
- **Integration tests:** `ant clean build integration-test`
- **UI tests:** `xvfb-run -a ant clean build ui-test` (requires Xvfb + X11 libs)
- **Run (CLI):** `java -Djava.awt.headless=true -Xmx512m -Dfastqc.output_dir=<outdir> -classpath bin:htsjdk.jar:jbzip2-0.9.jar:cisd-jhdf5.jar uk.ac.babraham.FastQC.FastQCApplication <input.fastq>`

See `test/readme.md` for details on the test framework and test data.

### Environment notes

- **Java 11** is required. Set `JAVA_HOME=/usr/lib/jvm/java-11-openjdk-amd64` and prepend `$JAVA_HOME/bin` to `PATH` before running `ant` or `java`.
- **Apache Ant** is required (installed via `apt`). The build file is `build.xml`.
- Dependencies (htsjdk, jbzip2, cisd-jhdf5) are vendored as JAR files in the repo root. Test dependencies (JUnit 5, ApprovalTests) are vendored in `lib/`. No dependency manager needed.
- The `verify_html` integration tests (`FileContentsTest`) compare base64-encoded PNG chart images against approved snapshots. These are environment-sensitive: the approved files were generated with Temurin JDK 11, so they fail on other OpenJDK builds due to font rendering differences. The `verify_data` tests (text output) are reliable. This is a known limitation, not a code bug.
- The `fastqc` script at the repo root is a Perl wrapper. For direct invocation, use `java` with the correct classpath and `-D` system properties (not `--` CLI flags). CLI options map to system properties like `-Dfastqc.output_dir=<dir>`.
- The output directory must exist before running; FastQC does not create it.
- Always pass `-Djava.awt.headless=true` when running in a headless environment (no display).
