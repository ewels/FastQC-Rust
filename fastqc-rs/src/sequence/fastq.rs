// FASTQ file reader
// Corresponds to Sequence/FastQFile.java

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, Stdin};
use std::path::Path;

use bzip2_rs::DecoderReader;
use flate2::read::MultiGzDecoder;

use super::{Sequence, SequenceFile};
use crate::config::FastQCConfig;

// ---------------------------------------------------------------------------
// Decompression layer
// ---------------------------------------------------------------------------

/// Wrapper enum so we can store different reader types without trait objects.
/// Each variant wraps a `BufReader` around the appropriate decompression stream.
enum ReaderKind {
    Plain(BufReader<File>),
    Gzip(BufReader<MultiGzDecoder<File>>),
    Bzip2(Box<BufReader<DecoderReader<File>>>),
    Stdin(BufReader<Stdin>),
}

impl BufRead for ReaderKind {
    fn fill_buf(&mut self) -> io::Result<&[u8]> {
        match self {
            ReaderKind::Plain(r) => r.fill_buf(),
            ReaderKind::Gzip(r) => r.fill_buf(),
            ReaderKind::Bzip2(r) => r.fill_buf(),
            ReaderKind::Stdin(r) => r.fill_buf(),
        }
    }

    fn consume(&mut self, amt: usize) {
        match self {
            ReaderKind::Plain(r) => r.consume(amt),
            ReaderKind::Gzip(r) => r.consume(amt),
            ReaderKind::Bzip2(r) => r.consume(amt),
            ReaderKind::Stdin(r) => r.consume(amt),
        }
    }
}

impl Read for ReaderKind {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            ReaderKind::Plain(r) => r.read(buf),
            ReaderKind::Gzip(r) => r.read(buf),
            ReaderKind::Bzip2(r) => r.read(buf),
            ReaderKind::Stdin(r) => r.read(buf),
        }
    }
}

// ---------------------------------------------------------------------------
// Compression detection
// ---------------------------------------------------------------------------

/// Detect compression from the first two bytes (magic numbers).
/// Returns "gz", "bz2", or "none".
fn detect_compression_from_magic(path: &Path) -> io::Result<&'static str> {
    let mut f = File::open(path)?;
    let mut magic = [0u8; 2];
    let n = f.read(&mut magic)?;
    if n >= 2 {
        // Java detects gzip via file extension or MIME type probing which
        // checks magic bytes 1f 8b internally. We replicate by checking magic directly.
        if magic[0] == 0x1f && magic[1] == 0x8b {
            return Ok("gz");
        }
        // Java only checks .bz2 extension, but we also check magic bytes
        // 42 5a ('BZ') for robustness.
        if magic[0] == 0x42 && magic[1] == 0x5a {
            return Ok("bz2");
        }
    }
    Ok("none")
}

// ---------------------------------------------------------------------------
// FastQFile
// ---------------------------------------------------------------------------

/// FASTQ file reader that supports plain text, gzip, and bzip2 compressed files.
///
/// Mirrors `Sequence.FastQFile` in Java. Uses a look-ahead design
/// where `readNext()` is called at construction time and after each `next()` call,
/// so `hasNext()` can report whether more sequences are available. In Rust we use
/// `Option<Sequence>` stored in `next_sequence` for the same pattern.
pub struct FastQFile {
    reader: ReaderKind,
    name: String,
    file_size: u64,
    /// Cloned file handle used solely to query the compressed byte position
    /// via seek(Current). Java does this with fis.getChannel().position().
    /// None for stdin (no file to track).
    position_handle: Option<File>,

    /// The next sequence ready to be returned (look-ahead buffer).
    next_sequence: Option<Sequence>,

    /// Current line number for error messages, incremented on every
    /// `readLine()` call exactly as in Java.
    line_number: u64,

    /// Whether colorspace was detected (checked on the first sequence only).
    is_colorspace: bool,
    /// Whether we have already checked for colorspace (first record only).
    colorspace_checked: bool,

    /// CASAVA filter mode flags.
    casava_mode: bool,
    nofilter: bool,

    /// The lowest raw quality character seen so far (for Phred encoding detection).
    pub lowest_char: u8,

