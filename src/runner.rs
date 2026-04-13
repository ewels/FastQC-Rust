use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use rayon::prelude::*;

use crate::config::FastQCConfig;
use crate::modules;
use crate::report;
use crate::sequence::casava;
use crate::sequence::open_sequence_file;
use crate::sequence::{SequenceFile, SequenceFileGroup};

/// A unit of work: one logical sample to process through all QC modules.
/// Contains a display name and the list of file paths that constitute it.
struct FileGroup {
    /// The display name for reports (CASAVA basename or original filename).
    name: String,
    /// The file paths in this group (usually 1, but >1 for CASAVA groups).
    files: Vec<PathBuf>,
}

/// Run FastQC analysis on the given input files.
///
/// The Java OfflineRunner iterates over files, creates a
/// SequenceFile reader for each, instantiates all QC modules, feeds every
/// Sequence to each module, then writes the report. With --threads, files
/// are processed in parallel via AnalysisQueue.
pub fn run(config: &FastQCConfig, files: &[PathBuf]) -> Result<(), i32> {
    let limits = config.load_limits().map_err(|e| {
        eprintln!("Failed to load limits: {}", e);
        1
    })?;

    // Validate all files exist before starting processing.
    // For stdin, skip the existence check (Java: `filenames[0].startsWith("stdin")`).
    // For --nano mode, expand directories to find .fast5 files within them.
    let mut valid_files = Vec::new();
    let mut something_failed = false;
    for file_path in files {
        let file_name = file_path.to_string_lossy();
        if !file_name.starts_with("stdin") && !file_path.exists() {
            eprintln!("{} doesn't exist", file_name);
            something_failed = true;
        } else if config.nano && file_path.is_dir() {
            // In --nano mode, directories are recursively searched for .fast5 files.
            // Matches OfflineRunner.java's directory expansion logic.
            match find_fast5_files(file_path) {
                Ok(fast5_files) => {
                    if fast5_files.is_empty() {
                        eprintln!("No .fast5 files found in {}", file_path.display());
                        something_failed = true;
                    } else {
                        valid_files.extend(fast5_files);
                    }
                }
                Err(e) => {
                    eprintln!("Error scanning directory {}: {}", file_path.display(), e);
                    something_failed = true;
                }
            }
        } else {
            valid_files.push(file_path.clone());
        }
    }

    // Group files based on mode (casava, nano, or individual).
    // Java's OfflineRunner.java lines 103-117 handles this branching.
    let file_groups = build_file_groups(config, &valid_files);

    // Build rayon thread pool matching --threads
    // Java's AnalysisQueue uses a fixed thread pool of size --threads
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(config.threads)
        .build()
        .map_err(|e| {
            eprintln!("Failed to create thread pool: {}", e);
            1
        })?;

    let failed = AtomicBool::new(something_failed);

    pool.install(|| {
        file_groups.par_iter().for_each(|group| {
            if !config.quiet {
                eprintln!("Started analysis of {}", group.name);
            }

            match process_group(config, &limits, group) {
                Ok(()) => {
                    if !config.quiet {
                        eprintln!("Analysis complete for {}", group.name);
                    }
                }
                Err(e) => {
                    eprintln!("Failed to process {}: {}", group.name, e);
                    failed.store(true, Ordering::Relaxed);
                }
            }
        });
    });

    if failed.load(Ordering::Relaxed) {
        Err(1)
    } else {
        Ok(())
    }
}

/// Build file groups based on the current mode (casava, or individual files).
///
/// Matches the grouping logic in OfflineRunner.java lines 103-117.
/// - If `--casava`: group by CASAVA basename
/// - Otherwise: each file is its own group
fn build_file_groups(config: &FastQCConfig, files: &[PathBuf]) -> Vec<FileGroup> {
    if config.casava {
        // CasavaBasename.getCasavaGroups() groups files by their
        // extracted basename. Files that don't match the pattern become singletons.
        let casava_groups = casava::get_casava_groups(files);
        casava_groups
            .into_iter()
            .map(|(name, paths)| FileGroup { name, files: paths })
            .collect()
    } else {
        // Default mode - each file is processed individually.
        // Java creates `fileGroups = new File[files.size()][1]` with one file per group.
        files
            .iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                FileGroup {
                    name,
                    files: vec![path.clone()],
                }
            })
            .collect()
    }
}

