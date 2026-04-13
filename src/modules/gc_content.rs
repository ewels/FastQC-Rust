// Per Sequence GC Content module
// Corresponds to Modules/PerSequenceGCContent.java

use std::io;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::find_optimal_y_interval;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::sequence::Sequence;
use crate::utils::format::java_format_double;

/// Mirrors GCModel/GCModelValue.java - a single percentage bin and its increment weight.
struct GCModelValue {
    percentage: usize,
    increment: f64,
}

/// Mirrors GCModel/GCModel.java - maps a GC base count to weighted percentage bins.
/// For a given read length, each possible GC count (0..=length) maps to one or more percentage
/// bins with fractional increments so that counts at bin boundaries are shared.
struct GCModel {
    models: Vec<Vec<GCModelValue>>,
}

impl GCModel {
    fn new(read_length: usize) -> Self {
        // Two-pass algorithm from GCModel.java
        // First pass: count how many GC-count positions claim each percentage bin
        let mut claiming_counts = vec![0usize; 101];

        for pos in 0..=read_length {
            let low_count = if pos == 0 { 0.0 } else { pos as f64 - 0.5 };
            let high_count = (pos as f64 + 0.5).min(read_length as f64);
            // clamp is also applied for lowCount < 0 (only matters for pos==0)
            let low_count = low_count.max(0.0);

            let low_percentage = ((low_count * 100.0) / read_length as f64).round() as usize;
            let high_percentage = ((high_count * 100.0) / read_length as f64).round() as usize;

            for cc in &mut claiming_counts[low_percentage..=high_percentage] {
                *cc += 1;
            }
        }

        // Second pass: build the model with weighted increments
        let mut models = Vec::with_capacity(read_length + 1);

        for pos in 0..=read_length {
            let low_count = if pos == 0 { 0.0 } else { pos as f64 - 0.5 };
            let high_count = (pos as f64 + 0.5).min(read_length as f64);
            let low_count = low_count.max(0.0);

            let low_percentage = ((low_count * 100.0) / read_length as f64).round() as usize;
            let high_percentage = ((high_count * 100.0) / read_length as f64).round() as usize;

            let mut values = Vec::with_capacity((high_percentage - low_percentage) + 1);
            // need both index p (for percentage field) and claiming_counts[p]
            for (p, &cc) in (low_percentage..=high_percentage)
                .zip(&claiming_counts[low_percentage..=high_percentage])
            {
                values.push(GCModelValue {
                    percentage: p,
                    increment: 1.0 / cc as f64,
                });
            }
            models.push(values);
        }

        GCModel { models }
    }

    fn get_model_values(&self, gc_count: usize) -> &[GCModelValue] {
        &self.models[gc_count]
    }
}

pub struct PerSequenceGCContent {
    gc_distribution: [f64; 101],
    // Cache GCModels by read length to avoid recomputing
    cached_models: Vec<Option<GCModel>>,
    limits: Limits,
    // Lazily computed results
    deviation_percent: Option<f64>,
    /// Theoretical normal distribution for the chart, computed alongside deviation_percent.
    theoretical_distribution: Option<[f64; 101]>,
}

impl PerSequenceGCContent {
    pub fn new(limits: &Limits) -> Self {
        PerSequenceGCContent {
            gc_distribution: [0.0; 101],
            // Initial cache size 200, grows as needed
            cached_models: Vec::new(),
            limits: limits.clone(),
            deviation_percent: None,
            theoretical_distribution: None,
        }
    }

    /// Truncate sequence to reduce number of distinct GCModel lengths.
    /// Sequences >1000bp are truncated to a multiple of 1000, >100bp to a multiple of 100.
    fn truncate_length(len: usize) -> usize {
        if len > 1000 {
            (len / 1000) * 1000
        } else if len > 100 {
            (len / 100) * 100
        } else {
            len
        }
    }

