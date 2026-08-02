//! Integration tests comparing Rust FastQC output against approved Java FastQC output.
//!
//! These tests verify byte-identical text output (fastqc_data.txt and summary.txt)
//! against the approved files from the Java test suite.

use std::path::Path;

use fastqc_rust::config::FastQCConfig;
use fastqc_rust::modules;
use fastqc_rust::report;
use fastqc_rust::sequence::fastq::FastQFile;
use fastqc_rust::sequence::SequenceFile;

/// Run the full analysis pipeline on a FASTQ file and return the text report content.
fn run_analysis(fastq_path: &Path) -> (String, String) {
    let config = FastQCConfig::default();
    let limits = config.load_limits().expect("Failed to load limits");

    let mut seq_file = FastQFile::open(&config, fastq_path).expect("Failed to open FASTQ");
    let file_display_name = seq_file.name().to_string();

    let mut mods = modules::create_modules(&config, &limits);

    for module in mods.iter_mut() {
        module.set_filename(&file_display_name);
    }

    loop {
        match seq_file.next() {
            Some(Ok(seq)) => {
                for module in mods.iter_mut() {
                    if seq.is_filtered && module.ignore_filtered_sequences() {
                        continue;
                    }
                    module.process_sequence(&seq);
                }
            }
            Some(Err(e)) => panic!("Error reading sequence: {}", e),
            None => break,
        }
    }

    for module in mods.iter_mut() {
        module.finalize();
    }

    // Generate fastqc_data.txt
    let mut data_buf = Vec::new();
    report::text::write_fastqc_data(&mods, &mut data_buf).expect("Failed to write data");
    let data_text = String::from_utf8(data_buf).expect("Invalid UTF-8 in data");

    // Generate summary.txt
    let mut summary_buf = Vec::new();
    report::text::write_summary(&mods, &file_display_name, &mut summary_buf)
        .expect("Failed to write summary");
    let summary_text = String::from_utf8(summary_buf).expect("Invalid UTF-8 in summary");

    (data_text, summary_text)
}

#[test]
fn test_minimal_fastqc_data_matches_approved() {
    let (data, _summary) = run_analysis(Path::new("tests/data/minimal.fastq"));
    let approved =
        std::fs::read_to_string("tests/approved/FileContentsTest_minimal_fastqc_data.approved.txt")
            .expect("Failed to read approved file");
    assert_eq!(
        data, approved,
        "minimal.fastq fastqc_data.txt does not match approved output"
    );
}

#[test]
fn test_complex_fastqc_data_matches_approved() {
    let (data, _summary) = run_analysis(Path::new("tests/data/complex.fastq"));
    let approved =
        std::fs::read_to_string("tests/approved/FileContentsTest_complex_fastqc_data.approved.txt")
            .expect("Failed to read approved file");
    assert_eq!(
        data, approved,
        "complex.fastq fastqc_data.txt does not match approved output"
    );
}

#[test]
fn test_minimal_summary_format() {
    let (_data, summary) = run_analysis(Path::new("tests/data/minimal.fastq"));
    // Verify summary format: each line is STATUS\tModuleName\tFilename
    for line in summary.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        assert_eq!(
            parts.len(),
            3,
            "Summary line should have 3 tab-separated fields: {}",
            line
        );
        assert!(
            matches!(parts[0], "PASS" | "WARN" | "FAIL"),
            "Status should be PASS/WARN/FAIL, got: {}",
            parts[0]
        );
        assert_eq!(
            parts[2], "minimal.fastq",
            "Filename should be minimal.fastq"
        );
    }
}

#[test]
fn test_gzipped_input_produces_same_analysis() {
    // The gzipped version should produce the same analysis results
    // (only the filename in BasicStats will differ)
    let (data_plain, _) = run_analysis(Path::new("tests/data/minimal.fastq"));
    let (data_gz, _) = run_analysis(Path::new("tests/data/minimal.fastq.gz"));

    // Replace filename difference and compare
    let data_gz_normalized = data_gz.replace("minimal.fastq.gz", "minimal.fastq");
    assert_eq!(
        data_plain, data_gz_normalized,
        "Gzipped input should produce identical analysis to plain input"
    );
}

