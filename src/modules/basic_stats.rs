// Basic Statistics module
// Corresponds to Modules/BasicStats.java

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::config::Limits;
use crate::modules::QCModule;
use crate::sequence::Sequence;
use crate::utils::base_counts::{count_acgt, IDX_A, IDX_C, IDX_G, IDX_T};
use crate::utils::phred;

/// How often to check whether the display wants a fresh snapshot. Small enough
/// that a slow file (nanopore reads take orders of magnitude longer than
/// Illumina ones) still looks alive, and large enough that the per-read cost
/// rounds to nothing.
const PUBLISH_INTERVAL: u32 = 256;

/// The raw accumulated counters behind the Basic Statistics table.
///
/// Kept separate from [`BasicStats`] so that exactly the same numbers can be
/// formatted for the text/HTML report and for the live terminal table, with
/// [`BasicStatsCounters::rows`] as the single source of truth for both — which
/// is also why the counters themselves are private: everything outside this
/// module wants the formatted rows, not the raw tallies. It is `Copy` so a
/// consistent snapshot can be handed to the progress display without holding a
/// lock while rendering.
#[derive(Debug, Clone, Copy)]
pub struct BasicStatsCounters {
    actual_count: u64,
    min_length: usize,
    max_length: usize,
    total_bases: u64,
    g_count: u64,
    c_count: u64,
    a_count: u64,
    t_count: u64,
    lowest_char: u16,
    // Set by QCModule::set_phred_encoding; see the trait docs.
    known_encoding: Option<phred::PhredEncoding>,
    /// Whether the base calls were converted from colorspace, taken from the
    /// first sequence. `None` until then.
    colorspace: Option<bool>,
    /// Derived from [`BasicStats`]'s length histogram, which is too big to copy
    /// into every snapshot; refreshed whenever the counters are published.
    median_length: usize,
}

/// Spelled out rather than derived: `lowest_char` starts at a sentinel above
/// the range and is lowered, so a derived all-zeroes default would be a state
/// the counters can never legitimately reach — and one whose `rows()` reports
/// a bogus encoding. This is the only constructor, so there is nowhere for that
/// state to come from.
impl Default for BasicStatsCounters {
    fn default() -> Self {
        BasicStatsCounters {
            actual_count: 0,
            min_length: 0,
            max_length: 0,
            total_bases: 0,
            g_count: 0,
            c_count: 0,
            a_count: 0,
            t_count: 0,
            lowest_char: phred::NO_QUALITY_SEEN,
            known_encoding: None,
            colorspace: None,
            median_length: 0,
        }
    }
}

impl BasicStatsCounters {
    /// The measures reported, in report order, minus the leading "Filename"
    /// row. [`Self::rows`] returns a value for each of these, in this order.
    pub const MEASURES: [&'static str; 8] = [
        "File type",
        "Encoding",
        "Total Sequences",
        "Total Bases",
        "Sequence length",
        "Mean Length",
        "Median Length",
        "%GC",
    ];

    /// The Basic Statistics rows, minus the leading "Filename" row, in report
    /// order. Both the text report and the live progress table render from
    /// this, so the values on screen always agree with the values on disk.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        // JAVA COMPAT: Java prints its unset String as "null" for empty input.
        let file_type = match self.colorspace {
            None => "null",
            Some(true) => "Colorspace converted to bases",
            Some(false) => "Conventional base calls",
        };

        // The report propagates a resolve failure before rendering rows (see
        // write_text_report), so "Unknown" only ever reaches the live table.
        let encoding = phred::resolve(self.known_encoding, self.lowest_char)
            .map(|e| e.name.to_string())
            .unwrap_or_else(|_| "Unknown".to_string());

        let sequence_length = if self.min_length == self.max_length {
            self.min_length.to_string()
        } else {
            format!("{}-{}", self.min_length, self.max_length)
        };

        // JAVA COMPAT: integer division
        let mean_length = self.total_bases.checked_div(self.actual_count).unwrap_or(0);

        // JAVA COMPAT: Integer division: ((gCount+cCount)*100)/(aCount+tCount+gCount+cCount)
        let total = self.a_count + self.t_count + self.g_count + self.c_count;
        let gc = ((self.g_count + self.c_count) * 100)
            .checked_div(total)
            .unwrap_or(0);

        let values = [
            file_type.to_string(),
            encoding,
            self.actual_count.to_string(),
            format_length(self.total_bases),
            sequence_length,
            mean_length.to_string(),
            self.median_length.to_string(),
            gc.to_string(),
        ];
        Self::MEASURES.into_iter().zip(values).collect()
    }
}

