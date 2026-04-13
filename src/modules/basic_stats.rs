// Basic Statistics module
// Corresponds to Modules/BasicStats.java

use std::io;

use crate::config::Limits;
use crate::modules::QCModule;
use crate::sequence::Sequence;
use crate::utils::base_counts::{BASE_INDEX, IDX_A, IDX_C, IDX_G, IDX_N, IDX_T};
use crate::utils::phred;

pub struct BasicStats {
    name: Option<String>,
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
    // Java initialises lowestChar to 126 (char), which is the highest
    // printable ASCII. We use Option to represent "no quality chars seen yet".
    lowest_char: u8,
    file_type: Option<String>,
}

impl BasicStats {
    pub fn new(_limits: &Limits) -> Self {
        BasicStats {
            name: None,
            actual_count: 0,
            filtered_count: 0,
            min_length: 0,
            max_length: 0,
            total_bases: 0,
            g_count: 0,
            c_count: 0,
            a_count: 0,
            t_count: 0,
            n_count: 0,
            // Java starts at 126 (char), we mirror that
            lowest_char: 126,
            file_type: None,
        }
    }

    /// Set the filename, stripping any "stdin:" prefix.
    ///
    /// Matches `setFileName()` which strips "stdin:" prefix.
    pub fn set_file_name(&mut self, name: &str) {
        let name = name.strip_prefix("stdin:").unwrap_or(name);
        self.name = Some(name.to_string());
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
}

impl QCModule for BasicStats {
    fn process_sequence(&mut self, sequence: &Sequence) {
        // Java counts filtered sequences separately
        if sequence.is_filtered {
            self.filtered_count += 1;
            return;
        }

        self.actual_count += 1;
        self.total_bases += sequence.sequence.len() as u64;

        if self.file_type.is_none() {
            self.file_type = if sequence.colorspace.is_some() {
                Some("Colorspace converted to bases".to_string())
            } else {
                Some("Conventional base calls".to_string())
            };
        }

        // min/max length initialised on first non-filtered sequence
        let len = sequence.sequence.len();
        if self.actual_count == 1 {
            self.min_length = len;
            self.max_length = len;
        } else {
            self.min_length = self.min_length.min(len);
            self.max_length = self.max_length.max(len);
        }

        // Use lookup table to avoid branch misprediction on random DNA data
        let mut counts = [0u64; 6];
        for &b in &sequence.sequence {
            counts[BASE_INDEX[b as usize] as usize] += 1;
        }
        self.a_count += counts[IDX_A];
        self.c_count += counts[IDX_C];
        self.g_count += counts[IDX_G];
        self.t_count += counts[IDX_T];
        self.n_count += counts[IDX_N];

        for &q in &sequence.quality {
            if q < self.lowest_char {
                self.lowest_char = q;
            }
        }
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
        self.min_length = 0;
        self.max_length = 0;
        self.g_count = 0;
        self.c_count = 0;
        self.a_count = 0;
        self.t_count = 0;
        self.n_count = 0;
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

        // Row 1: File type
        writeln!(
            writer,
            "File type\t{}",
            self.file_type
                .as_deref()
                .unwrap_or("Conventional base calls")
        )?;

        // Row 2: Encoding
        // Uses PhredEncoding.getFastQEncodingOffset(lowestChar)
        let encoding_name = phred::detect(self.lowest_char)
            .map(|e| e.name.to_string())
            .unwrap_or_else(|_| "Unknown".to_string());
        writeln!(writer, "Encoding\t{}", encoding_name)?;

        // Row 3: Total Sequences
        writeln!(writer, "Total Sequences\t{}", self.actual_count)?;

        // Row 4: Total Bases
        writeln!(
            writer,
            "Total Bases\t{}",
            Self::format_length(self.total_bases)
        )?;

        // Row 5: Sequences flagged as poor quality
        writeln!(
            writer,
            "Sequences flagged as poor quality\t{}",
            self.filtered_count
        )?;

        // Row 6: Sequence length
        if self.min_length == self.max_length {
            writeln!(writer, "Sequence length\t{}", self.min_length)?;
        } else {
            writeln!(
                writer,
                "Sequence length\t{}-{}",
                self.min_length, self.max_length
            )?;
        }

        // Row 7: %GC
        // JAVA COMPAT: Integer division: ((gCount+cCount)*100)/(aCount+tCount+gCount+cCount)
        let total = self.a_count + self.t_count + self.g_count + self.c_count;
        let gc = if total > 0 {
            ((self.g_count + self.c_count) * 100) / total
        } else {
            0
        };
        writeln!(writer, "%GC\t{}", gc)?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_length_bp() {
        assert_eq!(BasicStats::format_length(16), "16 bp");
        assert_eq!(BasicStats::format_length(80), "80 bp");
        assert_eq!(BasicStats::format_length(999), "999 bp");
    }

    #[test]
    fn test_format_length_kbp() {
        assert_eq!(BasicStats::format_length(1000), "1 kbp");
        assert_eq!(BasicStats::format_length(1500), "1.5 kbp");
        assert_eq!(BasicStats::format_length(10000), "10 kbp");
    }

    #[test]
    fn test_format_length_mbp() {
        assert_eq!(BasicStats::format_length(1_000_000), "1 Mbp");
        assert_eq!(BasicStats::format_length(1_200_000), "1.2 Mbp");
    }

    #[test]
    fn test_format_length_gbp() {
        assert_eq!(BasicStats::format_length(1_000_000_000), "1 Gbp");
    }
}
