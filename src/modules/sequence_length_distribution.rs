// Sequence Length Distribution module
// Corresponds to Modules/SequenceLengthDistribution.java

use std::io;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::find_optimal_y_interval;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::sequence::Sequence;
use crate::utils::format::java_format_double;

pub struct SequenceLengthDistribution {
    /// length_counts[i] = number of sequences with length i
    length_counts: Vec<u64>,
    limits: Limits,
    nogroup: bool,
    // Lazily computed results
    computed: Option<ComputedDistribution>,
}

struct ComputedDistribution {
    graph_counts: Vec<f64>,
    x_categories: Vec<String>,
}

impl SequenceLengthDistribution {
    pub fn new(limits: &Limits, nogroup: bool) -> Self {
        SequenceLengthDistribution {
            length_counts: Vec::new(),
            limits: limits.clone(),
            nogroup,
            computed: None,
        }
    }

    /// Find interval and starting point for binning sequence lengths.
    /// Replicates getSizeDistribution() from SequenceLengthDistribution.java.
    fn get_size_distribution(min: usize, max: usize, nogroup: bool) -> (usize, usize) {
        // If nogroup is set, don't bin
        if nogroup {
            return (min, 1);
        }

        // Find the smallest interval from [1,2,5]*base that gives <=50 bins
        let mut base: usize = 1;

        // The Java code starts with base=1 and has `while (base > (max-min)) base /= 10`
        // which is a no-op when base starts at 1 (since 1 is always <= max-min for any valid range).
        // We skip this loop as it has no effect in practice.

        let divisions = [1, 2, 5];
        let interval;

        'outer: loop {
            for &d in &divisions {
                let tester = base * d;
                if (max - min) / tester <= 50 {
                    interval = tester;
                    break 'outer;
                }
            }
            base *= 10;
        }

        // Calculate starting value aligned to interval boundary
        let basic_division = min / interval;
        let starting = basic_division * interval;

        (starting, interval)
    }

    fn calculate_distribution(&mut self) {
        if self.computed.is_some() {
            return;
        }

        let mut max_len: usize = 0;
        let mut min_len: Option<usize> = None;

        // Find min and max lengths
        for i in 0..self.length_counts.len() {
            if self.length_counts[i] > 0 {
                if min_len.is_none() {
                    min_len = Some(i);
                }
                max_len = i;
            }
        }

        // Default min to 0 if no sequences
        let mut min_len = min_len.unwrap_or(0);

        // Add one extra category on either side
        min_len = min_len.saturating_sub(1);
        max_len += 1;

        let (starting, interval) = Self::get_size_distribution(min_len, max_len, self.nogroup);

        // Count how many categories we need
        let mut categories = 0;
        let mut current_value = starting;
        while current_value <= max_len {
            categories += 1;
            current_value += interval;
        }

        let mut graph_counts = vec![0.0f64; categories];
        let mut x_categories = Vec::with_capacity(categories);

        // i needed to compute bin boundaries
        for (i, gc) in graph_counts.iter_mut().enumerate() {
            let min_value = starting + (interval * i);
            let mut max_value = (starting + (interval * (i + 1))) - 1;

            // Clamp max_value to maxLen
            if max_value > max_len {
                max_value = max_len;
            }

            // Sum counts in this bin
            for bp in min_value..=max_value {
                if bp < self.length_counts.len() {
                    *gc += self.length_counts[bp] as f64;
                }
            }

            // Label format depends on interval
            if interval == 1 {
                x_categories.push(format!("{}", min_value));
            } else {
                x_categories.push(format!("{}-{}", min_value, max_value));
            }
        }

        self.computed = Some(ComputedDistribution {
            graph_counts,
            x_categories,
        });
    }

    fn ensure_calculated(&self) -> &ComputedDistribution {
        // SAFETY: finalize() must be called before any reporting method.
        // If a caller skips finalize(), we provide a static default to avoid panicking.
        static DEFAULT: ComputedDistribution = ComputedDistribution {
            graph_counts: Vec::new(),
            x_categories: Vec::new(),
        };
        self.computed.as_ref().unwrap_or(&DEFAULT)
    }
}

impl SequenceLengthDistribution {
    fn build_chart_svg(&self) -> String {
        let computed = self.ensure_calculated();
        let max_val = computed
            .graph_counts
            .iter()
            .cloned()
            .fold(0.0_f64, f64::max);
        let y_interval = find_optimal_y_interval(max_val);
        let max_y = (max_val / y_interval).ceil() * y_interval;

        // Matches Java constructor call
        render_line_graph(&LineGraphData {
            data: vec![computed.graph_counts.clone()],
            min_y: 0.0,
            max_y,
            x_label: "Sequence Length (bp)".to_string(),
            series_names: vec!["Sequence Length".to_string()],
            x_categories: computed.x_categories.clone(),
            title: "Distribution of sequence lengths over all sequences".to_string(),
        })
    }
}

impl QCModule for SequenceLengthDistribution {
    fn process_sequence(&mut self, sequence: &Sequence) {
        self.computed = None;
        let seq_len = sequence.sequence.len();

        // Array is extended to seqLen+2 to match Java's `seqLen+2 > lengthCounts.length`
        if seq_len + 2 > self.length_counts.len() {
            self.length_counts.resize(seq_len + 2, 0);
        }

        self.length_counts[seq_len] += 1;
    }

    fn name(&self) -> &str {
        "Sequence Length Distribution"
    }

    fn description(&self) -> &str {
        "Shows the distribution of sequence length over all sequences"
    }

    fn reset(&mut self) {
        self.length_counts.clear();
        self.computed = None;
    }

    fn finalize(&mut self) {
        self.calculate_distribution();
    }

    fn raises_error(&self) -> bool {
        // If error threshold is 0, the test is disabled
        let threshold = self.limits.threshold("sequence_length\terror", 1.0);
        if threshold == 0.0 {
            return false;
        }

        // Error if there are sequences of length 0
        if !self.length_counts.is_empty() && self.length_counts[0] > 0 {
            return true;
        }
        false
    }

    fn raises_warning(&self) -> bool {
        // If warn threshold is 0, the test is disabled
        let threshold = self.limits.threshold("sequence_length\twarn", 1.0);
        if threshold == 0.0 {
            return false;
        }

        // Warn if there are sequences of different lengths
        let mut seen_length = false;
        for &count in &self.length_counts {
            if count > 0 {
                if seen_length {
                    return true;
                }
                seen_length = true;
            }
        }
        false
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        self.limits.is_ignored("sequence_length")
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let computed = self.ensure_calculated();

        writeln!(writer, "#Length\tCount")?;
        for i in 0..computed.x_categories.len() {
            // Skip empty padding bins at the start and end
            if (i == 0 || i == computed.x_categories.len() - 1) && computed.graph_counts[i] == 0.0 {
                continue;
            }
            writeln!(
                writer,
                "{}\t{}",
                computed.x_categories[i],
                java_format_double(computed.graph_counts[i])
            )?;
        }
        Ok(())
    }

    // Image filename matches Java's "sequence_length_distribution.png" in Images/
    fn chart_image_name(&self) -> Option<&str> {
        Some("sequence_length_distribution")
    }
    fn chart_alt_text(&self) -> Option<&str> {
        Some("Sequence length distribution")
    }
    fn generate_chart_svg(&self) -> Option<String> {
        Some(self.build_chart_svg())
    }
}