    /// Calculate the theoretical normal distribution and deviation percentage.
    fn calculate_distribution(&mut self) {
        if self.deviation_percent.is_some() {
            return;
        }

        let mut total_count: f64 = 0.0;
        let mut first_mode: usize = 0;
        let mut mode_count: f64 = 0.0;

        for i in 0..101 {
            total_count += self.gc_distribution[i];
            if self.gc_distribution[i] > mode_count {
                mode_count = self.gc_distribution[i];
                first_mode = i;
            }
        }

        // Average over adjacent points that stay above 90% of the modal value
        // (the comment says 95% but the code checks gcDistribution[firstMode] - gcDistribution[firstMode]/10,
        // which is 90% of the mode value)
        let mut mode: f64 = 0.0;
        let mut mode_duplicates: usize = 0;
        let mut fell_off_top = true;

        for i in first_mode..101 {
            if self.gc_distribution[i]
                > self.gc_distribution[first_mode] - (self.gc_distribution[first_mode] / 10.0)
            {
                mode += i as f64;
                mode_duplicates += 1;
            } else {
                fell_off_top = false;
                break;
            }
        }

        let mut fell_off_bottom = true;
        if first_mode > 0 {
            for i in (0..first_mode).rev() {
                if self.gc_distribution[i]
                    > self.gc_distribution[first_mode] - (self.gc_distribution[first_mode] / 10.0)
                {
                    mode += i as f64;
                    mode_duplicates += 1;
                } else {
                    fell_off_bottom = false;
                    break;
                }
            }
        }

        // If distribution is so skewed that 90% of the mode falls off the
        // 0-100% scale, keep first_mode as center
        if fell_off_bottom || fell_off_top {
            mode = first_mode as f64;
        } else {
            mode /= mode_duplicates as f64;
        }

        // Calculate standard deviation
        let mut stdev: f64 = 0.0;
        for i in 0..101 {
            stdev += (i as f64 - mode).powi(2) * self.gc_distribution[i];
        }
        // Divides by totalCount-1 (Bessel's correction)
        stdev /= total_count - 1.0;
        stdev = stdev.sqrt();

        // Calculate theoretical distribution using the normal PDF
        // NormalDistribution.getZScoreForValue() is actually a PDF, not a z-score
        let mut deviation_percent: f64 = 0.0;
        let mut theoretical = [0.0f64; 101];

        // Calculate theoretical[i] and compare with gc_distribution[i]
        for (i, (theo, &observed)) in theoretical
            .iter_mut()
            .zip(self.gc_distribution.iter())
            .enumerate()
        {
            let lhs = 1.0 / (2.0 * std::f64::consts::PI * stdev * stdev).sqrt();
            let rhs =
                std::f64::consts::E.powf(-(((i as f64) - mode).powi(2)) / (2.0 * stdev * stdev));
            *theo = lhs * rhs * total_count;

            deviation_percent += (*theo - observed).abs();
        }

        deviation_percent /= total_count;
        deviation_percent *= 100.0;

        self.theoretical_distribution = Some(theoretical);
        self.deviation_percent = Some(deviation_percent);
    }

    fn get_deviation_percent(&self) -> f64 {
        self.deviation_percent.unwrap_or(0.0)
    }
}

impl PerSequenceGCContent {
    fn build_chart_svg(&self) -> String {
        let gc_dist: Vec<f64> = self.gc_distribution.to_vec();
        let theoretical = self.theoretical_distribution.unwrap_or([0.0; 101]).to_vec();

        // max is the maximum of either distribution
        let max_val = gc_dist
            .iter()
            .chain(theoretical.iter())
            .cloned()
            .fold(0.0_f64, f64::max);

        let y_interval = find_optimal_y_interval(max_val);
        let max_y = (max_val / y_interval).ceil() * y_interval;

        let x_categories: Vec<String> = (0..101).map(|i| format!("{}", i)).collect();

        // Two series: GC distribution and theoretical distribution
        render_line_graph(&LineGraphData {
            data: vec![gc_dist, theoretical],
            min_y: 0.0,
            max_y,
            x_label: "Mean GC content (%)".to_string(),
            series_names: vec![
                "GC count per read".to_string(),
                "Theoretical Distribution".to_string(),
            ],
            x_categories,
            title: "GC distribution over all sequences".to_string(),
        })
    }
}

impl QCModule for PerSequenceGCContent {
    fn process_sequence(&mut self, sequence: &Sequence) {
        // Invalidate cached calculation when new data arrives
        self.deviation_percent = None;
        self.theoretical_distribution = None;

        let seq = &sequence.sequence;
        let truncated_len = Self::truncate_length(seq.len());
        if truncated_len == 0 {
            return;
        }

        // Count G and C in the truncated portion only
        let mut gc_count: usize = 0;
        for &b in &seq[..truncated_len] {
            if b == b'G' || b == b'C' {
                gc_count += 1;
            }
        }

        // Ensure cache is large enough
        if truncated_len >= self.cached_models.len() {
            self.cached_models.resize_with(truncated_len + 1, || None);
        }

        // Create model if not cached
        if self.cached_models[truncated_len].is_none() {
            self.cached_models[truncated_len] = Some(GCModel::new(truncated_len));
        }

        let model = self.cached_models[truncated_len].as_ref().unwrap();
        let values = model.get_model_values(gc_count);

        for v in values {
            self.gc_distribution[v.percentage] += v.increment;
        }
    }

    fn name(&self) -> &str {
        "Per sequence GC content"
    }

    fn description(&self) -> &str {
        "Shows the distribution of GC contents for whole sequences"
    }

    fn reset(&mut self) {
        self.gc_distribution = [0.0; 101];
        self.deviation_percent = None;
        self.theoretical_distribution = None;
    }

    fn finalize(&mut self) {
        self.calculate_distribution();
    }

    fn raises_error(&self) -> bool {
        self.get_deviation_percent() > self.limits.threshold("gc_sequence\terror", 30.0)
    }

    fn raises_warning(&self) -> bool {
        self.get_deviation_percent() > self.limits.threshold("gc_sequence\twarn", 15.0)
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        self.limits.is_ignored("gc_sequence")
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        // Header line with #GC Content\tCount
        writeln!(writer, "#GC Content\tCount")?;
        // Always output all 101 rows (0-100)
        for i in 0..101 {
            writeln!(
                writer,
                "{}\t{}",
                i,
                java_format_double(self.gc_distribution[i])
            )?;
        }
        Ok(())
    }

    // Image filename matches Java's "per_sequence_gc_content.png" in Images/
    fn chart_image_name(&self) -> Option<&str> {
        Some("per_sequence_gc_content")
    }
    fn chart_alt_text(&self) -> Option<&str> {
        Some("Per sequence GC content graph")
    }
    fn generate_chart_svg(&self) -> Option<String> {
        Some(self.build_chart_svg())
    }
}
