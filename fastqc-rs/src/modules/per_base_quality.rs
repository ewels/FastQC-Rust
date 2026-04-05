// Per Base Sequence Quality module
// Corresponds to Modules/PerBaseQualityScores.java

use std::io;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::quality_boxplot::{QualityBoxPlotData, render_quality_boxplot};
use crate::sequence::Sequence;
use crate::utils::base_group::BaseGroup;
use crate::utils::format::java_format_double;
use crate::utils::phred;
use crate::utils::quality_count::{self, QualityCount};

pub struct PerBaseQualityScores {
    quality_counts: Vec<QualityCount>,
    nogroup: bool,
    expgroup: bool,
    limits: Limits,
}

impl PerBaseQualityScores {
    pub fn new(limits: &Limits, nogroup: bool, expgroup: bool) -> Self {
        PerBaseQualityScores {
            quality_counts: Vec::new(),
            nogroup,
            expgroup,
            limits: limits.clone(),
        }
    }

    fn calculate(&self) -> CalculatedData {
        let (min_char, _max_char) = quality_count::calculate_offsets(&self.quality_counts);
        // If no quality data, default to Sanger offset (33).
        let offset = phred::detect(min_char)
            .map(|e| e.offset)
            .unwrap_or(33);

        let groups = BaseGroup::make_base_groups(
            self.quality_counts.len(),
            self.nogroup,
            self.expgroup,
        );

        let mut means = vec![0.0f64; groups.len()];
        let mut medians = vec![0.0f64; groups.len()];
        let mut lower_quartile = vec![0.0f64; groups.len()];
        let mut upper_quartile = vec![0.0f64; groups.len()];
        let mut lowest = vec![0.0f64; groups.len()];
        let mut highest = vec![0.0f64; groups.len()];
        let mut x_labels = Vec::with_capacity(groups.len());

        for (i, group) in groups.iter().enumerate() {
            x_labels.push(group.label());
            // Java uses 1-based lowerCount/upperCount; our BaseGroup
            // stores 0-based lower_count/upper_count.
            let min_base = group.lower_count;
            let max_base = group.upper_count;
            lowest[i] = self.get_percentile(min_base, max_base, offset, 10);
            highest[i] = self.get_percentile(min_base, max_base, offset, 90);
            means[i] = self.get_mean(min_base, max_base, offset);
            medians[i] = self.get_percentile(min_base, max_base, offset, 50);
            lower_quartile[i] = self.get_percentile(min_base, max_base, offset, 25);
            upper_quartile[i] = self.get_percentile(min_base, max_base, offset, 75);
        }

        CalculatedData {
            means,
            medians,
            lower_quartile,
            upper_quartile,
            lowest,
            highest,
            x_labels,
        }
    }

    /// Replicates `getPercentile(int minbp, int maxbp, int offset, int percentile)`.
    /// Only includes positions with >100 total counts.
    /// minbp and maxbp are 0-based inclusive indices into quality_counts.
    fn get_percentile(&self, min_base: usize, max_base: usize, offset: u8, percentile: u8) -> f64 {
        let mut count = 0;
        let mut total = 0.0;

        // Java loop is `for (int i=minbp-1;i<maxbp;i++)` where minbp
        // is 1-based. Our min_base is 0-based, so we iterate min_base..=max_base.
        for i in min_base..=max_base {
            // Only include positions with >100 counts for percentile calculation
            if self.quality_counts[i].get_total_count() > 100 {
                count += 1;
                total += self.quality_counts[i].get_percentile(offset, percentile);
            }
        }

        if count > 0 {
            total / count as f64
        } else {
            f64::NAN
        }
    }

    /// Replicates `getMean(int minbp, int maxbp, int offset)`.
    /// Only includes positions with >0 total counts.
    fn get_mean(&self, min_base: usize, max_base: usize, offset: u8) -> f64 {
        let mut count = 0;
        let mut total = 0.0;

        for i in min_base..=max_base {
            // getMean includes positions with >0 counts (not >100 like percentile)
            if self.quality_counts[i].get_total_count() > 0 {
                count += 1;
                total += self.quality_counts[i].get_mean(offset);
            }
        }

        if count > 0 {
            total / count as f64
        } else {
            // Java returns 0 when count is 0 for mean (not NaN)
            0.0
        }
    }
}

