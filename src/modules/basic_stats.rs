// Basic Statistics module
// Corresponds to Modules/BasicStats.java

use std::io;
use std::sync::{Arc, Mutex};

use crate::config::Limits;
use crate::modules::QCModule;
use crate::sequence::Sequence;
use crate::utils::base_counts::{BASE_INDEX, IDX_A, IDX_C, IDX_G, IDX_N, IDX_T};
use crate::utils::phred;

/// Sequences between publications of the counters to the live snapshot.
///
/// Only an upper bound on the update rate: the display samples the snapshot on
/// its own schedule, so this just has to be small enough that a slow file
/// (nanopore reads take orders of magnitude longer than Illumina ones) still
/// looks alive, and large enough that the per-read cost rounds to nothing.
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
#[derive(Debug, Clone, Copy, Default)]
pub struct BasicStatsCounters {
    actual_count: u64,
    filtered_count: u64,
    min_length: usize,
    max_length: usize,
    total_bases: u64,
    g_count: u64,
    c_count: u64,
    a_count: u64,
    t_count: u64,
    n_count: u64,
    /// Lowest quality character seen. Java initialises this to 126 (the
    /// highest printable ASCII) and lowers it as sequences are read.
    lowest_char: u8,
    /// Whether the base calls were converted from colorspace, for the "File
    /// type" row. Java leaves the field null until the first sequence and
    /// prints "Conventional base calls" for it, so `false` is also the
    /// no-sequences-yet value.
    colorspace: bool,
}

impl BasicStatsCounters {
    fn new() -> Self {
        BasicStatsCounters {
            // Java starts at 126 (char), we mirror that
            lowest_char: 126,
            ..Default::default()
        }
    }

    /// The measures reported, in report order, minus the leading "Filename"
    /// row. [`Self::rows`] returns a value for each of these, in this order.
    pub const MEASURES: [&'static str; 7] = [
        "File type",
        "Encoding",
        "Total Sequences",
        "Total Bases",
        "Sequences flagged as poor quality",
        "Sequence length",
        "%GC",
    ];