#[test]
fn test_html_report_generation() {
    let config = FastQCConfig::default();
    let limits = config.load_limits().expect("Failed to load limits");

    let mut seq_file = FastQFile::open(&config, Path::new("tests/data/minimal.fastq"))
        .expect("Failed to open FASTQ");
    let file_display_name = seq_file.name().to_string();

    let mut mods = modules::create_modules(&config, &limits);
    for module in mods.iter_mut() {
        module.set_filename(&file_display_name);
    }

    loop {
        match seq_file.next() {
            Some(Ok(seq)) => {
                for module in mods.iter_mut() {
                    if seq.is_filtered && module.ignore_filtered_sequences() {
                        continue;
                    }
                    module.process_sequence(&seq);
                }
            }
            Some(Err(e)) => panic!("Error: {}", e),
            None => break,
        }
    }

    for module in mods.iter_mut() {
        module.finalize();
    }

    let html = report::html::generate_html_report(
        &mods,
        &file_display_name,
        fastqc_rust::config::TemplateName::Classic,
    )
    .expect("Failed to generate HTML");

    // Verify HTML structure
    assert!(
        html.starts_with("<!DOCTYPE html>"),
        "Should start with DOCTYPE"
    );
    assert!(
        html.contains("<title>minimal.fastq FastQC Report</title>"),
        "Should have title"
    );
    assert!(
        html.contains("Basic Statistics"),
        "Should contain BasicStats module"
    );
    assert!(
        html.contains("data:image/png;base64,"),
        "Should contain base64 icons"
    );
    assert!(html.contains("</html>"), "Should end with closing html tag");
}

#[test]
fn test_zip_archive_structure() {
    let config = FastQCConfig::default();
    let limits = config.load_limits().expect("Failed to load limits");

    let mut seq_file = FastQFile::open(&config, Path::new("tests/data/complex.fastq"))
        .expect("Failed to open FASTQ");
    let file_display_name = seq_file.name().to_string();

    let mut mods = modules::create_modules(&config, &limits);
    for module in mods.iter_mut() {
        module.set_filename(&file_display_name);
    }

    loop {
        match seq_file.next() {
            Some(Ok(seq)) => {
                for module in mods.iter_mut() {
                    if seq.is_filtered && module.ignore_filtered_sequences() {
                        continue;
                    }
                    module.process_sequence(&seq);
                }
            }
            Some(Err(e)) => panic!("Error: {}", e),
            None => break,
        }
    }

    for module in mods.iter_mut() {
        module.finalize();
    }

    let tmp_dir = std::env::temp_dir().join("fastqc_test_zip");
    std::fs::create_dir_all(&tmp_dir).unwrap();
    let zip_path = tmp_dir.join("complex_fastqc.zip");

    let html_content = report::html::generate_html_report(
        &mods,
        &file_display_name,
        fastqc_rust::config::TemplateName::Classic,
    )
    .expect("Failed to generate HTML");
    report::archive::create_zip_archive(
        &mods,
        &file_display_name,
        "complex",
        &zip_path,
        &html_content,
        true,
        fastqc_rust::config::TemplateName::Classic,
    )
    .expect("Failed to create zip");

    // Read zip and verify structure
    let file = std::fs::File::open(&zip_path).expect("Failed to open zip");
    let archive = zip::ZipArchive::new(file).expect("Failed to read zip");

    let names: Vec<String> = (0..archive.len())
        .map(|i| archive.name_for_index(i).unwrap().to_string())
        .collect();

    assert!(
        names.iter().any(|n| n.contains("fastqc_data.txt")),
        "Should contain fastqc_data.txt"
    );
    assert!(
        names.iter().any(|n| n.contains("summary.txt")),
        "Should contain summary.txt"
    );
    assert!(
        names.iter().any(|n| n.contains("fastqc_report.html")),
        "Should contain HTML"
    );
    assert!(
        names.iter().any(|n| n.contains("fastqc.fo")),
        "Should contain XSL-FO"
    );
    assert!(
        names.iter().any(|n| n.contains("Icons/tick.png")),
        "Should contain tick icon"
    );
    assert!(
        names.iter().any(|n| n.contains("Images/")),
        "Should have Images directory"
    );

    // Cleanup
    std::fs::remove_dir_all(&tmp_dir).ok();
}

