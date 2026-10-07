// Sequence Duplication Levels module
// Corresponds to Modules/DuplicationLevel.java

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use crate::config::{Limits, LimitsExt};
use crate::modules::overrepresented_seqs::OverRepresentedData;
use crate::modules::QCModule;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::report::charts::CHART_WIDTH;
use crate::sequence::Sequence;
use crate::utils::format::java_format_double;

pub struct DuplicationLevel {
    shared_data: Arc<Mutex<OverRepresentedData>>,
    limits: Limits,
    // Lazily computed
    computed: Option<ComputedLevels>,
}

struct ComputedLevels {
    total_percentages: [f64; 16],
    percent_different_seqs: f64,
}

/// The 16 duplication level labels, matching DuplicationLevel.java
const LABELS: [&str; 16] = [
    "1", "2", "3", "4", "5", "6", "7", "8", "9", ">10", ">50", ">100", ">500", ">1k", ">5k", ">10k",
];

impl DuplicationLevel {
    pub fn new(shared_data: Arc<Mutex<OverRepresentedData>>, limits: &Limits) -> Self {
        DuplicationLevel {
            shared_data,
            limits: limits.clone(),
            computed: None,
        }
    }

    /// Replicates calculateLevels() from DuplicationLevel.java.
    /// Must be called after all sequences have been processed.
    pub fn calculate_levels(&mut self) {
        if self.computed.is_some() {
            return;
        }

        let data = self.shared_data.lock().unwrap_or_else(|e| e.into_inner());

        let mut total_percentages = [0.0f64; 16];

        // Collate how many unique sequences have each duplication count
        let mut collated_counts: HashMap<u64, u64> = HashMap::new();
        for &count in data.sequences.values() {
            *collated_counts.entry(count).or_insert(0) += 1;
        }

        // Apply statistical correction to each duplication level
        let mut corrected_counts: Vec<(u64, f64)> = collated_counts
            .into_iter()
            .map(|(dup_level, num_observations)| {
                let corrected = get_corrected_count(
                    data.count_at_unique_limit,
                    data.count,
                    dup_level,
                    num_observations,
                );
                (dup_level, corrected)
            })
            .collect();
        sort_in_java_hashmap_order(&mut corrected_counts);

        // Calculate raw and deduplicated totals from corrected counts
        let mut dedup_total: f64 = 0.0;
        let mut raw_total: f64 = 0.0;

        for &(dup_level, count) in &corrected_counts {
            dedup_total += count;
            raw_total += count * dup_level as f64;

            // Map duplication level to one of 16 bins
            let temp_dup_slot = dup_level as i64 - 1;

            // The dupSlot < 0 is a kludge to handle duplication levels > 2^31
            let dup_slot: usize = if !(0..=9999).contains(&temp_dup_slot) {
                15
            } else if temp_dup_slot > 4999 {
                14
            } else if temp_dup_slot > 999 {
                13
            } else if temp_dup_slot > 499 {
                12
            } else if temp_dup_slot > 99 {
                11
            } else if temp_dup_slot > 49 {
                10
            } else if temp_dup_slot > 9 {
                9
            } else {
                temp_dup_slot as usize
            };

            total_percentages[dup_slot] += count * dup_level as f64;
        }

        // JAVA COMPAT: no zero guard, so empty input gives NaN like Java.
        for tp in &mut total_percentages {
            *tp = *tp / raw_total * 100.0;
        }

        // percentDifferentSeqs = (dedupTotal/rawTotal)*100
        let percent_different_seqs = if raw_total == 0.0 {
            100.0
        } else {
            (dedup_total / raw_total) * 100.0
        };

        self.computed = Some(ComputedLevels {
            total_percentages,
            percent_different_seqs,
        });
    }

    fn ensure_calculated(&self) -> &ComputedLevels {
        // SAFETY: finalize() must be called before any reporting method.
        // If a caller skips finalize(), we provide a static default to avoid panicking.
        static DEFAULT: ComputedLevels = ComputedLevels {
            total_percentages: [0.0; 16],
            percent_different_seqs: 100.0,
        };
        self.computed.as_ref().unwrap_or(&DEFAULT)
    }
}

/// JAVA COMPAT: Java sums corrected counts in `HashMap<Long, Double>` order, and
/// float addition is order-sensitive. That order is hash bucket (16 slots, doubling
/// past 0.75 load), then insertion order within a bucket, approximated here by key.
fn sort_in_java_hashmap_order(entries: &mut [(u64, f64)]) {
    let mut capacity = 16;
    while entries.len() * 4 > capacity * 3 {
        capacity *= 2;
    }
    entries.sort_unstable_by_key(|&(key, _)| {
        let h = (key ^ (key >> 32)) as u32;
        (((h ^ (h >> 16)) as usize) & (capacity - 1), key)
    });
}