/// Process a file group (one or more files) through all QC modules and generate reports.
///
/// When a group has multiple files (CASAVA), they are combined
/// into a SequenceFileGroup that reads them sequentially as one logical sample.
fn process_group(
    config: &FastQCConfig,
    limits: &crate::config::Limits,
    group: &FileGroup,
) -> io::Result<()> {
    // Open the sequence file(s)
    let mut seq_file: Box<dyn SequenceFile> = if group.files.len() == 1 {
        // Single file - open directly
        // Uses format detection logic from SequenceFactory.java
        open_sequence_file(config, &group.files[0])?
    } else {
        // Multiple files in a CASAVA group - wrap them in a
        // SequenceFileGroup that reads all files sequentially as one stream.
        let mut readers: Vec<Box<dyn SequenceFile>> = Vec::new();
        for path in &group.files {
            readers.push(open_sequence_file(config, path)?);
        }
        Box::new(SequenceFileGroup::new(group.name.clone(), readers))
    };

    let file_display_name = group.name.clone();

    // Create module instances
    let mut modules = modules::create_modules(config, limits);

    // Set the filename on all modules (BasicStats uses it for the report)
    for module in modules.iter_mut() {
        module.set_filename(&file_display_name);
    }

    // Process all sequences through all modules
    // Matches AnalysisRunner.java:64-126
    let mut sequence_count: u64 = 0;
    let mut last_percent: i32 = -1;

    loop {
        match seq_file.next() {
            Some(Ok(seq)) => {
                sequence_count += 1;

                for module in modules.iter_mut() {
                    // Skip filtered sequences for modules that request it
                    if seq.is_filtered && module.ignore_filtered_sequences() {
                        continue;
                    }
                    module.process_sequence(&seq);
                }

                // Progress reporting every 5%
                if !config.quiet && sequence_count.is_multiple_of(1000) {
                    let percent = seq_file.percent_complete() as i32;
                    if percent != last_percent && percent % 5 == 0 {
                        eprintln!("Approx {}% complete for {}", percent, file_display_name);
                        last_percent = percent;
                    }
                }
            }
            Some(Err(e)) => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, e));
            }
            None => break, // EOF
        }
    }

    // Finalize all modules (lazy computation)
    for module in modules.iter_mut() {
        module.finalize();
    }

    // Generate output filename
    // For CASAVA groups, the display name is used as the base for
    // output files. For single files, it's the filename.
    // Strip extensions in order: .gz, .bz2, .txt, .fastq, .fq, .csfastq, .sam, .bam, .ubam
    let base_name = strip_extensions(&file_display_name.replace("stdin:", ""));

    // For output directory, use --outdir if specified, otherwise
    // use the parent directory of the first file in the group.
    let output_dir = if let Some(ref dir) = config.output_dir {
        dir.clone()
    } else {
        group
            .files
            .first()
            .and_then(|f| f.parent())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };

    // The Java code creates files at:
    //   {output_dir}/{base_name}_fastqc.html  (standalone HTML)
    //   {output_dir}/{base_name}_fastqc.zip   (zip archive)
    let html_path = output_dir.join(format!("{}_fastqc.html", base_name));
    let zip_path = output_dir.join(format!("{}_fastqc.zip", base_name));

    // Generate HTML report as a string (used for both standalone file and zip entry)
    let html_content = report::html::generate_html_report(&modules, &file_display_name)?;

    // Write standalone HTML file
    // The Java code writes the HTML via PrintWriter after creating the zip
    std::fs::write(&html_path, &html_content)?;

    // Create zip archive, reusing the already-generated HTML content
    report::archive::create_zip_archive(
        &modules,
        &file_display_name,
        &base_name,
        &zip_path,
        &html_content,
        config.svg_output,
    )?;

    // Handle --extract flag
    // If do_unzip is true, extract the zip file to the output directory.
    // If do_unzip is None (not specified), do not extract (matches Java default).
    if config.do_unzip == Some(true) {
        report::archive::extract_zip(&zip_path)?;

        // Handle --delete flag (only effective when --extract is also used)
        // Matches FastQCConfig.delete_after_unzip behavior
        if config.delete_after_unzip {
            std::fs::remove_file(&zip_path)?;
        }
    }

    Ok(())
}

