// Kmer Content module
// Corresponds to Modules/KmerContent.java

use std::collections::HashMap;
use std::io;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::sequence::Sequence;
use crate::utils::base_group::BaseGroup;

/// A tracked Kmer with its total count and per-position counts.
struct Kmer {
    sequence: String,
    count: u64,
    positions: Vec<u64>,
}

impl Kmer {
    fn new(sequence: String, position: usize, seq_length: usize) -> Self {
        let mut positions = vec![0u64; seq_length];
        positions[position] = 1;
        Kmer {
            sequence,
            count: 1,
            positions,
        }
    }

    fn increment_count(&mut self, position: usize) {
        self.count += 1;
        // Expand positions array if needed
        if position >= self.positions.len() {
            self.positions.resize(position + 1, 0);
        }
        self.positions[position] += 1;
    }
}

pub struct KmerContent {
    kmers: HashMap<String, Kmer>,
    longest_sequence: usize,
    /// 2D array - totalKmerCounts[position][kmer_length_index]
    /// kmer_length_index = kmer_size - 1 (only one entry when min==max kmer size)
    total_kmer_counts: Vec<Vec<u64>>,
    skip_count: u64,
    kmer_size: usize,
    limits: Limits,
    nogroup: bool,
    expgroup: bool,
    // Lazily computed
    computed: Option<ComputedKmerResults>,
}

struct ComputedKmerResults {
    enriched_kmers: Vec<EnrichedKmer>,
    groups: Vec<BaseGroup>,
}

/// A post-calculation enriched kmer result for reporting.
struct EnrichedKmer {
    sequence: String,
    /// count * 5 is reported (because 2% sampling = every 50th read, then * 5??)
    /// Actually the Java code reports count*5 in getValueAt for the Count column
    count: u64,
    p_value: f32,
    max_obs_exp: f32,
    max_position_group: String,
    /// Per-group obs/exp values for chart rendering.
    /// Java stores these raw (not log2 transformed) even though the chart Y-axis says "Log2 Obs/Exp".
    obs_exp_per_group: Vec<f32>,
}

impl KmerContent {
    pub fn new(limits: &Limits, kmer_size: u8, nogroup: bool, expgroup: bool) -> Self {
        let ks = kmer_size as usize;
        KmerContent {
            kmers: HashMap::with_capacity(4usize.pow(ks as u32)),
            longest_sequence: 0,
            total_kmer_counts: Vec::new(),
            skip_count: 0,
            kmer_size: ks,
            limits: limits.clone(),
            nogroup,
            expgroup,
            computed: None,
        }
    }

    /// Replicates addKmerCount() - track total kmer counts per position.
    /// Only counts if the kmer doesn't contain N.
    /// Returns true if the kmer contains N (caller can skip further processing).
    fn add_kmer_count(&mut self, position: usize, kmer_length: usize, kmer: &str) -> bool {
        if position >= self.total_kmer_counts.len() {
            // Expand array, new entries get a vec of size MAX_KMER_SIZE
            let old_len = self.total_kmer_counts.len();
            self.total_kmer_counts
                .resize_with(position + 1, || vec![0u64; self.kmer_size]);
            // Ensure old entries have correct length too (shouldn't be needed but safe)
            for i in old_len..self.total_kmer_counts.len() {
                if self.total_kmer_counts[i].len() < self.kmer_size {
                    self.total_kmer_counts[i].resize(self.kmer_size, 0);
                }
            }
        }

        // Only count if kmer doesn't contain N
        if kmer.contains('N') {
            return true;
        }

        // kmer_length - 1 is the index (when min==max, always 0 offset from min)
        self.total_kmer_counts[position][kmer_length - 1] += 1;
        false
    }

