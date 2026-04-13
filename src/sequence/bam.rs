// BAM/SAM file reader
// Corresponds to Sequence/BAMFile.java

use std::fs::File;
use std::io::{self, BufReader, Seek};
use std::path::Path;

use noodles::bam;
use noodles::bgzf;
use noodles::sam;
use noodles::sam::alignment::record::cigar::op::Kind as CigarKind;
// Import the Sequence trait so .iter() is available on SAM record sequences
use noodles::sam::alignment::record::Sequence as _;

use super::{Sequence, SequenceFile};
use crate::utils::dna::reverse_complement;

// ---------------------------------------------------------------------------
// Reader abstraction over BAM and SAM formats
// ---------------------------------------------------------------------------

/// Wrapper enum to unify BAM and SAM record iteration behind a single type.
///
/// Java uses HTSJDK's SamReader which auto-detects format.
/// We dispatch explicitly based on file extension/format config.
enum InnerReader {
    Bam(bam::io::Reader<bgzf::Reader<File>>),
    Sam(sam::io::Reader<BufReader<File>>),
}

// ---------------------------------------------------------------------------
// BAMFile
// ---------------------------------------------------------------------------

/// BAM/SAM file reader that supports both BAM and SAM input formats.
///
/// Mirrors `Sequence.BAMFile` in Java. Uses a look-ahead design
/// where `read_next()` is called at construction time and after each `next()` call.
pub struct BAMFile {
    reader: InnerReader,
    name: String,
    file_size: u64,
    only_mapped: bool,
    /// Cloned file handle for progress tracking via seek position.
    position_handle: Option<File>,

    /// The next sequence ready to be returned (look-ahead buffer).
    /// Matches Java's `nextSequence` field.
    next_sequence: Option<Sequence>,

    /// Whether this is a BAM (true) or SAM (false) file, used for progress estimation.
    is_bam: bool,

    /// Rough record size for progress estimation.
    /// Java computes this as `(readLength * 2) + 150`, divided by 4 for BAM.
    record_size: u64,

    // Reusable record buffers to avoid allocations per record.
    bam_record: bam::Record,
    sam_record: sam::Record,
}

impl BAMFile {
    /// Open a BAM or SAM file for reading.
    ///
    /// If `is_bam` is true, the file is read as BAM format; otherwise as SAM.
    /// If `only_mapped` is true, unmapped reads are skipped.
    ///
    /// The Java constructor opens the file, creates a SamReader with
    /// SILENT validation stringency, gets an iterator, and calls readNext() to prime
    /// the look-ahead buffer.
    pub fn open(path: &Path, is_bam: bool, only_mapped: bool) -> io::Result<Self> {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        let file_size = std::fs::metadata(path)?.len();

        // Clone the file handle before passing to the reader so we can
        // query the compressed byte position for progress tracking.
        let file = File::open(path)?;
        let position_handle = file.try_clone().ok();

        let reader = if is_bam {
            let mut r = bam::io::Reader::new(file);
            let _header = r.read_header()?;
            InnerReader::Bam(r)
        } else {
            let buf = BufReader::new(file);
            let mut r = sam::io::Reader::new(buf);
            let _header = r.read_header()?;
            InnerReader::Sam(r)
        };

        let mut bam_file = BAMFile {
            reader,
            name,
            file_size,
            only_mapped,
            position_handle,
            next_sequence: None,
            is_bam,
            record_size: 0,
            bam_record: bam::Record::default(),
            sam_record: sam::Record::default(),
        };

        // Prime the look-ahead buffer by reading the first record.
        bam_file.read_next()?;

        Ok(bam_file)
    }

