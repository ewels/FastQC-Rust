pub mod bam;
pub mod casava;
pub mod fast5;
pub mod fastq;
pub mod group;

pub use bam::open_sequence_file;
pub use group::SequenceFileGroup;

/// A single sequence record with ID, bases, quality scores, and filter status.
///
/// Mirrors `Sequence.Sequence` in Java. The `sequence` field stores
/// uppercase ASCII bases as bytes (matching Java's `toUpperCase()` in the constructor).
/// The `quality` field stores raw ASCII quality characters as bytes.
#[derive(Debug, Clone)]
pub struct Sequence {
    pub id: String,
    /// Uppercase ASCII nucleotide bases (A, C, G, T, N).
    pub sequence: Vec<u8>,
    /// Raw ASCII quality characters (not yet offset-adjusted).
    pub quality: Vec<u8>,
    /// Whether this sequence was flagged as filtered (e.g. CASAVA filtered).
    pub is_filtered: bool,
    /// Colorspace representation, if applicable (SOLiD data).
    pub colorspace: Option<Vec<u8>>,
}

impl Sequence {
    /// Create a new Sequence, converting the base sequence to uppercase.
    ///
    /// The Java constructor calls `sequence.toUpperCase()` on the
    /// sequence string, so we replicate that here.
    pub fn new(id: String, mut sequence: Vec<u8>, quality: Vec<u8>) -> Self {
        // uppercase conversion matches Java constructor behavior.
        // In-place mutation avoids allocating a new Vec.
        sequence.make_ascii_uppercase();
        Self {
            id,
            sequence,
            quality,
            is_filtered: false,
            colorspace: None,
        }
    }

    /// Length of the sequence in bases.
    pub fn len(&self) -> usize {
        self.sequence.len()
    }

    /// Whether the sequence is empty.
    pub fn is_empty(&self) -> bool {
        self.sequence.is_empty()
    }
}

/// Trait for reading sequences from various file formats.
///
/// Mirrors `Sequence.SequenceFile` interface.
pub trait SequenceFile: Send {
    /// Read the next sequence from the file, or None at EOF.
    fn next(&mut self) -> Option<std::io::Result<Sequence>>;

    /// The display name of this file (typically the filename).
    fn name(&self) -> &str;

    /// Whether this file contains colorspace data (SOLiD).
    fn is_colorspace(&self) -> bool;

    /// Estimated percentage complete (0.0 - 100.0), for progress display.
    fn percent_complete(&self) -> f64;
}