    /// Replicates calculateEnrichment() from KmerContent.java.
    fn calculate_enrichment(&mut self) {
        if self.computed.is_some() {
            return;
        }

        // Group positions for (longestSequence - MIN_KMER_SIZE) + 1
        let group_length = if self.longest_sequence >= self.kmer_size {
            (self.longest_sequence - self.kmer_size) + 1
        } else {
            0
        };

        let groups = BaseGroup::make_base_groups(group_length, self.nogroup, self.expgroup);

        let mut uneven_kmers: Vec<(String, u64, f32, Vec<f32>, f32)> = Vec::new();

        for kmer in self.kmers.values() {
            let kmer_len = kmer.sequence.len();

            // Total count of all kmers of this length across all positions
            let mut total_kmer_count: u64 = 0;
            for pos_counts in &self.total_kmer_counts {
                if kmer_len - 1 < pos_counts.len() {
                    total_kmer_count += pos_counts[kmer_len - 1];
                }
            }

            if total_kmer_count == 0 {
                continue;
            }

            // Expected proportion of this specific kmer
            let expected_proportion = kmer.count as f32 / total_kmer_count as f32;

            let mut obs_exp_positions = vec![0.0f32; groups.len()];
            let mut binomial_p_values = vec![1.0f32; groups.len()];

            for (g, group) in groups.iter().enumerate() {
                let mut total_group_count: u64 = 0;
                let mut total_group_hits: u64 = 0;

                // Sum counts in this base group
                let lower = group.lower_count; // 0-based
                let upper = group.upper_count; // 0-based, inclusive

                for p in lower..=upper {
                    if p < self.total_kmer_counts.len()
                        && kmer_len - 1 < self.total_kmer_counts[p].len()
                    {
                        total_group_count += self.total_kmer_counts[p][kmer_len - 1];
                    }
                    if p < kmer.positions.len() {
                        total_group_hits += kmer.positions[p];
                    }
                }

                let predicted = expected_proportion * total_group_count as f32;
                // obs/exp ratio (not log2 transformed for the filter)
                if predicted > 0.0 {
                    obs_exp_positions[g] = total_group_hits as f32 / predicted;
                }

                // Binomial test with Bonferroni correction (4^k)
                if total_group_hits as f32 > predicted && total_group_count > 0 {
                    // Use the statrs binomial distribution for the p-value calculation
                    let p_val = binomial_p_value(
                        total_group_count,
                        expected_proportion as f64,
                        total_group_hits,
                    );
                    binomial_p_values[g] = (p_val * 4.0f64.powi(kmer_len as i32)) as f32;
                }
            }

            // Keep if any position has p<0.01 AND obs/exp>5
            let mut lowest_p_value: f32 = 1.0;
            for i in 0..binomial_p_values.len() {
                if binomial_p_values[i] < 0.01
                    && obs_exp_positions[i] > 5.0
                    && binomial_p_values[i] < lowest_p_value
                {
                    lowest_p_value = binomial_p_values[i];
                }
            }

            if lowest_p_value < 0.01 {
                uneven_kmers.push((
                    kmer.sequence.clone(),
                    kmer.count,
                    lowest_p_value,
                    obs_exp_positions,
                    0.0, // max_obs_exp calculated below
                ));
            }
        }

        // Calculate max obs/exp and sort by it descending
        for entry in &mut uneven_kmers {
            entry.4 = entry.3.iter().cloned().fold(0.0f32, f32::max);
        }
        // Sort by highest obs/exp ratio
        uneven_kmers.sort_by(|a, b| b.4.partial_cmp(&a.4).unwrap_or(std::cmp::Ordering::Equal));

        // Only report top 20
        uneven_kmers.truncate(20);

        let enriched_kmers: Vec<EnrichedKmer> = uneven_kmers
            .iter()
            .map(|(seq, count, p_value, obs_exp, max_oe)| {
                // Find max position (1-based index into groups)
                let mut max_pos = 0;
                let mut max_val = 0.0f32;
                for (i, &v) in obs_exp.iter().enumerate() {
                    if v > max_val {
                        max_val = v;
                        max_pos = i;
                    }
                }
                let max_position_group = if !groups.is_empty() {
                    groups[max_pos].label()
                } else {
                    String::new()
                };
                EnrichedKmer {
                    sequence: seq.clone(),
                    // count*5 because 2% sampling (every 50th read)
                    count: *count * 5,
                    p_value: *p_value,
                    max_obs_exp: *max_oe,
                    max_position_group,
                    obs_exp_per_group: obs_exp.clone(),
                }
            })
            .collect();

        self.computed = Some(ComputedKmerResults {
            enriched_kmers,
            groups,
        });
    }

    fn ensure_calculated(&self) -> &ComputedKmerResults {
        static DEFAULT: ComputedKmerResults = ComputedKmerResults {
            enriched_kmers: Vec::new(),
            groups: Vec::new(),
        };
        self.computed.as_ref().unwrap_or(&DEFAULT)
    }
}

/// Calculate binomial p-value: P(X > k) = 1 - CDF(k) for Binomial(n, p).
/// Uses the statrs crate for the binomial CDF.
fn binomial_p_value(n: u64, p: f64, k: u64) -> f64 {
    use statrs::distribution::Binomial;
    use statrs::distribution::DiscreteCDF;

    if n == 0 || p <= 0.0 || p >= 1.0 {
        return 1.0;
    }

    match Binomial::new(p, n) {
        Ok(binom) => {
            // P(X > k) = 1 - P(X <= k) = 1 - CDF(k)
            1.0 - binom.cdf(k)
        }
        Err(_) => 1.0,
    }
}

