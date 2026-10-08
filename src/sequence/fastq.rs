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
    Gzip(Box<BufReader<rapidgzip_core::DecoderReader>>),
    Bzip2(Box<BufReader<DecoderReader<File>>>),
    Stdin(BufReader<Stdin>),
}

/// Large so gzip refills (a decoder-thread handoff) are rare.
const READ_BUFFER: usize = 1 << 20;

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
/// track the compressed position differently, so each mode carries what it
/// needs and no more.
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
/// wraps and the trailer has to be unwrapped.
const ISIZE_MODULUS: u64 = 1 << 32;

/// Progress tracking for the gzip decoder.
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
/// single member and lands here. Past 4 GiB the trailer has wrapped, and the
/// ratio estimate below picks which wrap it is — see [`unwrap_isize`].
///
/// **Otherwise** — concatenated members, BGZF — fall back to scaling the file
/// size by the ratio the decoder has achieved so far. That ratio is an
/// underestimate while the read-ahead is outstanding (the compressed tally is
/// ahead of the decompressed one) and converges up to the true ratio by EOF, so
/// the running maximum is taken and the bar can run ahead and stall near the
/// end.
///
/// The trailer only describes the *last* member, and a concatenated file
/// (`cat L001.gz L002.gz`) looks single-member until the decoder finishes the
/// first one. The ratio estimate catches that early: it only ever errs low, so
/// once it clearly exceeds the trailer the trailer is dropped for good.
struct GzipProgress {
    /// Furthest compressed offset the decoder has read, from [`CountingSource`].
    compressed: Arc<AtomicU64>,
    /// Lock-free telemetry from the decoder: decompressed bytes produced, and
    /// decompressed bytes actually returned through `Read`.
    handle: rapidgzip_core::DecoderHandle,
    /// Contended once per [`PROGRESS_INTERVAL`](crate::runner) records at most.
    estimate: std::sync::Mutex<GzipEstimate>,
}

#[derive(Default)]
struct GzipEstimate {
    /// Running maximum of the ratio-scaled size estimate.
    ratio_total: u64,
    /// Total decompressed size from the gzip trailer while it is still
    /// believed; taken once the file turns out to have several members.
    trailer: Option<u64>,
}

impl GzipProgress {
    /// Progress through the file as a percentage, or `None` before the decoder
    /// has produced enough to estimate a total. `buffered` is decompressed
    /// output already read but not yet parsed.
    fn percent(&self, file_size: u64, buffered: u64) -> Option<f64> {
        if file_size == 0 {
            return None;
        }
        let stats = self.handle.stats();
        let parsed = stats.consumed_bytes.saturating_sub(buffered);
        let compressed = self.compressed.load(Ordering::Relaxed);
        if compressed == 0 || stats.decompressed_bytes == 0 {
            return None;
        }

        let mut estimate = self
            .estimate
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());

        let ratio = stats.decompressed_bytes as f64 / compressed as f64;
        estimate.ratio_total = estimate
            .ratio_total
            .max((file_size as f64 * ratio) as u64)
            .max(stats.decompressed_bytes);

        // Output is emitted in member order, so once a member has completed,
        // output that no longer matches the trailer must be a later member's.
        if let Some(trailer) = estimate.trailer {
            let past_first_member = match stats.member_count {
                0 => false,
                1 => stats.decompressed_bytes % ISIZE_MODULUS != trailer,
                _ => true,
            };
            // The quarter is headroom for the ratio varying along the file.
            let wrapped = unwrap_isize(trailer, 0, estimate.ratio_total);
            if past_first_member || estimate.ratio_total > wrapped + wrapped / 4 {
                estimate.trailer = None;
            }
        }
        let total = match estimate.trailer {
            Some(trailer) => unwrap_isize(trailer, parsed, estimate.ratio_total),
            None => estimate.ratio_total,
        };
        if total == 0 {
            return None;
        }