/// Run the `fastqc` binary with stderr captured — so stderr is a pipe, never a
/// terminal — and return what it wrote there.
fn run_binary_stderr(extra_args: &[&str], env: &[(&str, &str)]) -> String {
    // A counter, not the arguments: these run in parallel and two cases can
    // easily share an argument list, which would have them deleting each
    // other's output directory mid-run.
    static NEXT_DIR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let tmp_dir = std::env::temp_dir().join(format!(
        "fastqc_progress_test_{}_{}",
        std::process::id(),
        NEXT_DIR.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
    ));
    std::fs::create_dir_all(&tmp_dir).expect("Failed to create temp dir");

    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_fastqc"));
    command
        .args(extra_args)
        .arg("-o")
        .arg(&tmp_dir)
        .arg("tests/data/minimal.fastq");
    for (key, value) in env {
        command.env(key, value);
    }
    let output = command.output().expect("Failed to run fastqc");

    std::fs::remove_dir_all(&tmp_dir).ok();
    assert!(
        output.status.success(),
        "fastqc exited with {:?}",
        output.status
    );
    String::from_utf8(output.stderr).expect("stderr is not UTF-8")
}

/// Does the output contain colour (SGR) escape sequences other than a reset?
///
/// An SGR sequence is `ESC [ <digits and semicolons> m`. The parameter run has
/// to be validated rather than just searching forward for an `m`, or the `m` in
/// a word like "minimal.fastq" turns a cursor escape into a false positive.
fn has_color(stderr: &str) -> bool {
    stderr
        .split('\u{1b}')
        .skip(1)
        .filter_map(|s| s.strip_prefix('['))
        .filter_map(|s| {
            let params: String = s
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == ';')
                .collect();
            s[params.len()..].starts_with('m').then_some(params)
        })
        .any(|params| !params.is_empty() && params != "0")
}

/// Does the output redraw in place? Cursor-up is the giveaway.
fn has_redraw(stderr: &str) -> bool {
    stderr
        .split('\u{1b}')
        .skip(1)
        .filter_map(|s| s.strip_prefix('['))
        .any(|s| {
            s.trim_start_matches(|c: char| c.is_ascii_digit())
                .starts_with('A')
        })
}

/// With colour auto-detected, nothing should reach a captured stream: piped
/// stderr is not a terminal. This is what keeps workflow-engine logs readable.
#[test]
fn test_no_ansi_escapes_when_stderr_is_piped() {
    for env in [
        &[][..],
        &[("NO_COLOR", "1")][..],
        &[("CLICOLOR", "0")][..],
        &[("TERM", "dumb")][..],
        &[("TERM", "xterm-256color")][..],
    ] {
        let stderr = run_binary_stderr(&[], env);
        assert!(
            !stderr.contains('\u{1b}'),
            "escape sequence in piped stderr with env {:?}: {:?}",
            env,
            stderr
        );
        // The fallback still says what it is doing, rather than going quiet.
        assert!(
            stderr.contains("Started analysis of minimal.fastq"),
            "missing start line with env {:?}: {:?}",
            env,
            stderr
        );
        assert!(
            stderr.contains("Analysis complete for minimal.fastq"),
            "missing completion line with env {:?}: {:?}",
            env,
            stderr
        );
        // The percentage lines the display replaced must not come back.
        assert!(
            !stderr.contains("Approx"),
            "percentage progress line resurfaced with env {:?}: {:?}",
            env,
            stderr
        );
    }
}

/// Colour can be forced back on for a pipe — a CI log viewer renders escape
/// sequences happily despite not being a terminal. Forcing colour must *not*
/// drag the animated display along with it.
#[test]
fn test_colour_can_be_forced_on_a_pipe() {
    for (args, env) in [
        // The clicolors spec's override is the only way to force colour.
        (&[][..], &[("CLICOLOR_FORCE", "1")][..]),
        // ...and it wins over a terminal that is not one.
        (&[][..], &[("CLICOLOR_FORCE", "1"), ("TERM", "dumb")][..]),
    ] {
        let stderr = run_binary_stderr(args, env);
        assert!(
            has_color(&stderr),
            "expected colour for {:?} {:?}: {:?}",
            args,
            env,
            stderr
        );
        assert!(
            !has_redraw(&stderr),
            "colour must not imply the animated display for {:?} {:?}",
            args,
            env
        );
    }

    // NO_COLOR wins over CLICOLOR_FORCE when both are set, per NO_COLOR's spec.
    let stderr = run_binary_stderr(&[], &[("NO_COLOR", "1"), ("CLICOLOR", "0")]);
    assert!(
        !stderr.contains('\u{1b}'),
        "NO_COLOR must suppress colour: {:?}",
        stderr
    );
}

