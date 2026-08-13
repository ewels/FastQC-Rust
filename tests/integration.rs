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

/// Build the read data for the encoding tests: uniform Q40 ('I' in Phred+33)
/// with no base below Q31, which the lowest-char heuristic would misdetect
/// as Illumina 1.5 (issue #6).
fn high_quality_reads() -> Vec<(String, String, String)> {
    (0..150)
        .map(|i| {
            (
                format!("read{}", i),
                "ACGTACGTACGTACGTACGTACGTACGTACGTAC".to_string(), // 34 bp
                "I".repeat(34),                                   // Q40 throughout
            )
        })
        .collect()
}

/// Run the full pipeline via runner::run with --extract and return fastqc_data.txt.
fn run_pipeline_extracted(input_path: &Path, tmp_dir: &Path) -> String {
    let config = FastQCConfig {
        output_dir: Some(tmp_dir.to_path_buf()),
        do_unzip: Some(true),
        quiet: true,
        threads: 1,
        ..FastQCConfig::default()
    };

    fastqc_rust::runner::run(&config, &[input_path.to_path_buf()]).expect("Pipeline failed");

    let base = input_path
        .file_stem()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    std::fs::read_to_string(tmp_dir.join(format!("{}_fastqc", base)).join("fastqc_data.txt"))
        .expect("Failed to read extracted fastqc_data.txt")
}

#[test]
fn test_sam_input_uses_sanger_encoding() {
    // SAM/BAM quality is Phred+33 by construction, so the encoding must be
    // reported as Sanger even when no base is below Q31 — the lowest-char
    // heuristic would misdetect Illumina 1.5 and report every score 31 too
    // low (issue #6).
    let tmp_dir = std::env::temp_dir().join("fastqc_test_sam_encoding");
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let sam_path = tmp_dir.join("high_q.sam");
    let mut sam = String::from("@HD\tVN:1.6\n");
    for (name, seq, qual) in high_quality_reads() {
        sam.push_str(&format!(
            "{}\t4\t*\t0\t0\t*\t*\t0\t0\t{}\t{}\n",
            name, seq, qual
        ));
    }
    std::fs::write(&sam_path, sam).unwrap();

    let data = run_pipeline_extracted(&sam_path, &tmp_dir);

    assert!(
        data.contains("Encoding\tSanger / Illumina 1.9"),
        "SAM input must report Sanger encoding, got:\n{}",
        data.lines()
            .find(|l| l.starts_with("Encoding"))
            .unwrap_or("(no Encoding line)")
    );
    // Per-base mean must be 40.0, not 9.0 (= 40 - 31, the Illumina 1.5 offset error)
    assert!(
        data.contains("1\t40.0\t"),
        "Per-base quality for base 1 should have mean 40.0"
    );
    // Per-sequence quality distribution: all 150 reads average Q40
    assert!(
        data.contains("40\t150.0"),
        "Per-sequence quality should place all 150 reads at Q40"
    );

    std::fs::remove_dir_all(&tmp_dir).ok();
}

#[test]
fn test_fastq_input_keeps_detection_heuristic() {
    // For FASTQ the encoding genuinely is ambiguous, so the lowest-char
    // heuristic must be kept to match Java FastQC: uniform Q40 data
    // misdetects as Illumina 1.5 through the FASTQ path.
    let tmp_dir = std::env::temp_dir().join("fastqc_test_fastq_encoding");
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let fastq_path = tmp_dir.join("high_q.fastq");
    let mut fastq = String::new();
    for (name, seq, qual) in high_quality_reads() {
        fastq.push_str(&format!("@{}\n{}\n+\n{}\n", name, seq, qual));
    }
    std::fs::write(&fastq_path, fastq).unwrap();

    let data = run_pipeline_extracted(&fastq_path, &tmp_dir);

    assert!(
        data.contains("Encoding\tIllumina 1.5"),
        "FASTQ input must keep Java's detection heuristic"
    );

    std::fs::remove_dir_all(&tmp_dir).ok();
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