        Some((parsed as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
    }
}

/// The true uncompressed size given a trailer recorded modulo 4 GiB: the
/// candidate `trailer + k * 4 GiB` nearest the `hint` (the ratio estimate),
/// and never less than the `parsed` bytes already seen.
///
/// The hint only has to be within 2 GiB of the truth to pick the right wrap,
/// which a ratio measured over the first few hundred MB comfortably is.
fn unwrap_isize(trailer: u64, parsed: u64, hint: u64) -> u64 {
    let nearest = (hint.saturating_sub(trailer) + ISIZE_MODULUS / 2) / ISIZE_MODULUS;
    let at_least = parsed.saturating_sub(trailer).div_ceil(ISIZE_MODULUS);
    trailer + nearest.max(at_least) * ISIZE_MODULUS
}

/// The uncompressed size recorded in a gzip file's `ISIZE` trailer (the last
/// four bytes, little-endian), when it is worth believing.
///
/// Exact for a single-member file, which is what `gzip` and `pigz` produce and
/// so what almost every `.fastq.gz` is. Sizes over 4 GiB wrap, which
/// [`unwrap_isize`] undoes.
///
/// For concatenated members the trailer describes only the *last* one, so it
/// has to be rejected: requiring the implied ratio to be at least 1:1 catches
/// that (a trailing member is a small fraction of the file, and BGZF's empty
/// end-of-file member records zero), as well as the pathological case of a
/// stored-not-deflated stream. A trailing member that is still larger than the
/// file gets past this, and is caught by [`GzipProgress::percent`].
/// [`MAX_PLAUSIBLE_RATIO`] catches a truncated file whose
/// last four bytes are not a trailer at all.
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

/// A gzip source that tracks how far into the file the decoder has read, so the
/// FASTQ reader can report progress while rapidgzip owns the file. Read
/// positionally by the parallel decoder, or sequentially when decoding on the
/// reading thread. The decoder re-reads some ranges, so a sum of bytes served
/// would overcount.
struct CountingSource {
    file: File,
    counter: Arc<AtomicU64>,
}

impl rapidgzip_core::ReadAt for CountingSource {
    fn len(&self) -> io::Result<u64> {
        self.file.metadata().map(|metadata| metadata.len())
    }

    fn read_at(&self, offset: u64, buffer: &mut [u8]) -> io::Result<usize> {
        let read = rapidgzip_core::ReadAt::read_at(&self.file, offset, buffer)?;
        self.counter
            .fetch_max(offset + read as u64, Ordering::Relaxed);
        Ok(read)
    }
}

impl Read for CountingSource {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.file.read(buffer)?;
        self.counter.fetch_add(read as u64, Ordering::Relaxed);
        Ok(read)
    }
}

/// A gzip file opened through rapidgzip.
struct OpenedGzip {
    reader: rapidgzip_core::DecoderReader,
    progress: GzipProgress,
    /// Threads decoding in the background, beside the one reading.
    background_threads: usize,
}