impl PerBaseQualityScores {
    /// Generate the SVG chart for this module.
    fn build_chart_svg(&self) -> String {
        let data = self.calculate();
        let (min_char, _) = quality_count::calculate_offsets(&self.quality_counts);
        let encoding_name = phred::detect(min_char)
            .map(|e| e.name)
            .unwrap_or("Sanger / Illumina 1.9");

        // The chart title includes the encoding scheme name
        let title = format!(
            "Quality scores across all bases ({} encoding)",
            encoding_name
        );

        // maxY is calculated in Java as Math.ceil(max/yInterval)*yInterval
        // where max starts at highest value rounded up. In practice Java uses
        // the highest percentile value + padding. We compute a sensible max.
        let mut max_val: f64 = 0.0;
        for &v in &data.highest {
            if v > max_val {
                max_val = v;
            }
        }
        // Java passes yInterval=2 to the constructor
        let y_interval = 2.0;
        let max_y = (max_val / y_interval).ceil() * y_interval;
        let min_y = 0.0;

        render_quality_boxplot(&QualityBoxPlotData {
            means: data.means,
            medians: data.medians,
            lower_quartile: data.lower_quartile,
            upper_quartile: data.upper_quartile,
            lowest: data.lowest,
            highest: data.highest,
            min_y,
            max_y,
            y_interval,
            x_labels: data.x_labels,
            title,
        })
    }
}

impl QCModule for PerBaseQualityScores {
    fn process_sequence(&mut self, sequence: &Sequence) {
        let qual = &sequence.quality;

        // Grow the quality_counts array if needed
        if self.quality_counts.len() < qual.len() {
            self.quality_counts.resize_with(qual.len(), QualityCount::new);
        }

        for (i, &q) in qual.iter().enumerate() {
            self.quality_counts[i].add_value(q);
        }
    }

    fn name(&self) -> &str {
        "Per base sequence quality"
    }

    fn description(&self) -> &str {
        "Shows the Quality scores of all bases at a given position in a sequencing run"
    }

    fn reset(&mut self) {
        self.quality_counts.clear();
    }

    fn raises_error(&self) -> bool {
        let data = self.calculate();
        let lq_error = self.limits.threshold("quality_base_lower\terror", 5.0);
        let median_error = self.limits.threshold("quality_base_median\terror", 20.0);

        for i in 0..data.lower_quartile.len() {
            if data.lower_quartile[i].is_nan() {
                // Skip groups without enough data
                continue;
            }
            if data.lower_quartile[i] < lq_error || data.medians[i] < median_error {
                return true;
            }
        }
        false
    }

    fn raises_warning(&self) -> bool {
        let data = self.calculate();
        let lq_warn = self.limits.threshold("quality_base_lower\twarn", 10.0);
        let median_warn = self.limits.threshold("quality_base_median\twarn", 25.0);

        for i in 0..data.lower_quartile.len() {
            if data.lower_quartile[i].is_nan() {
                continue;
            }
            if data.lower_quartile[i] < lq_warn || data.medians[i] < median_warn {
                return true;
            }
        }
        false
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        // Ignore if configured to ignore or no quality data
        self.limits.is_ignored("quality_base") || self.quality_counts.is_empty()
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let data = self.calculate();

        // Header matches Java's makeReport output exactly
        writeln!(
            writer,
            "#Base\tMean\tMedian\tLower Quartile\tUpper Quartile\t10th Percentile\t90th Percentile"
        )?;

        for i in 0..data.means.len() {
            // Java uses StringBuffer.append(double) which calls
            // Double.toString(double) for each value
            writeln!(
                writer,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                data.x_labels[i],
                java_format_double(data.means[i]),
                java_format_double(data.medians[i]),
                java_format_double(data.lower_quartile[i]),
                java_format_double(data.upper_quartile[i]),
                java_format_double(data.lowest[i]),
                java_format_double(data.highest[i]),
            )?;
        }

        Ok(())
    }

    // Image filename matches Java's "per_base_quality.png" in Images/
    fn chart_image_name(&self) -> Option<&str> { Some("per_base_quality") }
    fn chart_alt_text(&self) -> Option<&str> { Some("Per base quality graph") }
    fn generate_chart_svg(&self) -> Option<String> { Some(self.build_chart_svg()) }
}

/// Internal struct holding calculated per-base quality data.
struct CalculatedData {
    means: Vec<f64>,
    medians: Vec<f64>,
    lower_quartile: Vec<f64>,
    upper_quartile: Vec<f64>,
    lowest: Vec<f64>,
    highest: Vec<f64>,
    x_labels: Vec<String>,
}
