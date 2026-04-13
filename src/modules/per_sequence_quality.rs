// Per Sequence Quality Scores module
// Corresponds to Modules/PerSequenceQualityScores.java

use std::io;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::sequence::Sequence;
use crate::utils::format::java_format_double;
use crate::utils::phred;

/// Maximum raw ASCII average quality score we can track.
/// Quality chars are in range 0-127; the average of any sequence of such chars
/// fits in the same range. 128 slots covers all possible values.
const MAX_QUALITY_SCORE: usize = 128;

pub struct PerSequenceQualityScores {
    // JAVA COMPAT: Indexed by raw ASCII average quality (before offset subtraction),
    // computed using integer arithmetic: sum of quality chars / length.
    // Using a fixed array instead of HashMap eliminates hashing on every read.
    average_score_counts: [u64; MAX_QUALITY_SCORE],
    has_data: bool,
    lowest_char: u8,
    limits: Limits,
}

impl PerSequenceQualityScores {
    pub fn new(limits: &Limits) -> Self {
        PerSequenceQualityScores {
            average_score_counts: [0u64; MAX_QUALITY_SCORE],
            has_data: false,
            // Java initialises lowestChar to 126
            lowest_char: 126,
            limits: limits.clone(),
        }
    }

    fn calculate(&self) -> Option<DistributionData> {
        if !self.has_data {
            return None;
        }

        let encoding = phred::detect(self.lowest_char).unwrap();

        // Find the range of scores with non-zero counts
        let mut range_start: Option<usize> = None;
        let mut range_end: usize = 0;
        for i in 0..MAX_QUALITY_SCORE {
            if self.average_score_counts[i] > 0 {
                if range_start.is_none() {
                    range_start = Some(i);
                }
                range_end = i;
            }
        }

        let range_start = range_start? as i32;
        let range_end = range_end as i32;

        // Distribution runs from lowest to highest raw score
        let len = (1 + range_end - range_start) as usize;

        let mut quality_distribution = vec![0.0f64; len];
        let mut x_categories = Vec::with_capacity(len);

        // Build distribution and x_categories arrays in parallel
        for (i, qd) in quality_distribution.iter_mut().enumerate() {
            x_categories.push(range_start + i as i32 - encoding.offset as i32);
            let key = (range_start + i as i32) as usize;
            *qd = self.average_score_counts[key] as f64;
        }

        // Find most frequent score
        let mut max_count = 0.0;
        let mut most_frequent_score = 0;
        // index needed for both quality_distribution and x_categories
        for (&qd, &xc) in quality_distribution.iter().zip(x_categories.iter()) {
            if qd > max_count {
                max_count = qd;
                most_frequent_score = xc;
            }
        }

        Some(DistributionData {
            quality_distribution,
            x_categories,
            most_frequent_score,
        })
    }
}

impl PerSequenceQualityScores {
    fn build_chart_svg(&self) -> Option<String> {
        let data = self.calculate()?;

        let max_count = data
            .quality_distribution
            .iter()
            .cloned()
            .fold(0.0_f64, f64::max);
        // Java passes raw max to LineGraph (no ceil rounding)
        let max_y = max_count;

        let x_categories: Vec<String> =
            data.x_categories.iter().map(|v| format!("{}", v)).collect();

        Some(render_line_graph(&LineGraphData {
            data: vec![data.quality_distribution],
            min_y: 0.0,
            max_y,
            // These labels match the Java constructor call
            x_label: "Mean Sequence Quality (Phred Score)".to_string(),
            series_names: vec!["Average Quality per read".to_string()],
            x_categories,
            title: "Quality score distribution over all sequences".to_string(),
        }))
    }
}

impl QCModule for PerSequenceQualityScores {
    fn process_sequence(&mut self, sequence: &Sequence) {
        let qual = &sequence.quality;

        // JAVA COMPAT: Average quality computed using integer arithmetic on raw ASCII values.
        // sum of quality chars (as int) / length (integer division), stored as raw value.
        let mut average_quality: i32 = 0;

        for &q in qual.iter() {
            if q < self.lowest_char {
                self.lowest_char = q;
            }
            average_quality += q as i32;
        }

        if !qual.is_empty() {
            // JAVA COMPAT: Integer division truncates towards zero, matching Java's `/`
            average_quality /= qual.len() as i32;

            self.average_score_counts[average_quality as usize] += 1;
            self.has_data = true;
        }
    }

    fn name(&self) -> &str {
        "Per sequence quality scores"
    }

    fn description(&self) -> &str {
        "Shows the distribution of average quality scores for whole sequences"
    }

    fn reset(&mut self) {
        self.average_score_counts = [0u64; MAX_QUALITY_SCORE];
        self.has_data = false;
        self.lowest_char = 126;
    }

    fn raises_error(&self) -> bool {
        let error_threshold = self.limits.threshold("quality_sequence\terror", 20.0);
        // Error if most frequent quality score <= threshold
        self.calculate()
            .is_some_and(|data| (data.most_frequent_score as f64) <= error_threshold)
    }

    fn raises_warning(&self) -> bool {
        let warn_threshold = self.limits.threshold("quality_sequence\twarn", 27.0);
        self.calculate()
            .is_some_and(|data| (data.most_frequent_score as f64) <= warn_threshold)
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        self.limits.is_ignored("quality_sequence") || !self.has_data
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let data = match self.calculate() {
            Some(d) => d,
            None => return Ok(()),
        };

        // Header format matches Java's makeReport
        writeln!(writer, "#Quality\tCount")?;

        for i in 0..data.x_categories.len() {
            writeln!(
                writer,
                "{}\t{}",
                data.x_categories[i],
                java_format_double(data.quality_distribution[i]),
            )?;
        }

        Ok(())
    }

    // Image filename matches Java's "per_sequence_quality.png" in Images/
    fn chart_image_name(&self) -> Option<&str> {
        Some("per_sequence_quality")
    }
    fn chart_alt_text(&self) -> Option<&str> {
        Some("Per Sequence quality graph")
    }
    fn generate_chart_svg(&self) -> Option<String> {
        self.build_chart_svg()
    }
}

struct DistributionData {
    quality_distribution: Vec<f64>,
    x_categories: Vec<i32>,
    most_frequent_score: i32,
}