    /// A reusable String buffer to avoid allocating on every `read_line`.
    line_buf: String,
}

impl FastQFile {
    /// Open a FASTQ file for reading.
    ///
    /// The Java constructor opens the file, wraps it in the
    /// appropriate decompression stream, and immediately calls `readNext()` to
    /// prime the look-ahead buffer.
    pub fn open<P: AsRef<Path>>(config: &FastQCConfig, path: P) -> io::Result<Self> {
        let path = path.as_ref();
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());

        let is_stdin = name.starts_with("stdin");

        // For stdin, Java sets fileSize to Long.MAX_VALUE.
        let file_size = if is_stdin {
            u64::MAX
        } else {
            std::fs::metadata(path)?.len()
        };

        // Java keeps the raw FileInputStream (fis) and queries
        // fis.getChannel().position() for progress tracking. We clone the File
        // handle before wrapping it in decompression so we can seek on the clone
        // to get the compressed byte position.
        let (reader, position_handle) = if is_stdin {
            (ReaderKind::Stdin(BufReader::new(io::stdin())), None)
        } else {
            let lower_name = name.to_lowercase();
            let compression = if lower_name.ends_with(".gz") {
                "gz"
            } else if lower_name.ends_with(".bz2") {
                "bz2"
            } else {
                detect_compression_from_magic(path)?
            };

            let file = File::open(path)?;
            let pos_handle = file.try_clone()?;

            let rdr = match compression {
                "gz" => ReaderKind::Gzip(BufReader::new(MultiGzDecoder::new(file))),
                "bz2" => ReaderKind::Bzip2(Box::new(BufReader::new(DecoderReader::new(file)))),
                _ => ReaderKind::Plain(BufReader::new(file)),
            };
            (rdr, Some(pos_handle))
        };

        let casava_mode = config.casava;
        let nofilter = config.nofilter;

        let mut fq = FastQFile {
            reader,
            name,
            file_size,
            position_handle,
            next_sequence: None,
            line_number: 0,
            is_colorspace: false,
            colorspace_checked: false,
            casava_mode,
            nofilter,
            lowest_char: 255,
            line_buf: String::with_capacity(512),
        };

        // Prime the look-ahead buffer by reading the first record.
        fq.read_next()?;