/// A snapshot of the Basic Statistics counters that a [`BasicStats`] module
/// publishes as it works, so another thread (the progress display) can read
/// partial results while the file is still being analysed.
///
/// `None` until the first publication, which lets the reader distinguish
/// "nothing counted yet" from "genuinely zero".
///
/// Publishing costs a pass over the read-length histogram for the median,
/// which for long reads is far more than the module's per-read work. So the
/// module only publishes when asked: once to start with, and then each time
/// the display has drawn the last snapshot and wants another. A table that is
/// hidden asks for nothing.
pub struct LiveStats {
    snapshot: Mutex<Option<BasicStatsCounters>>,
    wanted: AtomicBool,
}

impl Default for LiveStats {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(None),
            wanted: AtomicBool::new(true),
        }
    }
}

impl LiveStats {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ask the module for a fresh snapshot at its next opportunity.
    pub fn request(&self) {
        self.wanted.store(true, Ordering::Relaxed);
    }

    /// Whether a snapshot has been asked for, clearing the request.
    fn take_request(&self) -> bool {
        self.wanted.load(Ordering::Relaxed) && self.wanted.swap(false, Ordering::Relaxed)
    }

    /// The most recently published counters, or `None` if the module has not
    /// processed any sequences yet.
    pub fn snapshot(&self) -> Option<BasicStatsCounters> {
        *self.snapshot.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn publish(&self, counters: BasicStatsCounters) {
        *self.snapshot.lock().unwrap_or_else(|e| e.into_inner()) = Some(counters);
    }
}

/// Format a base count into a human-readable string.
///
/// Replicates `BasicStats.formatLength(long)` exactly, including
/// its custom decimal truncation logic (keeps at most 1 non-zero decimal digit).
pub fn format_length(original_length: u64) -> String {
    let mut length = original_length as f64;

    let unit = if length >= 1_000_000_000.0 {
        length /= 1_000_000_000.0;
        " Gbp"
    } else if length >= 1_000_000.0 {
        length /= 1_000_000.0;
        " Mbp"
    } else if length >= 1_000.0 {
        length /= 1_000.0;
        " kbp"
    } else {
        " bp"
    };

    // JAVA COMPAT: Java builds `"" + length` which calls Double.toString(),
    // then applies a custom truncation: find the dot, keep one more char if
    // it's non-zero, otherwise drop the dot.
    let raw = format!("{}", length);
    let chars: Vec<char> = raw.chars().collect();

    let mut last_index = 0;

    // Find the dot
    for (i, &ch) in chars.iter().enumerate() {
        last_index = i;
        if ch == '.' {
            break;
        }
    }

    // Keep next char if non-zero
    if last_index + 1 < chars.len() && chars[last_index + 1] != '0' {
        last_index += 1;
    } else if last_index > 0 && chars[last_index] == '.' {
        // Lose the dot if it would be the last character
        last_index -= 1;
    }

    let truncated: String = chars[..=last_index].iter().collect();
    format!("{}{}", truncated, unit)
}

pub struct BasicStats {
    name: Option<String>,
    counters: BasicStatsCounters,
    // Java reads the median from the Sequence Length Distribution module,
    // which skips filtered reads, so this does too.
    length_counts: Vec<u64>,
    /// Optional live snapshot sink for the terminal progress table.
    live: Option<Arc<LiveStats>>,
}

impl BasicStats {
    pub fn new(_limits: &Limits) -> Self {
        BasicStats {
            name: None,
            counters: BasicStatsCounters::default(),
            length_counts: Vec::new(),
            live: None,
        }
    }

    /// Set the filename, stripping any "stdin:" prefix.
    ///
    /// Matches `setFileName()` which strips "stdin:" prefix.
    pub fn set_file_name(&mut self, name: &str) {
        let name = name.strip_prefix("stdin:").unwrap_or(name);
        self.name = Some(name.to_string());
    }

    /// Push the current counters to the live snapshot, if one is attached.
    fn publish(&mut self) {
        if let Some(ref live) = self.live {
            self.counters.median_length = median_length(&self.length_counts);
            live.publish(self.counters);
        }
    }
}

/// Replicates `SequenceLengthDistribution.medianLength()`: the upper of the
/// two central values for an even read count, 0 when there are no reads.
fn median_length(length_counts: &[u64]) -> usize {
    let rank50 = length_counts.iter().sum::<u64>() / 2;
    let mut running = 0;
    for (len, &count) in length_counts.iter().enumerate() {
        running += count;
        if running > rank50 {
            return len;
        }
    }
    0
}

impl QCModule for BasicStats {
    fn cost_hint(&self) -> u32 {
        2
    }

