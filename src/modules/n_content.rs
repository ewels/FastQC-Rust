// Per Base N Content module
// Corresponds to Modules/NContent.java

use std::io;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::sequence::Sequence;
use crate::utils::base_counts::{BASE_INDEX, IDX_N};
use crate::utils::base_group::BaseGroup;
use crate::utils::format::java_format_double;

pub struct NContent {
    n_counts: Vec<u64>,
    not_n_counts: Vec<u64>,
    nogroup: bool,
    expgroup: bool,
    min_length: usize,
    limits: Limits,
}

impl NContent {
    pub fn new(limits: &Limits, nogroup: bool, expgroup: bool, min_length: usize) -> Self {
        NContent {
            n_counts: Vec::new(),
            not_n_counts: Vec::new(),
            nogroup,
            expgroup,
            min_length,
            limits: limits.clone(),
        }
    }

    fn calculate(&self) -> NContentData {
        let groups = BaseGroup::make_base_groups(
            self.n_counts.len(),
            self.min_length,
            self.nogroup,
            self.expgroup,
        );

        let mut x_categories = Vec::with_capacity(groups.len());
        let mut percentages = vec![0.0f64; groups.len()];

        for (i, group) in groups.iter().enumerate() {
            x_categories.push(group.label());

            let mut n_count: u64 = 0;
            let mut total: u64 = 0;

            // Java iterates `for (int bp=groups[i].lowerCount()-1;bp<groups[i].upperCount();bp++)`
            // Our lower_count/upper_count are 0-based.
            for bp in group.lower_count..=group.upper_count {
                n_count += self.n_counts[bp];
                total += self.n_counts[bp];
                total += self.not_n_counts[bp];
            }

            // percentages[i] = 100 * (nCount / (double)total)
            percentages[i] = 100.0 * (n_count as f64 / total as f64);
        }

        NContentData {
            x_categories,
            percentages,
        }
    }
}

impl NContent {
    fn build_chart_svg(&self) -> String {
        let data = self.calculate();

        // minY=0, maxY=100 for percentage, matching Java's constructor
        render_line_graph(&LineGraphData {
            data: vec![data.percentages],
            min_y: 0.0,
            max_y: 100.0,
            x_label: "Position in read (bp)".to_string(),
            series_names: vec!["%N".to_string()],
            x_categories: data.x_categories,
            title: "N content across all bases".to_string(),
        })
    }
}

impl QCModule for NContent {
    fn process_sequence(&mut self, sequence: &Sequence) {
        let seq = &sequence.sequence;

        // Grow arrays if needed
        if self.n_counts.len() < seq.len() {
            self.n_counts.resize(seq.len(), 0);
            self.not_n_counts.resize(seq.len(), 0);
        }

        // Use lookup table to classify each byte without a multi-way match
        for (i, &b) in seq.iter().enumerate() {
            if BASE_INDEX[b as usize] as usize == IDX_N {
                self.n_counts[i] += 1;
            } else {
                self.not_n_counts[i] += 1;
            }
        }
    }

    fn name(&self) -> &str {
        "Per base N content"
    }

    fn description(&self) -> &str {
        "Shows the percentage of bases at each position which are not being called"
    }

    fn reset(&mut self) {
        self.n_counts.clear();
        self.not_n_counts.clear();
    }

    fn raises_error(&self) -> bool {
        let threshold = self.limits.threshold("n_content\terror", 20.0);
        let data = self.calculate();
        data.percentages.iter().any(|&p| p > threshold)
    }

    fn raises_warning(&self) -> bool {
        let threshold = self.limits.threshold("n_content\twarn", 5.0);
        let data = self.calculate();
        data.percentages.iter().any(|&p| p > threshold)
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        self.limits.is_ignored("n_content")
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let data = self.calculate();

        // Header matches Java's makeReport
        writeln!(writer, "#Base\tN-Count")?;

        for i in 0..data.x_categories.len() {
            writeln!(
                writer,
                "{}\t{}",
                data.x_categories[i],
                java_format_double(data.percentages[i]),
            )?;
        }

        Ok(())
    }

    // Image filename matches Java's "per_base_n_content.png" in Images/
    fn chart_image_name(&self) -> Option<&str> {
        Some("per_base_n_content")
    }
    fn chart_alt_text(&self) -> Option<&str> {
        Some("N content graph")
    }
    fn generate_chart_svg(&self) -> Option<String> {
        Some(self.build_chart_svg())
    }
}

struct NContentData {
    x_categories: Vec<String>,
    percentages: Vec<f64>,
}
