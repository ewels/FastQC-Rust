// Per Base Sequence Content module
// Corresponds to Modules/PerBaseSequenceContent.java

use std::io;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::sequence::Sequence;
use crate::utils::base_counts::{BASE_INDEX, IDX_A, IDX_C, IDX_G, IDX_T};
use crate::utils::base_group::BaseGroup;
use crate::utils::format::java_format_double;

pub struct PerBaseSequenceContent {
    /// Per-position base counts stored as [A, C, G, T] per position.
    /// Using a single Vec of arrays gives better cache locality than
    /// four separate Vecs, since all counts for a position are adjacent.
    counts: Vec<[u64; 4]>,
    nogroup: bool,
    expgroup: bool,
    min_length: usize,
    limits: Limits,
}

impl PerBaseSequenceContent {
    pub fn new(limits: &Limits, nogroup: bool, expgroup: bool, min_length: usize) -> Self {
        PerBaseSequenceContent {
            counts: Vec::new(),
            nogroup,
            expgroup,
            min_length,
            limits: limits.clone(),
        }
    }

    fn calculate(&self) -> ContentData {
        let groups = BaseGroup::make_base_groups(
            self.counts.len(),
            self.min_length,
            self.nogroup,
            self.expgroup,
        );

        let mut x_categories = Vec::with_capacity(groups.len());
        let mut g_percent = vec![0.0f64; groups.len()];
        let mut a_percent = vec![0.0f64; groups.len()];
        let mut t_percent = vec![0.0f64; groups.len()];
        let mut c_percent = vec![0.0f64; groups.len()];

        for (i, group) in groups.iter().enumerate() {
            x_categories.push(group.label());

            let mut a_count: u64 = 0;
            let mut c_count: u64 = 0;
            let mut g_count: u64 = 0;
            let mut t_count: u64 = 0;
            let mut total: u64 = 0;

            // Java iterates `for (int bp=groups[i].lowerCount()-1;bp<groups[i].upperCount();bp++)`
            // which is 0-based lowerCount-1 to upperCount-1 inclusive. Our lower_count/upper_count
            // are already 0-based.
            for bp in group.lower_count..=group.upper_count {
                let c = &self.counts[bp];
                a_count += c[IDX_A];
                c_count += c[IDX_C];
                g_count += c[IDX_G];
                t_count += c[IDX_T];
                total += c[IDX_A] + c[IDX_C] + c[IDX_G] + c[IDX_T];
            }

            g_percent[i] = (g_count as f64 / total as f64) * 100.0;
            a_percent[i] = (a_count as f64 / total as f64) * 100.0;
            t_percent[i] = (t_count as f64 / total as f64) * 100.0;
            c_percent[i] = (c_count as f64 / total as f64) * 100.0;
        }

        // percentages stored in order [T, C, A, G] matching Java's array layout
        ContentData {
            x_categories,
            t_percent,
            c_percent,
            a_percent,
            g_percent,
        }
    }
}

impl PerBaseSequenceContent {
    fn build_chart_svg(&self) -> String {
        let data = self.calculate();

        // Series order in LineGraph is [%T, %C, %A, %G], matching Java's
        // `new LineGraph(percentages, 0d, 100d, ..., new String[] {"%T","%C","%A","%G"}, ...)`
        render_line_graph(&LineGraphData {
            data: vec![
                data.t_percent,
                data.c_percent,
                data.a_percent,
                data.g_percent,
            ],
            min_y: 0.0,
            max_y: 100.0,
            x_label: "Position in read (bp)".to_string(),
            series_names: vec![
                "%T".to_string(),
                "%C".to_string(),
                "%A".to_string(),
                "%G".to_string(),
            ],
            x_categories: data.x_categories,
            title: "Sequence content across all bases".to_string(),
        })
    }
}

impl QCModule for PerBaseSequenceContent {
    fn process_sequence(&mut self, sequence: &Sequence) {
        let seq = &sequence.sequence;

        // Grow array if needed
        if self.counts.len() < seq.len() {
            self.counts.resize(seq.len(), [0; 4]);
        }

        // Use lookup table for branchless base classification.
        // Only A/C/G/T (indices 0-3) are counted; N and others (indices 4-5)
        // are filtered by the single comparison `idx < 4`.
        for (i, &b) in seq.iter().enumerate() {
            let idx = BASE_INDEX[b as usize] as usize;
            if idx < 4 {
                self.counts[i][idx] += 1;
            }
        }
    }

    fn name(&self) -> &str {
        "Per base sequence content"
    }

    fn description(&self) -> &str {
        "Shows the relative amounts of each base at each position in a sequencing run"
    }

    fn reset(&mut self) {
        self.counts.clear();
    }

    fn raises_error(&self) -> bool {
        let error_threshold = self.limits.threshold("sequence\terror", 20.0);
        let data = self.calculate();

        // Check GC diff (C vs G) and AT diff (T vs A) per group
        for i in 0..data.g_percent.len() {
            let gc_diff = (data.c_percent[i] - data.g_percent[i]).abs();
            let at_diff = (data.t_percent[i] - data.a_percent[i]).abs();

            if gc_diff > error_threshold || at_diff > error_threshold {
                return true;
            }
        }
        false
    }

    fn raises_warning(&self) -> bool {
        let warn_threshold = self.limits.threshold("sequence\twarn", 10.0);
        let data = self.calculate();

        for i in 0..data.g_percent.len() {
            let gc_diff = (data.c_percent[i] - data.g_percent[i]).abs();
            let at_diff = (data.t_percent[i] - data.a_percent[i]).abs();

            if gc_diff > warn_threshold || at_diff > warn_threshold {
                return true;
            }
        }
        false
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        self.limits.is_ignored("sequence")
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let data = self.calculate();

        // Column order is G, A, T, C matching Java's makeReport
        writeln!(writer, "#Base\tG\tA\tT\tC")?;

        for i in 0..data.x_categories.len() {
            // Java outputs percentages[3][i] (G), [2][i] (A), [0][i] (T), [1][i] (C)
            writeln!(
                writer,
                "{}\t{}\t{}\t{}\t{}",
                data.x_categories[i],
                java_format_double(data.g_percent[i]),
                java_format_double(data.a_percent[i]),
                java_format_double(data.t_percent[i]),
                java_format_double(data.c_percent[i]),
            )?;
        }

        Ok(())
    }

    // Image filename matches Java's "per_base_sequence_content.png" in Images/
    fn chart_image_name(&self) -> Option<&str> {
        Some("per_base_sequence_content")
    }
    fn chart_alt_text(&self) -> Option<&str> {
        Some("Per base sequence content")
    }
    fn generate_chart_svg(&self) -> Option<String> {
        Some(self.build_chart_svg())
    }
}

struct ContentData {
    x_categories: Vec<String>,
    t_percent: Vec<f64>,
    c_percent: Vec<f64>,
    a_percent: Vec<f64>,
    g_percent: Vec<f64>,
}