    /// Read the next record and convert it to a Sequence, storing in `self.next_sequence`.
    ///
    /// This mirrors `readNext()` in BAMFile.java, including:
    /// - Skipping unmapped reads when `only_mapped` is true
    /// - Soft-clip trimming for mapped-only mode
    /// - Reverse complementing reads on the negative strand
    fn read_next(&mut self) -> io::Result<()> {
        loop {
            // Read the next record from the appropriate reader type
            let record_data = match &mut self.reader {
                InnerReader::Bam(reader) => {
                    match reader.read_record(&mut self.bam_record) {
                        Ok(0) => None, // EOF
                        Ok(_) => {
                            let rec = &self.bam_record;
                            let flags = rec.flags();

                            // Skip unmapped reads if only_mapped is set.
                            // Java: `if (onlyMapped && record.getReadUnmappedFlag()) continue;`
                            if self.only_mapped && flags.is_unmapped() {
                                continue;
                            }

                            // Extract fields from BAM record
                            let name = rec
                                .name()
                                .map(|n| String::from_utf8_lossy(n.as_ref()).into_owned())
                                .unwrap_or_else(|| "*".to_string());

                            let sequence_bases: Vec<u8> = rec.sequence().iter().collect();
                            let seq_len = sequence_bases.len();

                            // BAM quality scores are raw Phred values (0-93).
                            // Java's HTSJDK `getBaseQualityString()` converts them to ASCII
                            // by adding 33 (Sanger/Phred+33 encoding). We do the same.
                            let quality_bytes: Vec<u8> = rec
                                .quality_scores()
                                .as_ref()
                                .iter()
                                .map(|&q| q + 33) // Convert Phred+0 to ASCII Phred+33
                                .collect();

                            // Collect CIGAR ops for soft-clip handling
                            let cigar_ops: Vec<(CigarKind, usize)> = rec
                                .cigar()
                                .iter()
                                .filter_map(|r| r.ok())
                                .map(|op| (op.kind(), op.len()))
                                .collect();

                            let is_reverse = flags.is_reverse_complemented();

                            // Check vendor quality failure flag (0x200)
                            // for CASAVA-style filtering.
                            let is_qc_fail = flags.is_qc_fail();

                            // Estimate record size for progress tracking.
                            // Java: `recordSize = (record.getReadLength()*2)+150`
                            // then divides by 4 for BAM format.
                            if self.record_size == 0 && seq_len > 0 {
                                let mut rs = (seq_len as u64 * 2) + 150;
                                if self.is_bam {
                                    rs /= 4; // BAM records are ~4x smaller than SAM
                                }
                                self.record_size = rs;
                            }

                            Some(RecordData {
                                name,
                                sequence: sequence_bases,
                                quality: quality_bytes,
                                cigar_ops,
                                is_reverse,
                                is_qc_fail,
                            })
                        }
                        Err(e) => return Err(e),
                    }
                }
                InnerReader::Sam(reader) => {
                    match reader.read_record(&mut self.sam_record) {
                        Ok(0) => None, // EOF
                        Ok(_) => {
                            let rec = &self.sam_record;
                            let flags = rec
                                .flags()
                                .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

                            // Skip unmapped reads if only_mapped is set.
                            if self.only_mapped && flags.is_unmapped() {
                                continue;
                            }

                            let name = rec
                                .name()
                                .map(|n| String::from_utf8_lossy(n.as_ref()).into_owned())
                                .unwrap_or_else(|| "*".to_string());

                            // SAM sequence is stored as ASCII text
                            let sequence_bases: Vec<u8> = rec.sequence().iter().collect();
                            let seq_len = sequence_bases.len();

                            // SAM quality scores from HTSJDK's getBaseQualityString()
                            // are already in Phred+33 ASCII encoding. The noodles SAM reader
                            // also stores them as raw ASCII bytes, so we use them directly.
                            let quality_bytes: Vec<u8> = rec.quality_scores().as_ref().to_vec();

                            // Collect CIGAR ops
                            let cigar_ops: Vec<(CigarKind, usize)> = rec
                                .cigar()
                                .iter()
                                .filter_map(|r| r.ok())
                                .map(|op| (op.kind(), op.len()))
                                .collect();

                            let is_reverse = flags.is_reverse_complemented();
                            let is_qc_fail = flags.is_qc_fail();

                            if self.record_size == 0 && seq_len > 0 {
                                let mut rs = (seq_len as u64 * 2) + 150;
                                if self.is_bam {
                                    rs /= 4;
                                }
                                self.record_size = rs;
                            }

                            Some(RecordData {
                                name,
                                sequence: sequence_bases,
                                quality: quality_bytes,
                                cigar_ops,
                                is_reverse,
                                is_qc_fail,
                            })
                        }
                        Err(e) => return Err(e),
                    }
                }
            };

            match record_data {
                None => {
                    // EOF
                    self.next_sequence = None;
                    return Ok(());
                }
                Some(data) => {
                    let mut sequence = data.sequence;
                    let mut quality = data.quality;

                    // If only working with mapped data, exclude soft-clipped
                    // regions from the sequence and quality. Java clips the 3' end first
                    // (so that 5' indices remain correct), then clips the 5' end.
                    if self.only_mapped && !data.cigar_ops.is_empty() {
                        // Clip 3' end first (last CIGAR element)
                        // Java checks `elements.get(elements.size()-1).getOperator().equals(CigarOperator.S)`
                        if let Some(&(kind, len)) = data.cigar_ops.last() {
                            if kind == CigarKind::SoftClip {
                                let new_len = sequence.len().saturating_sub(len);
                                sequence.truncate(new_len);
                                quality.truncate(new_len);
                            }
                        }

                        // Clip 5' end (first CIGAR element)
                        // Java checks `elements.get(0).getOperator().equals(CigarOperator.S)`
                        if let Some(&(kind, len)) = data.cigar_ops.first() {
                            if kind == CigarKind::SoftClip {
                                // Java uses `sequence.substring(value)` which
                                // drops the first `value` characters.
                                if len <= sequence.len() {
                                    sequence = sequence[len..].to_vec();
                                    quality = quality[len..].to_vec();
                                }
                            }
                        }
                    }

                    // BAM/SAM files always show sequence relative to the top
                    // strand of the mapped reference. If this read maps to the reverse strand,
                    // we reverse complement the sequence and reverse the qualities to recover
                    // the original read orientation. Java does this unconditionally when
                    // getReadNegativeStrandFlag() is true, regardless of only_mapped.
                    if data.is_reverse {
                        sequence = reverse_complement(&sequence);
                        quality.reverse();
                    }

                    let mut seq = Sequence::new(data.name, sequence, quality);

                    // Java's BAMFile.java does not explicitly handle CASAVA
                    // filtering via the read name (that is a FastQ convention). However,
                    // the SAM flag 0x200 (QC fail / vendor quality check failure) serves
                    // a similar purpose. We set is_filtered when the QC fail flag is set.
                    if data.is_qc_fail {
                        seq.is_filtered = true;
                    }

                    self.next_sequence = Some(seq);
                    return Ok(());
                }
            }
        }
    }
}

