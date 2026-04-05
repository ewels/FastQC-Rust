// CASAVA basename extraction and file grouping
// Corresponds to Utilities/CasavaBasename.java

use std::collections::HashMap;
use std::path::PathBuf;

/// Error returned when a filename does not match the CASAVA naming convention.
///
/// Mirrors `Utilities.NameFormatException` in Java.
#[derive(Debug)]
pub struct NameFormatError;

impl std::fmt::Display for NameFormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Filename does not match CASAVA naming convention")
    }
}

impl std::error::Error for NameFormatError {}

/// Extract the CASAVA basename from a filename.
///
/// CASAVA filenames follow the pattern:
///   `{SampleName}_{Barcode}_{Lane}_{Read}_{FlowCellChunkNumber}.fastq.gz`
///
/// This strips the `_{3digits}` before the `.fastq[.gz]` extension, yielding the
/// sample group name. Files sharing the same basename are treated as one logical sample.
///
/// This is a direct translation of `CasavaBasename.getCasavaBasename()`.
/// The Java code uses character-position arithmetic rather than regex to parse
/// the trailing `_NNN.fastq[.gz]` pattern. We replicate the exact same index
/// math to ensure identical grouping behavior.
pub fn get_casava_basename(original_name: &str) -> Result<String, NameFormatError> {
    // The Java code checks two cases: `.fastq.gz` and `.fastq`.
    // For `.fastq.gz`: expects `_` at position len-13, then 3 digits at len-12..len-9.
    // For `.fastq`:    expects `_` at position len-10, then 3 digits at len-9..len-6.

    if original_name.ends_with(".fastq.gz") {
        let len = original_name.len();
        // Check for '_' 13 chars before the end (i.e. before "NNN.fastq.gz")
        if len >= 13
            && &original_name[len - 13..len - 12] == "_"
            && original_name[len - 12..len - 9].parse::<u32>().is_ok()
        {
            // basename = everything before the underscore + ".fastq.gz"
            let base_name = format!("{}.fastq.gz", &original_name[..len - 13]);
            return Ok(base_name);
        }
    } else if original_name.ends_with(".fastq") {
        let len = original_name.len();
        // Check for '_' 10 chars before the end (i.e. before "NNN.fastq")
        if len >= 10
            && &original_name[len - 10..len - 9] == "_"
            && original_name[len - 9..len - 6].parse::<u32>().is_ok()
        {
            // basename = everything before the underscore + ".fastq"
            let base_name = format!("{}.fastq", &original_name[..len - 10]);
            return Ok(base_name);
        }
    }

    Err(NameFormatError)
}