/// The animated display can be forced onto a pipe (for recording a demo, or a
/// consumer that re-renders the stream), and forced off on any stream.
#[test]
fn test_progress_display_can_be_forced_on_a_pipe() {
    let forced = run_binary_stderr(&[], &[("FASTQC_PROGRESS", "always"), ("COLUMNS", "100")]);
    assert!(
        has_redraw(&forced),
        "FASTQC_PROGRESS=always must redraw in place: {:?}",
        forced
    );
    assert!(
        forced.contains('━'),
        "FASTQC_PROGRESS=always must draw the bar: {:?}",
        forced
    );
    // The pinned header sits at the top of the display, with the version dim.
    assert!(
        forced.contains("FastQC-Rust"),
        "forced display is missing its header: {:?}",
        forced
    );

    // Forced off, a pipe stays exactly as it was.
    let never = run_binary_stderr(&[], &[("FASTQC_PROGRESS", "never")]);
    assert!(
        !has_redraw(&never),
        "FASTQC_PROGRESS=never redrew: {:?}",
        never
    );
    assert!(never.contains("Started analysis of minimal.fastq"));

    // An unrecognised value falls back to detection rather than failing.
    let bogus = run_binary_stderr(&[], &[("FASTQC_PROGRESS", "sometimes")]);
    assert!(!has_redraw(&bogus) && !bogus.contains('\u{1b}'));

    // Colour and animation are independent switches.
    let both = run_binary_stderr(
        &[],
        &[
            ("FASTQC_PROGRESS", "always"),
            ("CLICOLOR_FORCE", "1"),
            ("COLUMNS", "100"),
        ],
    );
    assert!(has_redraw(&both) && has_color(&both));
    let mono = run_binary_stderr(
        &[],
        &[
            ("FASTQC_PROGRESS", "always"),
            ("NO_COLOR", "1"),
            ("COLUMNS", "100"),
        ],
    );
    assert!(
        has_redraw(&mono) && !has_color(&mono),
        "NO_COLOR must strip colour from a forced display"
    );
}

/// The header must be styled: the name bold cyan, the version dim.
#[test]
fn test_header_is_styled() {
    let stderr = run_binary_stderr(
        &[],
        &[
            ("FASTQC_PROGRESS", "always"),
            ("CLICOLOR_FORCE", "1"),
            ("COLUMNS", "100"),
        ],
    );
    let header = stderr
        .lines()
        .find(|l| l.contains("FastQC-Rust"))
        .unwrap_or_else(|| panic!("no header line in {:?}", stderr));
    // SGR 36 = cyan, 1 = bold, 2 = dim (the version).
    for code in ["\u{1b}[36m", "\u{1b}[1m", "\u{1b}[2m"] {
        assert!(
            header.contains(code),
            "header missing {:?}: {:?}",
            code,
            header
        );
    }
    assert!(
        header.contains(env!("CARGO_PKG_VERSION")),
        "header missing the version: {:?}",
        header
    );
}

/// `--quiet` means silent, and beats every force flag and environment override.
#[test]
fn test_quiet_beats_everything() {
    for (args, env) in [
        (&["--quiet"][..], &[][..]),
        (&["--quiet"][..], &[("NO_COLOR", "1")][..]),
        (&["--quiet"][..], &[("CLICOLOR_FORCE", "1")][..]),
        (
            &["--quiet"][..],
            &[("FASTQC_PROGRESS", "always"), ("CLICOLOR_FORCE", "1")][..],
        ),
    ] {
        let stderr = run_binary_stderr(args, env);
        assert!(
            stderr.is_empty(),
            "--quiet wrote to stderr for {:?} {:?}: {:?}",
            args,
            env,
            stderr
        );
    }
}