/// Open a gzip file through rapidgzip.
///
/// `threads` decoders run in the background on a regular file. With
/// `threads == 0`, or a FIFO or other non-regular file that the positional
/// decoder cannot read, the stream is decoded on the reading thread instead,
/// inside its `read` calls.
///
/// See [`GzipProgress`] for how the returned telemetry becomes a progress
/// estimate. The decoder streams with backpressure, so peak memory is bounded
/// by the in-flight-chunk budget regardless of input size.
fn open_rapidgzip(
    file: File,
    threads: usize,
    trailer_total: Option<u64>,
) -> io::Result<OpenedGzip> {
    let positional = threads > 0
        && file
            .metadata()
            .is_ok_and(|metadata| metadata.file_type().is_file());
    let compressed = Arc::new(AtomicU64::new(0));
    let source = CountingSource {
        file,
        counter: Arc::clone(&compressed),
    };
    let decoder = rapidgzip_core::Decoder::builder()
        .decoder_threads(threads.max(1))
        .build()?;
    let reader = if positional {
        decoder.reader(source)
    } else {
        decoder.stream_reader(source)
    }
    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let progress = GzipProgress {
        compressed,
        handle: reader.handle(),
        estimate: std::sync::Mutex::new(GzipEstimate {
            ratio_total: 0,
            trailer: trailer_total,
        }),
    };
    Ok(OpenedGzip {
        reader,
        progress,
        background_threads: if positional { threads } else { 0 },
    })
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
    /// Decoder threads working in the background for this file.
    background_threads: usize,

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
        let (reader, progress, background_threads) = if is_stdin {
            (
                ReaderKind::Stdin(BufReader::new(io::stdin())),
                Progress::Stdin,
                0,
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
                "gz" => {
                    let trailer = gzip_trailer_size(path, file_size);
                    let gz = open_rapidgzip(file, config.decompress_threads, trailer)?;
                    (
                        ReaderKind::Gzip(Box::new(BufReader::with_capacity(
                            READ_BUFFER,
                            gz.reader,
                        ))),
                        Progress::Gzip(gz.progress),
                        gz.background_threads,
                    )
                }
                // bzip2 and plain text are read on this thread; progress uses a
                // cloned handle to query the compressed file position.
                "bz2" => {
                    let pos_handle = file.try_clone()?;
                    (
                        ReaderKind::Bzip2(Box::new(BufReader::new(DecoderReader::new(file)))),
                        Progress::FilePosition(pos_handle),
                        0,
                    )
                }
                _ => {
                    let pos_handle = file.try_clone()?;
                    (
                        ReaderKind::Plain(BufReader::with_capacity(READ_BUFFER, file)),
                        Progress::FilePosition(pos_handle),
                        0,
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
            background_threads,
            next_sequence: None,
            line_number: 0,
            is_colorspace: false,
            colorspace_checked: false,
            casava_mode,
            nofilter,
            lowest_char: 255,
        };

        // Prime the look-ahead buffer by reading the first record.
        fq.read_next()?;

        Ok(fq)
    }

    /// Read the next line, without its line ending, into `out` (cleared first),
    /// incrementing `line_number`. Returns `false` at EOF. The line is not
    /// UTF-8 validated; callers do that once, where the bytes are used.
    ///
    /// Not `read_until`: its newline search is measurably slower than the
    /// `memchr` crate's SIMD search on this, the parser's hottest loop.
    fn read_line(&mut self, out: &mut Vec<u8>) -> io::Result<bool> {
        out.clear();
        let mut read_any = false;
        loop {
            let available = match self.reader.fill_buf() {
                Ok(available) => available,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(e),
            };
            if available.is_empty() {
                break;
            }
            read_any = true;
            if let Some(newline) = memchr::memchr(b'\n', available) {
                out.extend_from_slice(&available[..newline]);
                self.reader.consume(newline + 1);
                break;
            }
            let len = available.len();
            out.extend_from_slice(available);
            self.reader.consume(len);
        }
        self.line_number += 1;
        if !read_any {
            return Ok(false);
        }
        while out.last() == Some(&b'\r') {
            out.pop();
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
        let mut id_bytes = Vec::new();
        loop {
            if !self.read_line(&mut id_bytes)? {
                // EOF
                self.next_sequence = None;
                return Ok(());
            }
            if !id_bytes.is_empty() {
                break;
            }
        }

        if !id_bytes.starts_with(b"@") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("ID line didn't start with '@' at line {}", self.line_number),
            ));
        }
        let id = String::from_utf8(id_bytes).map_err(|_| invalid_utf8())?;

        // -- Sequence line --
        let mut seq_bytes = Vec::new();
        if !self.read_line(&mut seq_bytes)? {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Ran out of data in the middle of a fastq entry. Your file is probably truncated",
            ));
        }

        // -- Mid-line ('+' line), read into what will hold the qualities --
        let mut quality_bytes = Vec::with_capacity(seq_bytes.len());
        if !self.read_line(&mut quality_bytes)? {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Ran out of data in the middle of a fastq entry. Your file is probably truncated",
            ));
        }
        check_utf8(&quality_bytes)?;
        if !quality_bytes.starts_with(b"+") {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "Midline '{}' didn't start with '+' at {}",
                    String::from_utf8_lossy(&quality_bytes),
                    self.line_number
                ),
            ));
        }

        // -- Quality line --
        if !self.read_line(&mut quality_bytes)? {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "Ran out of data in the middle of a fastq entry. Your file is probably truncated",
            ));
        }
        check_utf8(&quality_bytes)?;

        // Track lowest quality character for Phred encoding detection.
        if let Some(lowest) = quality_bytes.iter().copied().min() {
            self.lowest_char = self.lowest_char.min(lowest);
        }

        let seq_str = std::str::from_utf8(&seq_bytes).map_err(|_| invalid_utf8())?;

        // -- Colorspace detection (first record only) --
        // Java checks only the very first sequence for colorspace and
        // then assumes the rest of the file is the same. The check is that
        // `nextSequence` is null (i.e. no prior record) and `seq` is non-null.
        if !self.colorspace_checked {
            self.colorspace_checked = true;
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

    fn background_threads(&self) -> usize {
        self.background_threads
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
            Progress::Gzip(progress) => {
                let buffered = match &self.reader {
                    ReaderKind::Gzip(r) => r.buffer().len() as u64,
                    _ => 0,
                };
                progress.percent(self.file_size, buffered).unwrap_or(0.0)
            }
            // Java queries fis.getChannel().position() on the raw FileInputStream
            // to get the compressed byte position, then divides by fileSize. We
            // do the same via a cloned handle (seek(Current)) so we need only
            // `&self`; a clone or seek failure degrades to 0%.
            Progress::FilePosition(handle) => {
                let buffered = match &self.reader {
                    ReaderKind::Plain(r) => r.buffer().len() as u64,
                    _ => 0,
                };
                handle
                    .try_clone()
                    .and_then(|mut h| h.stream_position())
                    .map(|pos| {
                        (pos.saturating_sub(buffered) as f64 / self.file_size as f64) * 100.0
                    })
                    .unwrap_or(0.0)
            }
        }
    }
}

