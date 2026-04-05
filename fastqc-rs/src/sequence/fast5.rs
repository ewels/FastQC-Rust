// Fast5 (Nanopore HDF5) file reader
// Corresponds to Sequence/Fast5File.java

use std::io;
use std::path::{Path, PathBuf};

use hdf5_pure::File as Hdf5File;

use crate::sequence::{Sequence, SequenceFile};

/// HDF5 paths tried in order to find FASTQ data within each read.
/// Matches the rdfPaths array in Fast5File.java.
const FASTQ_PATHS: &[&str] = &[
    "Analyses/Basecall_2D_000/BaseCalled_template/Fastq",
    "Analyses/Basecall_2D_000/BaseCalled_2D/Fastq",
    "Analyses/Basecall_1D_000/BaseCalled_template/Fastq",
    "Analyses/Basecall_1D_000/BaseCalled_1D/Fastq",
];

pub struct Fast5File {
    /// Pre-loaded sequences from the HDF5 file.
    /// Java reads lazily via readPaths iterator. We pre-load all sequences
    /// at open time since the file must be fully in memory for hdf5-pure anyway.
    sequences: Vec<Sequence>,
    /// Current position in the sequences vector.
    current_index: usize,
    /// Display name of the file.
    name: String,
    #[allow(dead_code)]
    file_path: PathBuf,
}

impl Fast5File {
    /// Open a Fast5 file and extract all FASTQ sequences from it.
    ///
    /// Matches Fast5File constructor. Handles both single-read files
    /// (FASTQ data at top level) and multi-read files (read_XXX/ subfolders).
    pub fn open<P: AsRef<Path>>(path: P) -> io::Result<Self> {
        let path = path.as_ref();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        let hdf5 = Hdf5File::open(path).map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to open HDF5 file {}: {}", path.display(), e),
            )
        })?;

        // Check for multi-read structure (read_XXX/ folders at top level).
        // If present, each subfolder is a separate read. Otherwise, the top level IS the read.
        let root = hdf5.root();
        let top_groups = root.groups().map_err(|e| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("Failed to list groups in {}: {}", path.display(), e),
            )
        })?;

        let read_prefixes: Vec<String> = top_groups
            .into_iter()
            .filter(|name| name.starts_with("read_"))
            .map(|name| format!("{}/", name))
            .collect();

        // If we found read_ folders, use them. Otherwise use "" (top level).
        let prefixes = if read_prefixes.is_empty() {
            vec![String::new()]
        } else {
            read_prefixes
        };

        let mut sequences = Vec::new();

        for prefix in &prefixes {
            match Self::read_fastq_from_prefix(&hdf5, prefix) {
                Ok(seq) => sequences.push(seq),
                Err(e) => {
                    // Java throws SequenceFormatException if no valid paths found.
                    // We collect what we can and report errors for individual reads.
                    eprintln!(
                        "Warning: Could not extract FASTQ from {}{}: {}",
                        path.display(),
                        prefix,
                        e
                    );
                }
            }
        }

        if sequences.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("No valid FASTQ paths found in {}", path.display()),
            ));
        }

        Ok(Fast5File {
            sequences,
            current_index: 0,
            name,
            file_path: path.to_path_buf(),
        })
    }

    /// Try to read a FASTQ record from the given prefix path within the HDF5 file.
    ///
    /// Tries each of the rdfPaths in order until one is found.
    fn read_fastq_from_prefix(hdf5: &Hdf5File, prefix: &str) -> io::Result<Sequence> {
        for &fastq_path in FASTQ_PATHS {
            let full_path = format!("{}{}", prefix, fastq_path);

            if let Ok(dataset) = hdf5.dataset(&full_path) {
                // Try read_string() first (works for fixed-length HDF5 strings).
                // Fall back to read_u8() for variable-length strings (common in real Fast5 files
                // written by ONT tools), then decode as UTF-8.
                if let Ok(strings) = dataset.read_string() {
                    if let Some(fastq_str) = strings.first() {
                        return Self::parse_fastq_string(fastq_str);
                    }
                } else if let Ok(bytes) = dataset.read_u8() {
                    // Variable-length string: raw bytes, interpret as UTF-8
                    let fastq_str = String::from_utf8(bytes).map_err(|e| {
                        io::Error::new(io::ErrorKind::InvalidData, e)
                    })?;
                    return Self::parse_fastq_string(&fastq_str);
                }
            }
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("No valid FASTQ data found at prefix '{}'", prefix),
        ))
    }

    /// Parse a 4-line FASTQ string from HDF5 into a Sequence.
    ///
    /// Matches the `fastq.split("\\n")` parsing in Fast5File.next().
    fn parse_fastq_string(fastq: &str) -> io::Result<Sequence> {
        let lines: Vec<&str> = fastq.split('\n').collect();

        if lines.len() < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Didn't get 4 sections from FASTQ string (got {})",
                    lines.len()
                ),
            ));
        }

        let id = lines[0].to_string();
        let seq_bytes = lines[1].as_bytes().to_vec();
        let qual_bytes = lines[3].as_bytes().to_vec();

        // Sequence::new handles uppercase conversion
        Ok(Sequence::new(id, seq_bytes, qual_bytes))
    }
}

impl SequenceFile for Fast5File {
    fn next(&mut self) -> Option<io::Result<Sequence>> {
        if self.current_index >= self.sequences.len() {
            return None;
        }

        // Take ownership via swap with a cheap default, avoiding a full clone
        // of the id, sequence, and quality vectors for every read.
        let mut seq = Sequence::new(String::new(), Vec::new(), Vec::new());
        std::mem::swap(&mut seq, &mut self.sequences[self.current_index]);
        self.current_index += 1;
        Some(Ok(seq))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_colorspace(&self) -> bool {
        false
    }

    fn percent_complete(&self) -> f64 {
        if self.current_index >= self.sequences.len() {
            return 100.0;
        }
        // (readPathsIndexPosition * 100) / readPaths.length
        (self.current_index as f64 * 100.0) / self.sequences.len() as f64
    }
}