/// Replicates getCorrectedCount() from DuplicationLevel.java.
/// Corrects observed counts to account for sequences that might have been missed
/// because we stopped adding new unique sequences after the observation cutoff.
fn get_corrected_count(
    count_at_limit: u64,
    total_count: u64,
    duplication_level: u64,
    number_of_observations: u64,
) -> f64 {
    // Bail out early if we saw all sequences
    if count_at_limit == total_count {
        return number_of_observations as f64;
    }

    // If not enough sequences left to hide another sequence with this count
    if total_count - number_of_observations < count_at_limit {
        return number_of_observations as f64;
    }

    // Calculate probability of NOT seeing a sequence with this duplication
    // level within the first countAtLimit sequences
    let mut p_not_seeing_at_limit: f64 = 1.0;

    // Probability below which we stop caring (won't change count by 0.01)
    let limit_of_caring =
        1.0 - (number_of_observations as f64 / (number_of_observations as f64 + 0.01));

    for i in 0..count_at_limit {
        p_not_seeing_at_limit *=
            ((total_count - i) - duplication_level) as f64 / (total_count - i) as f64;

        if p_not_seeing_at_limit < limit_of_caring {
            p_not_seeing_at_limit = 0.0;
            break;
        }
    }

    // Invert to get chance of seeing, then scale up
    let p_seeing_at_limit = 1.0 - p_not_seeing_at_limit;
    number_of_observations as f64 / p_seeing_at_limit
}

impl DuplicationLevel {
    fn build_chart_svg(&self) -> String {
        let computed = self.ensure_calculated();
        // Java uses fixed maxCount=100 (percentage scale 0-100%), not dynamic
        let max_count = 100.0_f64;

        let labels: Vec<String> = LABELS.iter().map(|&l| l.to_string()).collect();

        // Title uses Java's DecimalFormat("#.##") — up to 2 decimals, trailing zeros stripped
        let pct_str = format!("{:.2}", computed.percent_different_seqs);
        let pct_str = pct_str.trim_end_matches('0').trim_end_matches('.');
        let title = format!("Percent of seqs remaining if deduplicated {}%", pct_str);

        render_line_graph(&LineGraphData {
            width: CHART_WIDTH,
            data: vec![computed.total_percentages.to_vec()],
            min_y: 0.0,
            max_y: max_count,
            x_label: "Sequence Duplication Level".to_string(),
            series_names: vec!["% Total sequences".to_string()],
            x_categories: labels,
            title,
        })
    }
}

impl QCModule for DuplicationLevel {
    fn process_sequence(&mut self, _sequence: &Sequence) {
        // DuplicationLevel doesn't process sequences itself;
        // it uses the shared data from OverRepresentedSeqs.
    }

    fn finalize(&mut self) {
        self.calculate_levels();
    }

    fn name(&self) -> &str {
        "Sequence Duplication Levels"
    }

    fn description(&self) -> &str {
        "Plots the number of sequences which are duplicated to different levels"
    }

    fn reset(&mut self) {
        self.computed = None;
    }

    fn raises_error(&self) -> bool {
        let threshold = self.limits.threshold("duplication\terror", 50.0);
        let computed = self.ensure_calculated();
        // Error if percent different seqs is BELOW the threshold
        computed.percent_different_seqs < threshold
    }

    fn raises_warning(&self) -> bool {
        let threshold = self.limits.threshold("duplication\twarn", 70.0);
        let computed = self.ensure_calculated();
        // Warning if percent different seqs is BELOW the threshold
        computed.percent_different_seqs < threshold
    }

    fn ignore_filtered_sequences(&self) -> bool {
        // The ignoreFilteredSequences in Java checks the ignore config
        self.limits.is_ignored("duplication")
    }

    fn ignore_in_report(&self) -> bool {
        self.limits.is_ignored("duplication")
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let computed = self.ensure_calculated();

        // First line is the total deduplicated percentage
        writeln!(
            writer,
            "#Total Deduplicated Percentage\t{}",
            java_format_double(computed.percent_different_seqs)
        )?;
        // Header for the duplication level data
        writeln!(writer, "#Duplication Level\tPercentage of total")?;

        // Iterate labels and percentages together
        for (i, (label_str, tp)) in LABELS
            .iter()
            .zip(computed.total_percentages.iter())
            .enumerate()
        {
            let label = if i == 15 {
                format!("{}+", label_str)
            } else {
                label_str.to_string()
            };
            writeln!(writer, "{}\t{}", label, java_format_double(*tp))?;
        }

        Ok(())
    }

    // Image filename matches Java's "duplication_levels.png" in Images/
    fn chart_image_name(&self) -> Option<&str> {
        Some("duplication_levels")
    }
    fn chart_alt_text(&self) -> Option<&str> {
        Some("Duplication level graph")
    }
    fn generate_chart_svg(&self) -> Option<String> {
        Some(self.build_chart_svg())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_java_hashmap_order() {
        // 12 keys fit 16 slots, so 32 and 17 wrap to buckets 0 and 1.
        let mut entries: Vec<(u64, f64)> = [32, 3, 17, 1, 2, 4, 5, 6, 7, 8, 9, 10]
            .iter()
            .map(|&k| (k, 0.0))
            .collect();
        sort_in_java_hashmap_order(&mut entries);
        let keys: Vec<u64> = entries.iter().map(|&(k, _)| k).collect();
        assert_eq!(keys, [32, 1, 17, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

        // A 13th key grows the table to 32 slots.
        let mut entries: Vec<(u64, f64)> = (1..=12).chain([40]).map(|k| (k, 0.0)).collect();
        sort_in_java_hashmap_order(&mut entries);
        let keys: Vec<u64> = entries.iter().map(|&(k, _)| k).collect();
        assert_eq!(keys, [1, 2, 3, 4, 5, 6, 7, 8, 40, 9, 10, 11, 12]);
    }
}