/// Strip known sequencing file extensions from a filename.
///
/// Matches the exact chain of replaceAll calls in OfflineRunner.java:181
fn strip_extensions(name: &str) -> String {
    let mut result = name.to_string();
    // Strip in this exact order, matching Java's replaceAll chain
    for ext in &[
        ".gz", ".bz2", ".txt", ".fastq", ".fq", ".csfastq", ".sam", ".bam", ".ubam", ".fast5",
    ] {
        if result.ends_with(ext) {
            result = result[..result.len() - ext.len()].to_string();
        }
    }
    result
}

/// Recursively find all .fast5 files within a directory.
///
/// In --nano mode, Java's OfflineRunner recursively searches directories
/// for .fast5 files to process.
fn find_fast5_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    find_fast5_files_recursive(dir, &mut files)?;
    files.sort(); // Deterministic ordering
    Ok(files)
}

fn find_fast5_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            find_fast5_files_recursive(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("fast5"))
        {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_strip_extensions() {
        assert_eq!(strip_extensions("sample.fastq"), "sample");
        assert_eq!(strip_extensions("sample.fastq.gz"), "sample");
        assert_eq!(strip_extensions("sample.fq.bz2"), "sample");
        assert_eq!(strip_extensions("sample.bam"), "sample");
        assert_eq!(strip_extensions("sample.sam"), "sample");
        assert_eq!(strip_extensions("sample.txt.gz"), "sample");
        assert_eq!(strip_extensions("minimal.fastq"), "minimal");
    }

    #[test]
    fn test_build_file_groups_default() {
        let config = FastQCConfig::default();
        let files = vec![PathBuf::from("a.fastq"), PathBuf::from("b.fastq")];
        let groups = build_file_groups(&config, &files);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "a.fastq");
        assert_eq!(groups[0].files.len(), 1);
        assert_eq!(groups[1].name, "b.fastq");
        assert_eq!(groups[1].files.len(), 1);
    }

    #[test]
    fn test_build_file_groups_casava() {
        let config = FastQCConfig {
            casava: true,
            ..FastQCConfig::default()
        };
        let files = vec![
            PathBuf::from("Sample_S1_L001_R1_001.fastq.gz"),
            PathBuf::from("Sample_S1_L001_R1_002.fastq.gz"),
            PathBuf::from("Other_S2_L001_R1_001.fastq.gz"),
        ];
        let groups = build_file_groups(&config, &files);
        assert_eq!(groups.len(), 2);

        // Find the Sample group
        let sample_group = groups
            .iter()
            .find(|g| g.name == "Sample_S1_L001_R1.fastq.gz")
            .unwrap();
        assert_eq!(sample_group.files.len(), 2);

        // Find the Other group
        let other_group = groups
            .iter()
            .find(|g| g.name == "Other_S2_L001_R1.fastq.gz")
            .unwrap();
        assert_eq!(other_group.files.len(), 1);
    }

    #[test]
    fn test_build_file_groups_stdin() {
        let config = FastQCConfig::default();
        let files = vec![PathBuf::from("stdin")];
        let groups = build_file_groups(&config, &files);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "stdin");
    }
}