impl KmerContent {
    /// Build the SVG chart showing obs/exp ratios for top enriched kmers.
    ///
    /// Java's makeReport() creates a LineGraph with the top 6 enriched kmers'
    /// obs/exp values per position group. The Y-axis label is "Log2 Obs/Exp" even though
    /// the values are raw obs/exp ratios (not log2 transformed) -- this is a quirk in Java.
    fn build_chart_svg(&self) -> Option<String> {
        let computed = self.computed.as_ref()?;

        if computed.enriched_kmers.is_empty() {
            return None;
        }

        // Only plot top 6 enriched kmers on the chart
        let num_series = computed.enriched_kmers.len().min(6);

        let x_categories: Vec<String> = computed.groups.iter().map(|g| g.label()).collect();

        let mut data: Vec<Vec<f64>> = Vec::with_capacity(num_series);
        let mut series_names: Vec<String> = Vec::with_capacity(num_series);
        let mut max_y: f64 = 0.0;

        for k in 0..num_series {
            let kmer = &computed.enriched_kmers[k];
            let values: Vec<f64> = kmer.obs_exp_per_group.iter().map(|&v| v as f64).collect();
            for &v in &values {
                if v > max_y {
                    max_y = v;
                }
            }
            data.push(values);
            series_names.push(kmer.sequence.clone());
        }

        // minGraphValue is forced to 0
        let min_y = 0.0;
        // Ensure max_y is at least 1 to avoid degenerate axis
        if max_y < 1.0 {
            max_y = 1.0;
        }

        Some(render_line_graph(&LineGraphData {
            data,
            min_y,
            max_y,
            x_label: "Position in read (bp)".to_string(),
            series_names,
            x_categories,
            // Title says "Log2 Obs/Exp" even though values are raw obs/exp ratios
            title: "Log2 Obs/Exp".to_string(),
        }))
    }
}

impl QCModule for KmerContent {
    fn process_sequence(&mut self, sequence: &Sequence) {
        self.computed = None;

        // Only sample 2% of reads (every 50th)
        self.skip_count += 1;
        if !self.skip_count.is_multiple_of(50) {
            return;
        }

        // Limit read length to 500bp to avoid memory issues
        let seq_str = std::str::from_utf8(&sequence.sequence).unwrap_or("");
        let seq = if seq_str.len() > 500 {
            &seq_str[..500]
        } else {
            seq_str
        };

        if seq.len() > self.longest_sequence {
            self.longest_sequence = seq.len();
        }

        let kmer_size = self.kmer_size;

        // Iterate over all kmers (only one size when min==max)
        if seq.len() >= kmer_size {
            for i in 0..=(seq.len() - kmer_size) {
                let kmer = &seq[i..i + kmer_size];

                // Always add to total counts (even if contains N).
                // add_kmer_count returns true if kmer contains N, so we can skip
                // the HashMap lookup without scanning for 'N' a second time.
                if self.add_kmer_count(i, kmer_size, kmer) {
                    continue;
                }

                if let Some(existing) = self.kmers.get_mut(kmer) {
                    existing.increment_count(i);
                } else {
                    let seq_kmer_length = (seq.len() - kmer_size) + 1;
                    self.kmers.insert(
                        kmer.to_string(),
                        Kmer::new(kmer.to_string(), i, seq_kmer_length),
                    );
                }
            }
        }
    }

    fn finalize(&mut self) {
        self.calculate_enrichment();
    }

    fn name(&self) -> &str {
        "Kmer Content"
    }

    fn description(&self) -> &str {
        "Identifies short sequences which have uneven representation"
    }

    fn reset(&mut self) {
        self.kmers.clear();
        self.total_kmer_counts.clear();
        self.longest_sequence = 0;
        self.skip_count = 0;
        self.computed = None;
    }

    fn raises_error(&self) -> bool {
        let threshold = self.limits.threshold("kmer\terror", 5.0);
        let computed = self.ensure_calculated();
        // Error if -log10(pvalue) of most enriched kmer exceeds threshold
        computed
            .enriched_kmers
            .first()
            .is_some_and(|k| -(k.p_value as f64).log10() > threshold)
    }

    fn raises_warning(&self) -> bool {
        let threshold = self.limits.threshold("kmer\twarn", 2.0);
        let computed = self.ensure_calculated();
        // Warning if -log10(pvalue) of most enriched kmer exceeds threshold
        computed
            .enriched_kmers
            .first()
            .is_some_and(|k| -(k.p_value as f64).log10() > threshold)
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        // Default is ignore=1 (ignored by default)
        self.limits.threshold("kmer\tignore", 1.0) > 0.0
    }

    // Image filename matches Java's "kmer_profiles.png" in Images/
    fn chart_image_name(&self) -> Option<&str> {
        Some("kmer_profiles")
    }
    fn chart_alt_text(&self) -> Option<&str> {
        Some("Kmer graph")
    }
    fn generate_chart_svg(&self) -> Option<String> {
        self.build_chart_svg()
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let computed = self.ensure_calculated();

        // Table header
        writeln!(
            writer,
            "#Sequence\tCount\tPValue\tObs/Exp Max\tMax Obs/Exp Position"
        )?;

        for kmer in &computed.enriched_kmers {
            writeln!(
                writer,
                "{}\t{}\t{}\t{}\t{}",
                kmer.sequence, kmer.count, kmer.p_value, kmer.max_obs_exp, kmer.max_position_group
            )?;
        }

        Ok(())
    }
}