    /// The Basic Statistics rows, minus the leading "Filename" row, in report
    /// order. Both the text report and the live progress table render from
    /// this, so the values on screen always agree with the values on disk.
    pub fn rows(&self) -> Vec<(&'static str, String)> {
        // Uses PhredEncoding.getFastQEncodingOffset(lowestChar)
        let encoding = phred::detect(self.lowest_char)
            .map(|e| e.name.to_string())
            .unwrap_or_else(|_| "Unknown".to_string());

        let sequence_length = if self.min_length == self.max_length {
            self.min_length.to_string()
        } else {
            format!("{}-{}", self.min_length, self.max_length)
        };

        // JAVA COMPAT: Integer division: ((gCount+cCount)*100)/(aCount+tCount+gCount+cCount)
        let total = self.a_count + self.t_count + self.g_count + self.c_count;
        let gc = ((self.g_count + self.c_count) * 100)
            .checked_div(total)
            .unwrap_or(0);

        let file_type = if self.colorspace {
            "Colorspace converted to bases"
        } else {
            "Conventional base calls"
        };

        let values = [
            file_type.to_string(),
            encoding,
            self.actual_count.to_string(),
            format_length(self.total_bases),
            self.filtered_count.to_string(),
            sequence_length,
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
#[derive(Default)]
pub struct LiveStats {
    snapshot: Mutex<Option<BasicStatsCounters>>,
}

impl LiveStats {
    pub fn new() -> Self {
        Self::default()
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
    let unit;

    if length >= 1_000_000_000.0 {
        length /= 1_000_000_000.0;
        unit = " Gbp";
    } else if length >= 1_000_000.0 {
        length /= 1_000_000.0;
        unit = " Mbp";
    } else if length >= 1_000.0 {
        length /= 1_000.0;
        unit = " kbp";
    } else {
        unit = " bp";
    }

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
    /// Optional live snapshot sink for the terminal progress table.
    live: Option<Arc<LiveStats>>,
    /// Sequences processed since the counters were last published.
    since_publish: u32,
}

impl BasicStats {
    pub fn new(_limits: &Limits) -> Self {
        BasicStats {
            name: None,
            counters: BasicStatsCounters::new(),
            live: None,
            since_publish: 0,
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
    fn publish(&self) {
        if let Some(ref live) = self.live {
            live.publish(self.counters);
        }
    }
}

impl QCModule for BasicStats {
    fn cost_hint(&self) -> u32 {
        1
    }

    fn process_sequence(&mut self, sequence: &Sequence) {
        // Publish a snapshot for the live progress table every so often. Done
        // first so the counter is advanced even for filtered sequences.
        self.since_publish += 1;
        if self.since_publish >= PUBLISH_INTERVAL {
            self.since_publish = 0;
            self.publish();
        }

        // Java counts filtered sequences separately
        if sequence.is_filtered {
            self.counters.filtered_count += 1;
            return;
        }

        let c = &mut self.counters;
        c.actual_count += 1;
        c.total_bases += sequence.sequence.len() as u64;

        // Both the file type and the length range are taken from the first
        // non-filtered sequence, as Java does.
        let len = sequence.sequence.len();
        if c.actual_count == 1 {
            c.colorspace = sequence.colorspace.is_some();
            c.min_length = len;
            c.max_length = len;
        } else {
            c.min_length = c.min_length.min(len);
            c.max_length = c.max_length.max(len);
        }

        // Use lookup table to avoid branch misprediction on random DNA data
        let mut counts = [0u64; 6];
        for &b in &sequence.sequence {
            counts[BASE_INDEX[b as usize] as usize] += 1;
        }
        c.a_count += counts[IDX_A];
        c.c_count += counts[IDX_C];
        c.g_count += counts[IDX_G];
        c.t_count += counts[IDX_T];
        c.n_count += counts[IDX_N];

        for &q in &sequence.quality {
            if q < c.lowest_char {
                c.lowest_char = q;
            }
        }
    }

    fn attach_live_stats(&mut self, live: Arc<LiveStats>) {
        self.live = Some(live);
    }

    /// Publish the final counters so the progress table ends on exactly the
    /// values that go into the report.
    fn finalize(&mut self) {
        self.publish();
    }

    fn set_filename(&mut self, name: &str) {
        self.set_file_name(name);
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
        self.counters.n_count = 0;
    }

    // BasicStats never raises error or warning
    fn raises_error(&self) -> bool {
        false
    }

    fn raises_warning(&self) -> bool {
        false
    }

    fn ignore_filtered_sequences(&self) -> bool {
        // BasicStats processes filtered sequences (to count them)
        false
    }

    fn ignore_in_report(&self) -> bool {
        false
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        // Header row matches writeTextTable output from AbstractQCModule
        writeln!(writer, "#Measure\tValue")?;

        // Row 0: Filename
        writeln!(writer, "Filename\t{}", self.name.as_deref().unwrap_or(""))?;

        // Rows 1-7: File type, Encoding, Total Sequences, Total Bases,
        // Sequences flagged as poor quality, Sequence length, %GC.
        for (measure, value) in self.counters.rows() {
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

    /// Publishing is opt-in: a module with no sink attached must behave
    /// exactly as before.
    #[test]
    fn test_no_live_stats_by_default() {
        let limits = Limits::new();
        let mut module = BasicStats::new(&limits);
        for seq in &sequences(100) {
            module.process_sequence(seq);
        }
        module.finalize();
        assert!(module.live.is_none());
        let rows = text_rows(&module);
        assert_eq!(rows[2], ("Total Sequences".to_string(), "100".to_string()));
    }

    /// Filtered sequences are counted, not analysed.
    #[test]
    fn test_counters_rows_placeholder_state() {
        let counters = BasicStatsCounters::new();
        let rows = counters.rows();
        assert_eq!(rows.len(), 7);
        assert_eq!(rows[0].0, "File type");
        assert_eq!(rows[0].1, "Conventional base calls");
        assert_eq!(rows[2], ("Total Sequences", "0".to_string()));
        // No bases at all must not divide by zero.
        assert_eq!(rows[6], ("%GC", "0".to_string()));
    }
}