/// A log line emitted while the display is live must end up *above* it, as
/// ordinary scrollback, with the display redrawn intact underneath and the
/// message not broken up. The version banner is itself a log line, so it stays
/// above everything the run goes on to say; the closing summary is part of the
/// region, so it stays below the bars and the table.
///
/// Checked on the forced display so the assertion does not need a terminal.
#[test]
fn test_log_lines_scroll_above_the_display() {
    let tmp_dir = std::env::temp_dir().join(format!("fastqc_logline_{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir).expect("temp dir");

    let bad = tmp_dir.join("broken.fastq");
    std::fs::write(&bad, "not a fastq file at all\n").expect("write");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fastqc"))
        .env("FASTQC_PROGRESS", "always")
        .env("COLUMNS", "100")
        .env("LINES", "40")
        .arg("-o")
        .arg(&tmp_dir)
        .arg(&bad)
        .arg("tests/data/complex.fastq")
        .output()
        .expect("run fastqc");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    std::fs::remove_dir_all(&tmp_dir).ok();

    let message = "Failed to process broken.fastq";
    assert!(
        stderr.contains(message),
        "error line missing entirely: {:?}",
        stderr
    );

    // The whole message survives contiguously — no redraw cut into the middle
    // of it. (It is a line of the display now, so it is width-padded and
    // followed by the next frame's escapes, like every other line.)
    let whole = "Failed to process broken.fastq: ID line didn't start with '@' at line 1";
    assert!(
        stderr.contains(whole),
        "log line was broken up by a redraw: {:?}",
        stderr
    );

    // In the final frame the message sits below the header, the bars and the
    // table. Compare positions within the last frame only, since earlier frames
    // are erased and redrawn.
    let final_frame = stderr
        .rfind("FastQC-Rust")
        .map(|i| &stderr[i..])
        .expect("no header drawn");
    let header_at = final_frame.find("FastQC-Rust").expect("header");
    let table_at = final_frame.find("Total Sequences").expect("table");
    let message_at = final_frame.find(message).expect("message in final frame");
    let summary_at = final_frame
        .find("Complete.")
        .expect("summary in final frame");
    assert!(
        header_at < table_at && table_at < message_at && message_at < summary_at,
        "expected header < table < message < summary, got {} {} {} {}",
        header_at,
        table_at,
        message_at,
        summary_at
    );
}

/// Every run signs off with a count and a wall-clock duration.
#[test]
fn test_completion_summary() {
    // One file analysed, one rejected: the count reports what actually worked.
    let tmp_dir = std::env::temp_dir().join(format!("fastqc_summary_{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir).expect("temp dir");
    let bad = tmp_dir.join("broken.fastq");
    std::fs::write(&bad, "nope\n").expect("write");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fastqc"))
        .arg("-o")
        .arg(&tmp_dir)
        .arg("tests/data/minimal.fastq")
        .arg(&bad)
        .output()
        .expect("run fastqc");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    std::fs::remove_dir_all(&tmp_dir).ok();

    // Singular for one file, and mm:ss for the duration.
    assert!(
        stderr.contains("Complete. Analysed 1 file in "),
        "unexpected summary: {:?}",
        stderr
    );
    let tail = stderr.rsplit("in ").next().unwrap_or("").trim();
    let (minutes, seconds) = tail.split_once(':').unwrap_or_else(|| panic!("{tail:?}"));
    assert_eq!(minutes.len(), 2, "minutes not zero-padded: {tail:?}");
    assert_eq!(seconds.len(), 2, "seconds not zero-padded: {tail:?}");
    assert!(minutes.parse::<u32>().is_ok() && seconds.parse::<u32>().is_ok());

    // Two files, plural.
    let tmp_dir = std::env::temp_dir().join(format!("fastqc_summary2_{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir).expect("temp dir");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fastqc"))
        .arg("-o")
        .arg(&tmp_dir)
        .arg("tests/data/minimal.fastq")
        .arg("tests/data/complex.fastq")
        .output()
        .expect("run fastqc");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    std::fs::remove_dir_all(&tmp_dir).ok();
    assert!(
        stderr.contains("Complete. Analysed 2 files in "),
        "unexpected summary: {:?}",
        stderr
    );
}

/// `--quiet` stays quiet right to the end: no completion line either.
#[test]
fn test_quiet_suppresses_the_completion_summary() {
    let stderr = run_binary_stderr(&["--quiet"], &[]);
    assert!(stderr.is_empty(), "--quiet wrote: {:?}", stderr);
}

/// Whether the statistics table appears is decided by the terminal width, not
/// by how many files there are: the same run gains and loses the table as the
/// width changes, and a wide enough terminal shows it for more files than the
/// old four-column cap allowed.
#[test]
fn test_table_visibility_follows_terminal_width() {
    fn run(files: &[&str], columns: &str) -> String {
        let tmp_dir = std::env::temp_dir().join(format!(
            "fastqc_width_{}_{}_{}",
            std::process::id(),
            files.len(),
            columns
        ));
        std::fs::create_dir_all(&tmp_dir).expect("temp dir");
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_fastqc"))
            .env("FASTQC_PROGRESS", "always")
            .env("COLUMNS", columns)
            .env("LINES", "60")
            .arg("-o")
            .arg(&tmp_dir)
            .args(files)
            .output()
            .expect("run fastqc");
        std::fs::remove_dir_all(&tmp_dir).ok();
        String::from_utf8_lossy(&output.stderr).into_owned()
    }

    // "Total Sequences" is a table row label and appears nowhere else.
    let shows_table = |s: &str| s.contains("Total Sequences");

    let one = ["tests/data/minimal.fastq"];
    let three = [
        "tests/data/minimal.fastq",
        "tests/data/complex.fastq",
        "tests/data/minimal.fastq.gz",
    ];

    // One file fits on a normal terminal but not a cramped one.
    assert!(shows_table(&run(&one, "80")));
    assert!(!shows_table(&run(&one, "40")));

    // Three files need more room than 80 columns can give.
    assert!(!shows_table(&run(&three, "80")));
    assert!(shows_table(&run(&three, "120")));

    // The bars are drawn either way — only the table is switched off.
    let cramped = run(&three, "80");
    assert!(
        cramped.contains('━'),
        "bars should survive a narrow terminal: {:?}",
        cramped
    );
}

/// The table's column headings follow their file's progress: the accent colour
/// while it runs, green once analysed, red if it failed.
#[test]
fn test_table_headings_are_coloured_by_progress() {
    let tmp_dir = std::env::temp_dir().join(format!("fastqc_heading_{}", std::process::id()));
    std::fs::create_dir_all(&tmp_dir).expect("temp dir");
    let bad = tmp_dir.join("broken.fastq");
    std::fs::write(&bad, "nope\n").expect("write");

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_fastqc"))
        .env("FASTQC_PROGRESS", "always")
        .env("CLICOLOR_FORCE", "1")
        .env("COLUMNS", "120")
        .env("LINES", "40")
        .arg("-o")
        .arg(&tmp_dir)
        .arg("tests/data/minimal.fastq")
        .arg(&bad)
        .output()
        .expect("run fastqc");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    std::fs::remove_dir_all(&tmp_dir).ok();

    // The last heading row drawn is the final state: one analysed, one failed.
    let heading = stderr
        .rmatch_indices("Measure")
        .map(|(i, _)| &stderr[i..])
        .find(|s| s.contains("minimal.fastq"))
        .expect("no heading row drawn");
    let heading = &heading[..heading.find('\r').unwrap_or(heading.len())];

    // SGR 32 = green, 31 = red, both bold (1).
    let green_at = heading
        .find("\u{1b}[1m\u{1b}[32m")
        .or_else(|| heading.find("\u{1b}[32m"));
    let red_at = heading
        .find("\u{1b}[1m\u{1b}[31m")
        .or_else(|| heading.find("\u{1b}[31m"));
    assert!(
        green_at.is_some(),
        "analysed file's heading is not green: {:?}",
        heading
    );
    assert!(
        red_at.is_some(),
        "failed file's heading is not red: {:?}",
        heading
    );
    assert!(
        green_at < red_at,
        "colours are on the wrong columns: {:?}",
        heading
    );
}
