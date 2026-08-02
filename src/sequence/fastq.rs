// FASTQ file reader
// Corresponds to Sequence/FastQFile.java

use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, Stdin};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use bzip2_rs::DecoderReader;

// NOTE: rapidgzip's reader is referred to by its full path
// (`rapidgzip_core::DecoderReader`) below because the name `DecoderReader` is
// already taken by the bzip2 reader imported above.

use super::{Sequence, SequenceFile};
use crate::config::FastQCConfig;

// ---------------------------------------------------------------------------
// Decompression layer
// ---------------------------------------------------------------------------

/// Wrapper enum so we can store different reader types without trait objects.
/// Each variant wraps a `BufReader` around the appropriate decompression stream.
enum ReaderKind {
    Plain(BufReader<File>),
    /// Parallel, multi-member gzip decompression via rapidgzip. The heavy
    /// lifting (inflate on a pool of background threads) happens behind a
    /// `Read + Send` handle, so it looks like any other buffered reader here.
    Gzip(BufReader<rapidgzip_core::DecoderReader>),
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

/// How a reader estimates its `percent_complete`. The three reader families
/// track the compressed position differently; making it an enum keeps the modes
/// mutually exclusive (previously two `Option` fields that must never both be
/// `Some`) and drives the `percent_complete` match directly.
enum Progress {
    /// stdin has no seekable position; report 0% until EOF (matching Java).
    Stdin,
    /// bzip2 / plain text read on this thread: seek a cloned file handle for the
    /// compressed byte position, exactly as Java queries
    /// `fis.getChannel().position()`.
    FilePosition(File),
    /// gzip via rapidgzip. See [`GzipProgress`].
    Gzip(GzipProgress),
}

/// Largest compression ratio a gzip trailer may imply before it is treated as
/// untrustworthy. FASTQ compresses around 3-5x; anything past this is a
/// truncated `ISIZE` rather than a real ratio.
const MAX_PLAUSIBLE_RATIO: u64 = 1000;

/// gzip records the uncompressed size modulo this, so a file larger than it
/// wraps and the trailer has to be unwrapped against what has been read.
const ISIZE_MODULUS: u64 = 1 << 32;

/// Progress tracking for the parallel gzip decoder.
///
/// The obvious measure — compressed bytes served to the decoder over the file
/// size — is badly wrong here, because rapidgzip's workers read *far* ahead of
/// what the parser has consumed. Measured on four cores, that estimate opens a
/// 99 MB `.gz` at 22%, reaches 100% at the halfway mark and sits there for the
/// rest of the run; a 16 MB `.gz` is pinned at 100% from the very first update.
/// The read-ahead is roughly a fixed window, so the smaller the file the more
/// useless the number.
///
/// So progress is measured on the *output* side instead: `consumed_bytes`, the
/// decompressed bytes the decoder has actually handed to the parser, which by
/// construction never runs ahead of the analysis. What that needs is a total to
/// divide by, and there are two ways to get one.
///
/// **The gzip trailer**, when it is trustworthy, is exact and free — see
/// [`gzip_trailer_size`]. Every `gzip`/`pigz`-compressed `.fastq.gz` is a
/// single member and lands here.
///
/// **Otherwise** — concatenated members, BGZF — fall back to scaling the file
/// size by the ratio the decoder has achieved so far. That ratio is an
/// underestimate while the read-ahead is outstanding (the compressed tally is
/// ahead of the decompressed one) and converges up to the true ratio by EOF, so
/// the running maximum is taken and the bar can run ahead and stall near the
/// end. That is the same weakness the old whole-file estimate had, but confined
/// to inputs whose true size genuinely cannot be known up front.
struct GzipProgress {
    /// Compressed bytes served to the decoder, counted by [`CountingReadAt`].
    compressed: Arc<AtomicU64>,
    /// Lock-free telemetry from the decoder: decompressed bytes produced, and
    /// decompressed bytes actually returned through `Read`.
    handle: rapidgzip_core::DecoderHandle,
    /// Total decompressed size from the gzip trailer, if it looked plausible.
    trailer_total: Option<u64>,
    /// Contended once per [`PROGRESS_INTERVAL`](crate::runner) records at most.
    estimate: std::sync::Mutex<GzipEstimate>,
}

#[derive(Default)]
struct GzipEstimate {
    /// Best estimate of the file's total decompressed size.
    total: u64,
    /// Highest permille reported so far. A bar that goes backwards looks
    /// broken, and the fallback estimate does move as it settles.
    high_water: u64,
}

impl GzipProgress {
    /// Progress through the file as a percentage, or `None` before the decoder
    /// has produced enough to estimate a total.
    fn percent(&self, file_size: u64) -> Option<f64> {
        if file_size == 0 {
            return None;
        }
        let stats = self.handle.stats();
        let compressed = self.compressed.load(Ordering::Relaxed);
        if compressed == 0 || stats.decompressed_bytes == 0 {
            return None;
        }

        let mut estimate = self
            .estimate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let total = match self.trailer_total {
            // The trailer counts modulo 4 GiB. Reading past it means the file
            // is larger than that, and the true size is the recorded one plus
            // however many whole wraps have been consumed -- still exact.
            Some(trailer) => {
                let wraps = stats.consumed_bytes.saturating_sub(1) / ISIZE_MODULUS;
                trailer + wraps * ISIZE_MODULUS
            }
            // Underestimates while the read-ahead is outstanding and converges
            // up to the truth, so keep the largest seen.
            None => {
                let ratio = stats.decompressed_bytes as f64 / compressed as f64;
                estimate.total.max((file_size as f64 * ratio) as u64)
            }
        };
        estimate.total = total;
        if total == 0 {
            return None;
        }

        let permille =
            ((stats.consumed_bytes as f64 / total as f64) * 1000.0).clamp(0.0, 1000.0) as u64;
        estimate.high_water = estimate.high_water.max(permille);
        Some(estimate.high_water as f64 / 10.0)
    }
}

/// The uncompressed size recorded in a gzip file's `ISIZE` trailer (the last
/// four bytes, little-endian), when it is worth believing.
///
/// Exact for a single-member file, which is what `gzip` and `pigz` produce and
/// so what almost every `.fastq.gz` is. Sizes over 4 GiB wrap, which the caller
/// unwraps against the bytes it has read.
///
/// For concatenated members the trailer describes only the *last* one, so it
/// has to be rejected: requiring the implied ratio to be at least 1:1 catches
/// that (a trailing member is a small fraction of the file, and BGZF's empty
/// end-of-file member records zero), as well as the pathological case of a
/// stored-not-deflated stream. [`MAX_PLAUSIBLE_RATIO`] catches a truncated
/// file whose last four bytes are not a trailer at all.
fn gzip_trailer_size(path: &Path, file_size: u64) -> Option<u64> {
    // Smaller than the smallest possible member: header, empty deflate block,
    // trailer.
    if file_size < 20 {
        return None;
    }
    let mut file = File::open(path).ok()?;
    file.seek(io::SeekFrom::End(-4)).ok()?;
    let mut trailer = [0u8; 4];
    file.read_exact(&mut trailer).ok()?;
    let total = u32::from_le_bytes(trailer) as u64;
    (total >= file_size && total <= file_size.saturating_mul(MAX_PLAUSIBLE_RATIO)).then_some(total)
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
// gzip decompression (parallel & multi-member, via rapidgzip)
// ---------------------------------------------------------------------------

/// A [`rapidgzip_core::ReadAt`] source that counts the compressed bytes served,
/// so the FASTQ reader can report progress while rapidgzip owns the file.
struct CountingReadAt {
    file: File,
    counter: Arc<AtomicU64>,
}

impl rapidgzip_core::ReadAt for CountingReadAt {
    fn len(&self) -> io::Result<u64> {
        self.file.metadata().map(|metadata| metadata.len())
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let read = rapidgzip_core::ReadAt::read_at(&self.file, offset, buffer)?;
        self.counter.fetch_add(read as u64, Ordering::Relaxed);
        Ok(read)
    }
}

/// Open a gzip file through the parallel rapidgzip decoder.
///
/// The returned [`GzipProgress`] pairs the compressed-byte counter with the
/// decoder's telemetry handle; see that type for how the two combine into a
/// progress estimate. The decoder streams with backpressure, so peak memory is
/// bounded by the in-flight-chunk budget regardless of input size.
fn open_rapidgzip(
    file: File,
    threads: usize,
    trailer_total: Option<u64>,
) -> io::Result<(rapidgzip_core::DecoderReader, GzipProgress)> {
    let compressed = Arc::new(AtomicU64::new(0));
    let source = CountingReadAt {
        file,
        counter: Arc::clone(&compressed),
    };
    let decoder = rapidgzip_core::Decoder::builder()
        .decoder_threads(threads.max(1))
        .build()?;
    let reader = decoder
        .reader(source)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let progress = GzipProgress {
        compressed,
        handle: reader.handle(),
        trailer_total,
        estimate: std::sync::Mutex::new(GzipEstimate::default()),
    };
    Ok((reader, progress))
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
    /// How this reader reports progress. See [`Progress`] and `percent_complete`.
    progress: Progress,

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
        let (reader, progress) = if is_stdin {
            (
                ReaderKind::Stdin(BufReader::new(io::stdin())),
                Progress::Stdin,
            )
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

            match compression {
                // .gz is decompressed in parallel by rapidgzip; progress is
                // tracked by the compressed-byte counter it returns.
                "gz" => {
                    // `runner::run` normalises the "auto" (0) budget to a positive
                    // value before we get here; `.max(1)` floors any direct caller
                    // (e.g. a unit test) that leaves it at 0.
                    let threads = config.decompress_threads.max(1);
                    let trailer = gzip_trailer_size(path, file_size);
                    let (reader, progress) = open_rapidgzip(file, threads, trailer)?;
                    (
                        ReaderKind::Gzip(BufReader::new(reader)),
                        Progress::Gzip(progress),
                    )
                }
                // bzip2 and plain text are read on this thread; progress uses a
                // cloned handle to query the compressed file position.
                "bz2" => {
                    let pos_handle = file.try_clone()?;
                    (
                        ReaderKind::Bzip2(Box::new(BufReader::new(DecoderReader::new(file)))),
                        Progress::FilePosition(pos_handle),
                    )
                }
                _ => {
                    let pos_handle = file.try_clone()?;
                    (
                        ReaderKind::Plain(BufReader::new(file)),
                        Progress::FilePosition(pos_handle),
                    )
                }
            }
        };

        let casava_mode = config.casava;
        let nofilter = config.nofilter;

        let mut fq = FastQFile {
            reader,
            name,
            file_size,
            progress,
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
                format!("ID line didn't start with '@' at line {}", self.line_number),
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
        let is_filtered =
            self.casava_mode && !self.nofilter && id.find(":Y:").is_some_and(|pos| pos > 0);

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
        match &self.progress {
            // stdin: Java returns 0 until EOF (handled above), then 100.
            Progress::Stdin => 0.0,
            // gzip: the rapidgzip decoder owns the file and reads it positionally
            // on background threads, so there is no single cursor to seek, and
            // its workers read far ahead of the parser. See [`GzipProgress`].
            Progress::Gzip(progress) => progress.percent(self.file_size).unwrap_or(0.0),
            // Java queries fis.getChannel().position() on the raw FileInputStream
            // to get the compressed byte position, then divides by fileSize. We
            // do the same via a cloned handle (seek(Current)) so we need only
            // `&self`; a clone or seek failure degrades to 0%.
            Progress::FilePosition(handle) => handle
                .try_clone()
                .and_then(|mut h| h.stream_position())
                .map(|pos| (pos as f64 / self.file_size as f64) * 100.0)
                .unwrap_or(0.0),
        }
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/minimal.fastq");
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/complex.fastq");
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/minimal.fastq");
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
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/minimal.fastq");
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

    // ---- gzip decoding (rapidgzip) ----

    /// Decoding a `.gz` file must produce byte-identical records to reading its
    /// plaintext twin. Opening a `.gz` goes through the parallel rapidgzip
    /// decoder; the plaintext side involves no decompression at all.
    #[test]
    fn test_gzip_matches_plaintext() {
        let config = FastQCConfig::default();
        let base = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/");

        let mut plain = FastQFile::open(&config, format!("{base}minimal.fastq")).unwrap();
        let mut gz = FastQFile::open(&config, format!("{base}minimal.fastq.gz")).unwrap();

        // Opening the .gz file must engage the gzip path (compressed-byte
        // progress tracking is used only there).
        assert!(
            matches!(gz.progress, Progress::Gzip(_)),
            "expected the gzip reader for .gz input"
        );

        loop {
            match (plain.next(), gz.next()) {
                (Some(a), Some(b)) => {
                    let a = a.unwrap();
                    let b = b.unwrap();
                    assert_eq!(a.id, b.id);
                    assert_eq!(a.sequence, b.sequence);
                    assert_eq!(a.quality, b.quality);
                }
                (None, None) => break,
                _ => panic!("record count mismatch between plaintext and gzip"),
            }
        }
    }

    /// The trailer is the exact uncompressed size for the single-member files
    /// `gzip` and `pigz` produce, and is rejected when it cannot be one.
    #[test]
    fn test_gzip_trailer_size() {
        let base = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/");

        // A real single-member fixture. The trailer must be the decompressed
        // size, which is checked here against the bytes the reader actually
        // produces rather than a constant, so the two cannot drift apart if
        // the fixture is ever regenerated.
        let gz = Path::new(base).join("realistic.fastq.gz");
        let gz_len = std::fs::metadata(&gz).unwrap().len();
        let trailer = gzip_trailer_size(&gz, gz_len).expect("a plausible trailer");

        let mut decoded = 0u64;
        let (mut reader, _) = open_rapidgzip(File::open(&gz).unwrap(), 1, None).unwrap();
        let mut buffer = [0u8; 8192];
        loop {
            match reader.read(&mut buffer).unwrap() {
                0 => break,
                n => decoded += n as u64,
            }
        }
        assert_eq!(trailer, decoded, "trailer is not the uncompressed size");

        // Too small to hold a member at all.
        assert_eq!(gzip_trailer_size(&gz, 19), None);
        // A trailer implying that the file barely compressed, or expanded, is
        // the last member of a concatenated stream rather than the whole size.
        assert_eq!(gzip_trailer_size(&gz, trailer + 1), None);
        // ...and one implying an absurd ratio is not a trailer at all.
        assert_eq!(gzip_trailer_size(&gz, 1), None);
    }

    /// Progress must come from the bytes handed to the parser, not from the
    /// compressed bytes the decoder's workers have raced ahead to read, and it
    /// must never go backwards.
    #[test]
    fn test_gzip_progress_tracks_consumed_bytes() {
        let config = FastQCConfig::default();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/realistic.fastq.gz");
        let mut reader = FastQFile::open(&config, path).unwrap();

        let mut last = 0.0f64;
        let mut seen_partial = false;
        while let Some(result) = reader.next() {
            result.unwrap();
            let percent = reader.percent_complete();
            assert!(
                (0.0..=100.0).contains(&percent),
                "percent out of range: {percent}"
            );
            assert!(
                percent >= last,
                "progress went backwards: {last} -> {percent}"
            );
            // The whole point: a small file must not be pinned at 100% from the
            // first record just because its bytes have all been read.
            if percent < 100.0 {
                seen_partial = true;
            }
            last = percent;
        }
        assert!(
            seen_partial,
            "progress was saturated for the whole file, which is the bug this guards"
        );
        assert_eq!(reader.percent_complete(), 100.0, "did not finish at 100%");
    }

    /// The gzip reader must decode a real (dynamic-Huffman) gzip stream
    /// correctly: the realistic fixture has 1009 records.
    #[test]
    fn test_gzip_reads_realistic() {
        let config = FastQCConfig::default();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/realistic.fastq.gz");
        let mut reader = FastQFile::open(&config, path).unwrap();
        assert!(matches!(reader.progress, Progress::Gzip(_)));

        let mut count = 0u64;
        while let Some(result) = reader.next() {
            let seq = result.unwrap();
            assert!(!seq.is_empty());
            count += 1;
        }
        assert_eq!(count, 1009);
    }
}