/// Intermediate struct holding extracted record fields, used to avoid
/// borrow-checker issues with the reader and record.
struct RecordData {
    name: String,
    sequence: Vec<u8>,
    quality: Vec<u8>,
    cigar_ops: Vec<(CigarKind, usize)>,
    is_reverse: bool,
    is_qc_fail: bool,
}

impl SequenceFile for BAMFile {
    fn next(&mut self) -> Option<io::Result<Sequence>> {
        // Java's `next()` returns the current `nextSequence` then calls
        // `readNext()` to prime the next one. Same pattern as FastQFile.
        let current = self.next_sequence.take()?;
        if let Err(e) = self.read_next() {
            return Some(Err(e));
        }
        Some(Ok(current))
    }

    fn name(&self) -> &str {
        &self.name
    }

    /// BAM/SAM files are never colorspace.
    /// Java's `BAMFile.isColorspace()` always returns false.
    fn is_colorspace(&self) -> bool {
        false
    }

    /// Java tracks progress using the raw FileInputStream position
    /// divided by file size. For BAM files, this gives a rough estimate since
    /// the underlying file is BGZF compressed. We estimate using the record size
    /// heuristic from Java when precise position tracking is not available.
    fn percent_complete(&self) -> f64 {
        if self.next_sequence.is_none() {
            return 100.0;
        }
        // Track progress via compressed byte position, same as FASTQ reader.
        if let Some(ref handle) = self.position_handle {
            if let Ok(mut h) = handle.try_clone() {
                if let Ok(pos) = h.stream_position() {
                    return (pos as f64 / self.file_size as f64) * 100.0;
                }
            }
        }
        0.0
    }
}