    fn process_sequence(&mut self, sequence: &Sequence) {
        // Publish a snapshot for the live progress table every so often.
        // Taken before this sequence is added so that nothing is published
        // until there is something to show.
        let seen = self.counters.actual_count;
        if seen != 0
            && seen.is_multiple_of(PUBLISH_INTERVAL as u64)
            && self.live.as_ref().is_some_and(|live| live.take_request())
        {
            self.publish();
        }

        let c = &mut self.counters;
        c.actual_count += 1;
        c.total_bases += sequence.sequence.len() as u64;

        let len = sequence.sequence.len();
        if !sequence.is_filtered {
            if self.length_counts.len() <= len {
                self.length_counts.resize(len + 1, 0);
            }
            self.length_counts[len] += 1;
        }
        if c.actual_count == 1 {
            c.colorspace = Some(sequence.colorspace.is_some());
            c.min_length = len;
            c.max_length = len;
        } else {
            c.min_length = c.min_length.min(len);
            c.max_length = c.max_length.max(len);
        }

        let counts = count_acgt(&sequence.sequence);
        c.a_count += counts[IDX_A];
        c.c_count += counts[IDX_C];
        c.g_count += counts[IDX_G];
        c.t_count += counts[IDX_T];

        if let Some(lowest) = sequence.quality.iter().copied().min() {
            c.lowest_char = c.lowest_char.min(lowest as u16);
        }
    }

    fn attach_live_stats(&mut self, live: Arc<LiveStats>) {
        self.live = Some(live);
    }

    /// Publish the final counters so the progress table ends on exactly the
    /// values that go into the report.
    fn finalize(&mut self) {
        if self.counters.known_encoding.is_none() {
            if let Some(warning) = phred::phred64_suspicion(self.counters.lowest_char) {
                // Files run concurrently, so say which one this is about.
                crate::progress::log_line(&match &self.name {
                    Some(name) => format!("{name}: {warning}"),
                    None => warning,
                });
            }
        }
        self.publish();
    }

    fn set_filename(&mut self, name: &str) {
        self.set_file_name(name);
    }

    fn set_phred_encoding(&mut self, encoding: phred::PhredEncoding) {
        self.counters.known_encoding = Some(encoding);
    }

    fn name(&self) -> &str {
        "Basic Statistics"
    }

    fn description(&self) -> &str {
        "Calculates some basic statistics about the file"
    }

    fn reset(&mut self) {
        self.counters.min_length = 0;
        self.counters.max_length = 0;
        self.counters.g_count = 0;
        self.counters.c_count = 0;
        self.counters.a_count = 0;
        self.counters.t_count = 0;
    }

    // BasicStats never raises error or warning
    fn raises_error(&self) -> bool {
        false
    }

    fn raises_warning(&self) -> bool {
        false
    }

    fn ignore_filtered_sequences(&self) -> bool {
        false
    }