/// The error `BufRead::read_line` gives for a line that is not UTF-8.
fn invalid_utf8() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "stream did not contain valid UTF-8",
    )
}

fn check_utf8(bytes: &[u8]) -> io::Result<()> {
    std::str::from_utf8(bytes)
        .map(|_| ())
        .map_err(|_| invalid_utf8())
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
        let mut reader = open_rapidgzip(File::open(&gz).unwrap(), 1, None)
            .unwrap()
            .reader;
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

    /// A trailer recorded modulo 4 GiB is unwrapped to the candidate nearest
    /// the ratio estimate, and never below what has already been parsed.
    #[test]
    fn test_unwrap_isize() {
        const GIB: u64 = 1 << 30;
        // Under 4 GiB the trailer is the size.
        assert_eq!(unwrap_isize(GIB, 0, GIB), GIB);
        assert_eq!(unwrap_isize(GIB, GIB / 2, 2 * GIB), GIB);
        // A 6 GiB file records 2 GiB; a rough ratio estimate picks the wrap
        // long before the parser reaches 2 GiB.
        assert_eq!(unwrap_isize(2 * GIB, GIB / 10, 5 * GIB), 6 * GIB);
        assert_eq!(unwrap_isize(2 * GIB, GIB / 10, 7 * GIB), 6 * GIB);
        // A poor estimate early is corrected once parsing passes the trailer.
        assert_eq!(unwrap_isize(2 * GIB, 3 * GIB, GIB), 6 * GIB);
        // Several wraps.
        assert_eq!(unwrap_isize(GIB, 0, 13 * GIB), 13 * GIB);
    }

    /// With no background decoder the file is decoded on the reading thread,
    /// to the same records.
    #[test]
    fn test_gzip_inline_decoding() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/realistic.fastq.gz");
        let background = FastQFile::open(&FastQCConfig::default(), path).unwrap();
        assert_eq!(background.background_threads(), 1);

        let config = FastQCConfig {
            decompress_threads: 0,
            ..FastQCConfig::default()
        };
        let mut inline = FastQFile::open(&config, path).unwrap();
        assert_eq!(inline.background_threads(), 0);
        let mut count = 0u64;
        let mut seen_partial = false;
        while let Some(result) = inline.next() {
            result.unwrap();
            count += 1;
            seen_partial |= inline.percent_complete() < 100.0;
        }
        assert_eq!(count, 1009);
        assert!(seen_partial, "progress was saturated for the whole file");
    }

    /// A named pipe has no length and cannot be read positionally, so it is
    /// streamed rather than rejected.
    #[cfg(unix)]
    #[test]
    fn test_gzip_from_a_fifo() {
        let dir = std::env::temp_dir().join(format!("fastqc_fifo_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fifo = dir.join("reads.fastq.gz");
        let _ = std::fs::remove_file(&fifo);
        let status = std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .expect("mkfifo");
        assert!(status.success());

        let source = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/realistic.fastq.gz");
        let writer = {
            let fifo = fifo.clone();
            std::thread::spawn(move || {
                let data = std::fs::read(source).unwrap();
                std::fs::write(&fifo, data).unwrap();
            })
        };

        let mut reader = FastQFile::open(&FastQCConfig::default(), &fifo).unwrap();
        assert_eq!(reader.background_threads(), 0);
        let mut count = 0u64;
        while let Some(result) = reader.next() {
            result.unwrap();
            count += 1;
        }
        writer.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(count, 1009);
    }

    /// Progress must come from the bytes handed to the parser, not from the
    /// compressed bytes the decoder's workers have raced ahead to read. With a
    /// single member the trailer is exact, so the bar must also never go back.
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

    /// A concatenated file's trailer holds only the last member's size; trusting
    /// it reads 100% halfway through and then sits there.
    #[test]
    fn test_gzip_progress_concatenated_members() {
        let source = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/realistic.fastq.gz");
        let member = std::fs::read(source).unwrap();
        let dir = std::env::temp_dir().join(format!("fastqc_concat_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("reads.fastq.gz");
        std::fs::write(&path, [member.as_slice(), member.as_slice()].concat()).unwrap();

        let mut reader = FastQFile::open(&FastQCConfig::default(), &path).unwrap();
        let mut count = 0u64;
        let mut at_halfway = None;
        while let Some(result) = reader.next() {
            result.unwrap();
            count += 1;
            // Polled throughout, as the runner does, so it can spot the extra member.
            let percent = reader.percent_complete();
            if count == 1009 {
                at_halfway = Some(percent);
            }
        }
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(count, 2018);
        let at_halfway = at_halfway.unwrap();
        assert!(
            (35.0..=65.0).contains(&at_halfway),
            "halfway through the reads but progress says {at_halfway}%"
        );
        assert_eq!(reader.percent_complete(), 100.0);
    }

    /// Plain-file progress must not count what the read buffer has fetched
    /// ahead of the parser, or a small file reads 100% from its first record.
    #[test]
    fn test_plain_progress_tracks_parsed_bytes() {
        let config = FastQCConfig::default();
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/varied.fastq");
        let mut reader = FastQFile::open(&config, path).unwrap();
        let mut last = 0.0f64;
        let mut seen_partial = false;
        while let Some(result) = reader.next() {
            result.unwrap();
            let percent = reader.percent_complete();
            assert!(
                percent >= last,
                "progress went backwards: {last} -> {percent}"
            );
            seen_partial |= percent < 100.0;
            last = percent;
        }
        assert!(seen_partial, "progress was saturated for the whole file");
        assert_eq!(reader.percent_complete(), 100.0);
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