        Ok(fq)
    }

    /// Read a single line into `self.line_buf`, incrementing `line_number`.
    /// Returns `true` if a line was read, `false` at EOF.
    fn read_line(&mut self) -> io::Result<bool> {
        self.line_buf.clear();
        let n = self.reader.read_line(&mut self.line_buf)?;
        self.line_number += 1;
        if n == 0 {
            return Ok(false);
        }
        // Strip trailing newline / carriage return
        while self.line_buf.ends_with('\n') || self.line_buf.ends_with('\r') {
            self.line_buf.pop();
        }
        Ok(true)
    }

    /// Read the next FASTQ record into `self.next_sequence`.
    ///
    /// This mirrors `readNext()` in the Java code, including:
    /// - Skipping blank lines between records
    /// - Validating the '@' prefix on the ID line
    /// - Validating the '+' prefix on the mid-line
    /// - Colorspace detection on the first record only
    /// - CASAVA filter detection via `:Y:` in the read ID
    fn read_next(&mut self) -> io::Result<()> {
        // -- ID line (skip blank lines) --
        // The Java code loops reading lines until it finds a non-empty
        // one or hits EOF. Blank lines between records are silently skipped.
        loop {
            if !self.read_line()? {
                // EOF
                self.next_sequence = None;
                return Ok(());
            }
            if !self.line_buf.is_empty() {
                break;
            }
        }

        if !self.line_buf.starts_with('@') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "ID line didn't start with '@' at line {}",
                    self.line_number
                ),
            ));
        }
        // Clone the ID string and clear line_buf, preserving its heap allocation
        // for reuse on subsequent read_line calls. Using std::mem::take here would
        // leave line_buf with zero capacity, forcing a new allocation every line --
        // 3 wasted allocations per record across millions of reads.
        let id = self.line_buf.clone();
        self.line_buf.clear();

        // -- Sequence line --
        if !self.read_line()? {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Ran out of data in the middle of a fastq entry. Your file is probably truncated",
            ));
        }
        let seq_bytes = self.line_buf.as_bytes().to_vec();
        self.line_buf.clear();

        // -- Mid-line ('+' line) --
        if !self.read_line()? {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Ran out of data in the middle of a fastq entry. Your file is probably truncated",
            ));
        }
        if !self.line_buf.starts_with('+') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Midline '{}' didn't start with '+' at {}",
                    self.line_buf, self.line_number
                ),
            ));
        }
        // Mid-line is not needed; just clear the buffer (keeping its allocation)
        self.line_buf.clear();

        // -- Quality line --
        if !self.read_line()? {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Ran out of data in the middle of a fastq entry. Your file is probably truncated",
            ));
        }
        let quality_bytes = self.line_buf.as_bytes().to_vec();
        self.line_buf.clear();

        // Track lowest quality character for Phred encoding detection.
        for &b in &quality_bytes {
            if b < self.lowest_char {
                self.lowest_char = b;
            }
        }

        // -- Colorspace detection (first record only) --
        // Java checks only the very first sequence for colorspace and
        // then assumes the rest of the file is the same. The check is that
        // `nextSequence` is null (i.e. no prior record) and `seq` is non-null.
        if !self.colorspace_checked {
            self.colorspace_checked = true;
            // Safety: seq_bytes originated from a valid UTF-8 String
            let seq_str = std::str::from_utf8(&seq_bytes).unwrap_or("");
            self.is_colorspace = check_colorspace(seq_str);
        }

        // -- CASAVA filtering --
        // If running in --casava mode without --nofilter, check the
        // ID for `:Y:` anywhere after position 0 and flag the sequence as filtered.
        let is_filtered = self.casava_mode && !self.nofilter
            && id.find(":Y:").is_some_and(|pos| pos > 0);

        // Build the Sequence
        let mut sequence = if self.is_colorspace {
            // For colorspace, `seq.toUpperCase()` is passed to both
            // `convertColorspaceToBases` and stored as `colorspaceSequence`.
            // Safety: seq_bytes originated from a valid UTF-8 String
            let seq_str = String::from_utf8(seq_bytes).unwrap_or_default();
            let upper = seq_str.to_ascii_uppercase();
            let bases = convert_colorspace_to_bases(&upper);
            let mut s = Sequence::new(id, bases.into_bytes(), quality_bytes);
            s.colorspace = Some(upper.into_bytes());
            s
        } else {
            // Normal path - Java calls `new Sequence(this, seq.toUpperCase(), quality, id)`.
            // The `Sequence::new` constructor already uppercases, matching Java.
            Sequence::new(id, seq_bytes, quality_bytes)
        };

        sequence.is_filtered = is_filtered;
        self.next_sequence = Some(sequence);

        Ok(())
    }
}