// ---------------------------------------------------------------------------
// Format detection and factory function
// ---------------------------------------------------------------------------

/// Detect the file format from the config or file extension, and open the appropriate reader.
///
/// This mirrors the format detection logic in `SequenceFactory.java`:
/// - If `config.sequence_format` is set, use that explicitly
/// - Otherwise, detect by file extension
///
/// Returns a boxed `SequenceFile` trait object.
pub fn open_sequence_file(
    config: &crate::config::FastQCConfig,
    path: &Path,
) -> io::Result<Box<dyn SequenceFile>> {
    use super::fastq::FastQFile;

    // Java checks FastQCConfig.getInstance().getSequenceFormat() first.
    // If set, it overrides extension-based detection.
    if let Some(ref format) = config.sequence_format {
        let format_lower = format.to_lowercase();
        match format_lower.as_str() {
            // "bam" format reads all records (mapped + unmapped)
            "bam" => {
                return Ok(Box::new(BAMFile::open(path, true, false)?));
            }
            // "sam" format reads all records
            "sam" => {
                return Ok(Box::new(BAMFile::open(path, false, false)?));
            }
            // "bam_mapped" reads only mapped records from BAM
            "bam_mapped" => {
                return Ok(Box::new(BAMFile::open(path, true, true)?));
            }
            // "sam_mapped" reads only mapped records from SAM
            "sam_mapped" => {
                return Ok(Box::new(BAMFile::open(path, false, true)?));
            }
            // "fastq" or "fastq.gz" or similar -> FastQFile
            "fastq" => {
                return Ok(Box::new(FastQFile::open(config, path)?));
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Unknown sequence format: {}", other),
                ));
            }
        }
    }

    // Extension-based detection from SequenceFactory.java
    let name_lower = path
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if name_lower.ends_with(".bam") || name_lower.ends_with(".ubam") {
        // .bam and .ubam files are opened as BAM with all reads
        Ok(Box::new(BAMFile::open(path, true, false)?))
    } else if name_lower.ends_with(".sam") {
        // .sam files are opened as SAM with all reads
        Ok(Box::new(BAMFile::open(path, false, false)?))
    } else if name_lower.ends_with(".fast5") {
        // Open Fast5 (Nanopore HDF5) files via pure-Rust hdf5-pure crate
        Ok(Box::new(super::fast5::Fast5File::open(path)?))
    } else {
        // Everything else is assumed to be FASTQ
        Ok(Box::new(FastQFile::open(config, path)?))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_detection_fast5() {
        // Fast5 files are now supported but still fail if the file doesn't exist
        let config = crate::config::FastQCConfig::default();
        let result = open_sequence_file(&config, Path::new("nonexistent.fast5"));
        assert!(result.is_err());
    }

    #[test]
    fn test_format_detection_unknown_format() {
        let config = crate::config::FastQCConfig {
            sequence_format: Some("unknown_format".to_string()),
            ..Default::default()
        };
        let result = open_sequence_file(&config, Path::new("test.dat"));
        assert!(result.is_err());
    }
}