    fn ignore_in_report(&self) -> bool {
        false
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        // Java fails the whole file when the encoding can't be resolved.
        phred::resolve(self.counters.known_encoding, self.counters.lowest_char)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

        // Header row matches writeTextTable output from AbstractQCModule
        writeln!(writer, "#Measure\tValue")?;

        // Row 0: Filename
        writeln!(writer, "Filename\t{}", self.name.as_deref().unwrap_or(""))?;

        let counters = BasicStatsCounters {
            median_length: median_length(&self.length_counts),
            ..self.counters
        };
        for (measure, value) in counters.rows() {
            writeln!(writer, "{}\t{}", measure, value)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_length_bp() {
        assert_eq!(format_length(16), "16 bp");
        assert_eq!(format_length(80), "80 bp");
        assert_eq!(format_length(999), "999 bp");
    }

    #[test]
    fn test_format_length_kbp() {
        assert_eq!(format_length(1000), "1 kbp");
        assert_eq!(format_length(1500), "1.5 kbp");
        assert_eq!(format_length(10000), "10 kbp");
    }

    #[test]
    fn test_format_length_mbp() {
        assert_eq!(format_length(1_000_000), "1 Mbp");
        assert_eq!(format_length(1_200_000), "1.2 Mbp");
    }

    #[test]
    fn test_format_length_gbp() {
        assert_eq!(format_length(1_000_000_000), "1 Gbp");
    }

    fn sequences(count: usize) -> Vec<Sequence> {
        (0..count)
            .map(|i| {
                // Vary the length so min != max, and the GC content with it.
                let len = 40 + (i % 7);
                let bases: Vec<u8> = (0..len).map(|p| b"ACGTGGCN"[(i * 3 + p) % 8]).collect();
                let quality = vec![b'I'; len];
                Sequence::new(format!("READ{}", i), bases, quality)
            })
            .collect()
    }

    fn text_rows(module: &BasicStats) -> Vec<(String, String)> {
        let mut buf = Vec::new();
        module.write_text_report(&mut buf).expect("text report");
        String::from_utf8(buf)
            .expect("utf8")
            .lines()
            .skip(2) // "#Measure\tValue" header and the Filename row
            .filter_map(|line| line.split_once('\t'))
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// The live snapshot the progress table renders from must end up holding
    /// exactly the values the report is written from — that equality is the
    /// whole point of publishing counters rather than recomputing them.
    #[test]
    fn test_live_snapshot_matches_report() {
        let limits = Limits::new();
        let live = Arc::new(LiveStats::new());
        let mut module = BasicStats::new(&limits);
        module.set_file_name("sample.fastq");
        module.attach_live_stats(Arc::clone(&live));

        // Nothing published before any sequence has been seen, so the table
        // can show "-" rather than a misleading row of zeroes.
        assert!(live.snapshot().is_none());

        // Enough sequences to cross the publish interval several times.
        let seqs = sequences(PUBLISH_INTERVAL as usize * 4);
        for seq in &seqs {
            module.process_sequence(seq);
        }

        // Mid-run the snapshot is behind, but never ahead, of the true count.
        let mid = live.snapshot().expect("published during the run");
        assert!(mid.actual_count > 0);
        assert!(mid.actual_count <= seqs.len() as u64);

        module.finalize();
        let final_snapshot = live.snapshot().expect("published at finalize");
        assert_eq!(final_snapshot.actual_count, seqs.len() as u64);

        let expected: Vec<(String, String)> = final_snapshot
            .rows()
            .into_iter()
            .map(|(k, v)| (k.to_string(), v))
            .collect();
        assert_eq!(text_rows(&module), expected);
    }

    /// After the first snapshot, the module publishes only when the display
    /// asks for one, so a hidden table costs nothing.
    #[test]
    fn test_live_stats_publish_on_request() {
        let limits = Limits::new();
        let live = Arc::new(LiveStats::new());
        let mut module = BasicStats::new(&limits);
        module.attach_live_stats(Arc::clone(&live));

        let seqs = sequences(PUBLISH_INTERVAL as usize * 4);
        let (first, rest) = seqs.split_at(PUBLISH_INTERVAL as usize * 2);
        for seq in first {
            module.process_sequence(seq);
        }
        let published = live.snapshot().expect("first snapshot").actual_count;
        assert_eq!(published, PUBLISH_INTERVAL as u64);

        let (unasked, asked) = rest.split_at(PUBLISH_INTERVAL as usize);
        for seq in unasked {
            module.process_sequence(seq);
        }
        assert_eq!(live.snapshot().unwrap().actual_count, published);

        live.request();
        for seq in asked {
            module.process_sequence(seq);
        }
        assert!(live.snapshot().unwrap().actual_count > published);
    }

    /// Publishing is opt-in: a module with no sink attached still reports.
    #[test]
    fn test_no_live_stats_by_default() {
        let limits = Limits::new();
        let mut module = BasicStats::new(&limits);
        for seq in &sequences(100) {
            module.process_sequence(seq);
        }
        module.finalize();
        let rows = text_rows(&module);
        assert_eq!(rows[2], ("Total Sequences".to_string(), "100".to_string()));
    }

    /// Counters that have seen nothing still render every row, so the live
    /// table has something to show before a file has been opened — including
    /// a `%GC` that does not divide by zero.
    #[test]
    fn test_counters_rows_placeholder_state() {
        let counters = BasicStatsCounters::default();
        let rows = counters.rows();
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[0], ("File type", "null".to_string()));
        assert_eq!(rows[2], ("Total Sequences", "0".to_string()));
        // No bases at all must not divide by zero.
        assert_eq!(rows[7], ("%GC", "0".to_string()));
    }
}