/// Group files by their CASAVA basename.
///
/// Returns a `Vec` of `(basename, files)` tuples. Files whose names do not
/// conform to the CASAVA pattern are placed in their own singleton group.
///
/// Direct translation of `CasavaBasename.getCasavaGroups()`.
/// When a file does not match, Java prints a warning to stderr and adds
/// it as a singleton group keyed by the raw filename.
pub fn get_casava_groups(files: &[PathBuf]) -> Vec<(String, Vec<PathBuf>)> {
    // Java uses a Hashtable (unordered) for grouping. We use
    // an IndexMap-style approach with a Vec to preserve insertion order, but
    // the Java code does not guarantee any ordering either. Using a HashMap
    // here matches the Java Hashtable semantics.
    let mut groups: HashMap<String, Vec<PathBuf>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for file in files {
        let file_name = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();

        match get_casava_basename(&file_name) {
            Ok(base_name) => {
                if !groups.contains_key(&base_name) {
                    order.push(base_name.clone());
                }
                groups
                    .entry(base_name)
                    .or_default()
                    .push(file.clone());
            }
            Err(_) => {
                // Java prints warning and adds as singleton
                eprintln!(
                    "File '{}' didn't look like part of a CASAVA group",
                    file_name
                );
                order.push(file_name.clone());
                groups
                    .entry(file_name)
                    .or_default()
                    .push(file.clone());
            }
        }
    }

    order
        .into_iter()
        .filter_map(|key| {
            groups.remove(&key).map(|files| (key, files))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Nanopore basename extraction
// ---------------------------------------------------------------------------

/// Extract the Nanopore basename from a filename.
///
/// Nanopore filenames follow the pattern:
///   `Computer_Samplename_number[_chXXX_fileXXX_strand].fast5`
///
/// This extracts the first three underscore-separated components as the group name.
///
/// Direct translation of `NanoporeBasename.getNanoporeBasename()`.
/// The Java code splits on `_` after stripping `.fast5`, requires at least 3
/// parts, and joins the first three with underscores.
pub fn get_nanopore_basename(original_name: &str) -> Result<String, NameFormatError> {
    // Java does `originalName.replaceAll(".fast5$", "").split("_")`
    let stripped = original_name.strip_suffix(".fast5").unwrap_or(original_name);
    let sub_names: Vec<&str> = stripped.split('_').collect();

    if sub_names.len() < 3 {
        return Err(NameFormatError);
    }

    // Java joins first 3 components: `subNames[0]+"_"+subNames[1]+"_"+subNames[2]`
    let basename = format!("{}_{}_{}",sub_names[0], sub_names[1], sub_names[2]);

    // Java prints basename to stderr for debugging
    eprintln!("Basename is {}", basename);

    Ok(basename)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- CASAVA basename tests ----

    #[test]
    fn test_casava_basename_fastq_gz() {
        // Standard CASAVA filename with .fastq.gz
        assert_eq!(
            get_casava_basename("SampleA_S1_L001_R1_001.fastq.gz").unwrap(),
            "SampleA_S1_L001_R1.fastq.gz"
        );
    }

    #[test]
    fn test_casava_basename_fastq() {
        // CASAVA filename without compression
        assert_eq!(
            get_casava_basename("SampleA_S1_L001_R1_001.fastq").unwrap(),
            "SampleA_S1_L001_R1.fastq"
        );
    }

    #[test]
    fn test_casava_basename_different_chunk() {
        assert_eq!(
            get_casava_basename("SampleA_S1_L001_R1_042.fastq.gz").unwrap(),
            "SampleA_S1_L001_R1.fastq.gz"
        );
    }

    #[test]
    fn test_casava_basename_non_casava() {
        // Non-CASAVA filenames should fail
        assert!(get_casava_basename("sample.fastq.gz").is_err());
        assert!(get_casava_basename("sample.bam").is_err());
    }

    #[test]
    fn test_casava_basename_not_digits() {
        // The 3 chars before .fastq must be parseable as integer
        assert!(get_casava_basename("sample_abc.fastq.gz").is_err());
    }

    #[test]
    fn test_casava_groups() {
        let files = vec![
            PathBuf::from("SampleA_S1_L001_R1_001.fastq.gz"),
            PathBuf::from("SampleA_S1_L001_R1_002.fastq.gz"),
            PathBuf::from("SampleB_S2_L001_R1_001.fastq.gz"),
            PathBuf::from("non_casava.fastq.gz"),
        ];
        let groups = get_casava_groups(&files);

        // Should have 3 groups: SampleA, SampleB, and the non-casava singleton
        assert_eq!(groups.len(), 3);

        // Find the SampleA group
        let sample_a = groups
            .iter()
            .find(|(name, _)| name == "SampleA_S1_L001_R1.fastq.gz")
            .unwrap();
        assert_eq!(sample_a.1.len(), 2);

        // Find the SampleB group
        let sample_b = groups
            .iter()
            .find(|(name, _)| name == "SampleB_S2_L001_R1.fastq.gz")
            .unwrap();
        assert_eq!(sample_b.1.len(), 1);
    }

    // ---- Nanopore basename tests ----

    #[test]
    fn test_nanopore_basename() {
        assert_eq!(
            get_nanopore_basename("Computer_Sample_123_ch100_file0_strand.fast5").unwrap(),
            "Computer_Sample_123"
        );
    }

    #[test]
    fn test_nanopore_basename_short() {
        // Newer format with just 3 components
        assert_eq!(
            get_nanopore_basename("Computer_Sample_123.fast5").unwrap(),
            "Computer_Sample_123"
        );
    }

    #[test]
    fn test_nanopore_basename_too_few() {
        assert!(get_nanopore_basename("Computer_Sample.fast5").is_err());
    }
}