impl SequenceFile for FastQFile {
    fn next(&mut self) -> Option<io::Result<Sequence>> {
        // Java's `next()` returns the current `nextSequence` then calls
        // `readNext()` to prime the next one. We do the same.
        let current = self.next_sequence.take()?;
        if let Err(e) = self.read_next() {
            // Store nothing for next time; the error is returned on the *next* call
            // would be confusing. Instead, return the error now and let the current
            // sequence be lost (matching Java which throws from next()).
            // Actually, Java's next() calls readNext() but returns the previous value.
            // If readNext() throws, the exception propagates out of next().
            // We replicate: return the error, dropping `current`.
            return Some(Err(e));
        }
        Some(Ok(current))
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_colorspace(&self) -> bool {
        self.is_colorspace
    }

    /// Java reads `fis.getChannel().position()` which gives the
    /// *compressed* byte position, then divides by file size (also compressed).
    /// For plain files this is exact; for compressed files it gives a rough
    /// estimate based on compressed bytes consumed.
    ///
    /// For stdin, Java always returns 0 until EOF then 100. We replicate that.
    fn percent_complete(&self) -> f64 {
        if self.next_sequence.is_none() {
            return 100.0;
        }
        if self.name.starts_with("stdin") {
            return 0.0;
        }
        // Java queries fis.getChannel().position() on the raw FileInputStream
        // to get the compressed byte position, then divides by fileSize.
        // We do the same via a cloned file handle using seek(Current).
        if let Some(ref handle) = self.position_handle {
            // try_clone to get a mutable handle without requiring &mut self
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
// Colorspace helpers
// ---------------------------------------------------------------------------

/// Check whether a sequence string is colorspace (SOLiD) format.
///
/// Uses the exact same regex `^[GATCNgatcn][\.0123456]+$` as Java.
/// We implement it manually instead of pulling in a regex crate.
fn check_colorspace(seq: &str) -> bool {
    let bytes = seq.as_bytes();
    if bytes.len() < 2 {
        return false;
    }
    // First character must be a DNA base
    if !matches!(
        bytes[0],
        b'G' | b'A' | b'T' | b'C' | b'N' | b'g' | b'a' | b't' | b'c' | b'n'
    ) {
        return false;
    }
    // Remaining characters must be '.', '0'-'6'
    for &b in &bytes[1..] {
        if !matches!(b, b'.' | b'0'..=b'6') {
            return false;
        }
    }
    true
}

/// Convert a colorspace sequence to base-space.
///
/// This is a direct translation of `convertColorspaceToBases()` from
/// FastQFile.java, preserving the exact same lookup table and the behavior where
/// encountering '.', '4', '5', or '6' causes all remaining positions to become 'N'.
fn convert_colorspace_to_bases(s: &str) -> String {
    let cs: Vec<u8> = s.as_bytes().to_vec();

    // Java returns "" for zero-length input.
    if cs.is_empty() {
        return String::new();
    }

    // Output is one shorter than input (the leading reference base is consumed).
    let mut bp = vec![0u8; cs.len() - 1];

    for i in 1..cs.len() {
        let ref_base = if i == 1 {
            // First iteration uses cs[0] (the leading reference base).
            cs[0]
        } else {
            // Subsequent iterations use the *previous output* base.
            bp[i - 2]
        };

        // If refBase is not a valid DNA letter, Java throws
        // IllegalArgumentException. We replicate with a panic for now, but
        // callers should ensure valid input.
        debug_assert!(
            matches!(ref_base, b'G' | b'A' | b'T' | b'C'),
            "Colorspace sequence data should always start with a real DNA letter, got '{}'",
            ref_base as char,
        );

        // The colorspace-to-base lookup table. Each color digit
        // encodes a transition from the reference base:
        //   0 = same base, 1 = transversion1, 2 = transition, 3 = transversion2
        //   '.', '4', '5', '6' = unknown -> fill rest with N
        bp[i - 1] = match cs[i] {
            b'0' => ref_base, // same base
            b'1' => match ref_base {
                b'A' => b'C',
                b'C' => b'A',
                b'G' => b'T',
                b'T' => b'G',
                _ => b'N',
            },
            b'2' => match ref_base {
                b'A' => b'G',
                b'G' => b'A',
                b'C' => b'T',
                b'T' => b'C',
                _ => b'N',
            },
            b'3' => match ref_base {
                b'A' => b'T',
                b'T' => b'A',
                b'G' => b'C',
                b'C' => b'G',
                _ => b'N',
            },
            // '.', '4', '5', '6' cause all *remaining* positions
            // (including the current one) to be set to 'N'. Java does this with
            // a for-loop from the current `i` to end.
            b'.' | b'4' | b'5' | b'6' => {
                for b in &mut bp[(i - 1)..] {
                    *b = b'N';
                }
                break;
            }
            other => {
                // Java throws IllegalArgumentException for unexpected chars.
                panic!("Unexpected colorspace char '{}'", other as char);
            }
        };
    }

    // Safety: bp contains only ASCII DNA letters or 'N'
    String::from_utf8(bp).expect("colorspace output should be valid UTF-8")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Colorspace helpers ----

    #[test]
    fn test_check_colorspace_positive() {
        assert!(check_colorspace("G0123456"));
        assert!(check_colorspace("A.012"));
        assert!(check_colorspace("t00"));
    }

    #[test]
    fn test_check_colorspace_negative() {
        assert!(!check_colorspace("ACGTACGT"));
        assert!(!check_colorspace("A")); // too short
        assert!(!check_colorspace(""));
        assert!(!check_colorspace("X012")); // invalid lead
    }

    #[test]
    fn test_convert_colorspace_basic() {
        // A0 -> same as A = A
        assert_eq!(convert_colorspace_to_bases("A0"), "A");
        // A1 -> A->C
        assert_eq!(convert_colorspace_to_bases("A1"), "C");
        // A2 -> A->G
        assert_eq!(convert_colorspace_to_bases("A2"), "G");
        // A3 -> A->T
        assert_eq!(convert_colorspace_to_bases("A3"), "T");
    }

    #[test]
    fn test_convert_colorspace_chained() {
        // A00 -> A,A (ref=A->A, then ref=A->A)
        assert_eq!(convert_colorspace_to_bases("A00"), "AA");
        // A01 -> A, C (ref=A->A, then ref=A->C)
        assert_eq!(convert_colorspace_to_bases("A01"), "AC");
        // G10 -> T, T (ref=G->T, then ref=T->T)
        assert_eq!(convert_colorspace_to_bases("G10"), "TT");
    }

    #[test]
    fn test_convert_colorspace_unknown_fills_n() {
        // '.' causes rest to be N
        assert_eq!(convert_colorspace_to_bases("A.12"), "NNN");
        // '4' also fills rest with N
        assert_eq!(convert_colorspace_to_bases("A04"), "AN");
    }

    #[test]
    fn test_convert_colorspace_empty() {
        assert_eq!(convert_colorspace_to_bases(""), "");
    }

    // ---- FastQFile reading ----

    #[test]
    fn test_read_minimal_fastq() {
        let config = FastQCConfig::default();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/minimal.fastq"
        );
        let mut reader = FastQFile::open(&config, path).unwrap();

        // Should have exactly one record
        let seq = reader.next().unwrap().unwrap();
        assert_eq!(seq.id, "@READ0001");
        assert_eq!(seq.sequence, b"AAAAAAAAAAAAAAAA");
        assert_eq!(seq.quality, b"IIIIIIIIIIIIIIII");
        assert!(!seq.is_filtered);
        assert!(!reader.is_colorspace());

        // No more records
        assert!(reader.next().is_none());
    }

    #[test]
    fn test_read_complex_fastq() {
        let config = FastQCConfig::default();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/complex.fastq"
        );
        let mut reader = FastQFile::open(&config, path).unwrap();

        let mut count = 0;
        while let Some(result) = reader.next() {
            let seq = result.unwrap();
            count += 1;
            // All reads in complex.fastq have the same sequence and quality
            assert_eq!(seq.sequence, b"ACGTACGTACGTACGT");
            assert_eq!(seq.quality, b"IIIIIIIIIIIIIIII");
            // IDs are @READ0001 through @READ0005
            assert_eq!(seq.id, format!("@READ{:04}", count));
        }
        assert_eq!(count, 5);
    }

    #[test]
    fn test_lowest_char_tracking() {
        let config = FastQCConfig::default();
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/minimal.fastq"
        );
        let mut reader = FastQFile::open(&config, path).unwrap();

        // Consume all records
        while reader.next().is_some() {}

        // 'I' is ASCII 73
        assert_eq!(reader.lowest_char, b'I');
    }

    #[test]
    fn test_casava_filter_detection() {
        // We can't easily create a temp file in a unit test without extra deps,
        // so we test the CASAVA logic by constructing a reader over a known file.
        // The test files don't have :Y: in the ID, so nothing should be filtered.
        let config = FastQCConfig {
            casava: true,
            nofilter: false,
            ..FastQCConfig::default()
        };
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/data/minimal.fastq"
        );
        let mut reader = FastQFile::open(&config, path).unwrap();
        let seq = reader.next().unwrap().unwrap();
        // "@READ0001" has no ":Y:", so not filtered
        assert!(!seq.is_filtered);
    }

    #[test]
    fn test_sequence_uppercase() {
        // Java uppercases the sequence. Our Sequence::new does the same.
        let seq = Sequence::new(
            "@test".to_string(),
            b"acgtACGT".to_vec(),
            b"IIIIIIII".to_vec(),
        );
        assert_eq!(seq.sequence, b"ACGTACGT");
    }
}
