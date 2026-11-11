// Per Sequence GC Content module
// Corresponds to Modules/PerSequenceGCContent.java

use std::collections::HashMap;
use std::io;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::report::charts::CHART_WIDTH;
use crate::sequence::Sequence;
use crate::utils::base_counts::count_acgt;
use crate::utils::format::java_format_double;

/// Mirrors GCModel/GCModel.java: maps a GC base count to weighted percentage
/// bins, so that counts at bin boundaries are shared.
///
/// Java stores the bins and weights of every GC count (one small array per
/// base of the read). Only the claim count of each bin is kept here, 101
/// numbers per length, and the read's own bins are derived when needed, so
/// every length stays cached even on megabase reads.
struct GCModel {
    read_length: usize,
    /// How many GC counts claim each percentage bin.
    claiming_counts: [usize; 101],
}

impl GCModel {
    fn new(read_length: usize) -> Self {
        let mut claiming_counts = [0usize; 101];
        for pos in 0..=read_length {
            for cc in &mut claiming_counts[Self::bins(read_length, pos)] {
                *cc += 1;
            }
        }
        GCModel {
            read_length,
            claiming_counts,
        }
    }

    /// The percentage bins a GC count of `pos` falls in, as in GCModel.java.
    fn bins(read_length: usize, pos: usize) -> std::ops::RangeInclusive<usize> {
        let low_count = if pos == 0 { 0.0 } else { pos as f64 - 0.5 };
        let high_count = (pos as f64 + 0.5).min(read_length as f64);
        // clamp is also applied for lowCount < 0 (only matters for pos==0)
        let low_count = low_count.max(0.0);

        let low_percentage = ((low_count * 100.0) / read_length as f64).round() as usize;
        let high_percentage = ((high_count * 100.0) / read_length as f64).round() as usize;
        low_percentage..=high_percentage
    }

    /// `(percentage, increment)` for each bin a read with `gc_count` G/C adds to.
    fn values(&self, gc_count: usize) -> impl Iterator<Item = (usize, f64)> + '_ {
        Self::bins(self.read_length, gc_count).map(|p| (p, 1.0 / self.claiming_counts[p] as f64))
    }
}

pub struct PerSequenceGCContent {
    gc_distribution: [f64; 101],
    /// One model per (truncated) read length seen: at most ~110 below 1 kb,
    /// plus one per kb of the longest read.
    models: HashMap<usize, GCModel>,
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
            models: HashMap::new(),
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

        // Java passes raw max to LineGraph (no ceil rounding)
        let max_y = max_val;

        let x_categories: Vec<String> = (0..101).map(|i| format!("{}", i)).collect();

        // Two series: GC distribution and theoretical distribution
        render_line_graph(&LineGraphData {
            width: CHART_WIDTH,
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
    fn cost_hint(&self) -> u32 {
        2
    }

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
        let [_, c, g, _] = count_acgt(&seq[..truncated_len]);
        let gc_count = (c + g) as usize;

        let model = self
            .models
            .entry(truncated_len)
            .or_insert_with(|| GCModel::new(truncated_len));
        for (percentage, increment) in model.values(gc_count) {
            self.gc_distribution[percentage] += increment;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// GCModel.java's table, built the way Java builds it: the bins and
    /// increment of every GC count.
    fn java_model(read_length: usize) -> Vec<Vec<(usize, f64)>> {
        let mut claiming = vec![0usize; 101];
        for pos in 0..=read_length {
            for cc in &mut claiming[GCModel::bins(read_length, pos)] {
                *cc += 1;
            }
        }
        (0..=read_length)
            .map(|pos| {
                GCModel::bins(read_length, pos)
                    .map(|p| (p, 1.0 / claiming[p] as f64))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn test_model_matches_java_table() {
        for len in [1, 2, 7, 50, 99, 100, 150, 1000, 3000, 25000] {
            let model = GCModel::new(len);
            for (gc, expected) in java_model(len).iter().enumerate() {
                assert_eq!(
                    &model.values(gc).collect::<Vec<_>>(),
                    expected,
                    "len {len} gc {gc}"
                );
            }
        }
    }

    #[test]
    fn test_reads_of_many_lengths_add_up() {
        let lengths = [2500, 3600, 2500, 150, 3600, 75];
        let mut module = PerSequenceGCContent::new(&Limits::new());
        let mut expected = [0.0f64; 101];
        for (i, &len) in lengths.iter().enumerate() {
            let bases: Vec<u8> = (0..len).map(|j| b"ACGTG"[(i + j) % 5]).collect();
            let truncated = PerSequenceGCContent::truncate_length(len);
            let [_, c, g, _] = count_acgt(&bases[..truncated]);
            for &(p, inc) in &java_model(truncated)[(c + g) as usize] {
                expected[p] += inc;
            }
            module.process_sequence(&Sequence::new(format!("@r{i}"), bases, vec![b'I'; len]));
        }
        assert_eq!(module.gc_distribution, expected);
        assert_eq!(module.models.len(), 4);
    }
}
