// Adapter Content module
// Corresponds to Modules/AdapterContent.java

use std::io;

use memchr::memmem;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::sequence::Sequence;
use crate::utils::base_group::BaseGroup;
use crate::utils::format::java_format_double;

/// A single adapter to search for in sequences.
struct Adapter {
    name: String,
    /// Pre-compiled substring searcher for the adapter sequence.
    /// Using memchr::memmem::Finder avoids re-compiling the searcher on every call
    /// (str::find creates a new TwoWaySearcher per invocation, which was 60% of total runtime).
    finder: memmem::Finder<'static>,
    /// positions[i] = cumulative count of sequences with adapter found at or before position i.
    positions: Vec<u64>,
}

impl Adapter {
    fn new(name: &str, sequence: &str) -> Self {
        let seq_bytes = sequence.as_bytes().to_vec();
        let finder = memmem::Finder::new(&seq_bytes).into_owned();
        Adapter {
            name: name.to_string(),
            finder,
            positions: vec![0; 1],
        }
    }

    /// Expand positions array to new_length, copying the last value
    /// to newly added slots (to propagate cumulative counts).
    fn expand_length_to(&mut self, new_length: usize) {
        let old_len = self.positions.len();
        if new_length > old_len {
            let last_val = if old_len > 0 {
                self.positions[old_len - 1]
            } else {
                0
            };
            self.positions.resize(new_length, last_val);
        }
    }

    fn increment_count(&mut self, position: usize) {
        self.positions[position] += 1;
    }
}

pub struct AdapterContent {
    adapters: Vec<Adapter>,
    longest_sequence: usize,
    longest_adapter: usize,
    total_count: u64,
    limits: Limits,
    nogroup: bool,
    expgroup: bool,
    // Lazily computed
    computed: Option<ComputedEnrichment>,
}

struct ComputedEnrichment {
    enrichments: Vec<Vec<f64>>,
    x_labels: Vec<String>,
}

impl AdapterContent {
    pub fn new(
        limits: &Limits,
        adapter_entries: &[(String, String)],
        nogroup: bool,
        expgroup: bool,
    ) -> Self {
        let mut longest_adapter = 0;
        let mut adapters = Vec::with_capacity(adapter_entries.len());

        for (name, seq) in adapter_entries {
            if seq.len() > longest_adapter {
                longest_adapter = seq.len();
            }
            adapters.push(Adapter::new(name, seq));
        }

        AdapterContent {
            adapters,
            longest_sequence: 0,
            longest_adapter,
            total_count: 0,
            limits: limits.clone(),
            nogroup,
            expgroup,
            computed: None,
        }
    }

    /// Replicates calculateEnrichment() from AdapterContent.java.
    fn calculate_enrichment(&mut self) {
        if self.computed.is_some() {
            return;
        }

        let mut max_length = 0;
        for adapter in &self.adapters {
            if adapter.positions.len() > max_length {
                max_length = adapter.positions.len();
            }
        }

        // Group positions using BaseGroup
        let groups = BaseGroup::make_base_groups(max_length, self.nogroup, self.expgroup);

        let x_labels: Vec<String> = groups.iter().map(|g| g.label()).collect();

        let mut enrichments = vec![vec![0.0f64; groups.len()]; self.adapters.len()];

        for (a, adapter) in self.adapters.iter().enumerate() {
            let positions = &adapter.positions;

            for (g, group) in groups.iter().enumerate() {
                // lowerCount() is 1-based in Java, we use 0-based internally
                // Java: p=groups[g].lowerCount()-1; p<groups[g].upperCount()
                let lower = group.lower_count; // already 0-based
                let upper = group.upper_count; // already 0-based, inclusive

                for p in lower..=upper {
                    if p < positions.len() {
                        enrichments[a][g] +=
                            (positions[p] as f64 * 100.0) / self.total_count as f64;
                    }
                }

                // Average over the group width
                enrichments[a][g] /= (upper - lower + 1) as f64;
            }
        }

        self.computed = Some(ComputedEnrichment {
            enrichments,
            x_labels,
        });
    }

    /// Derive adapter names from the adapters Vec (avoids storing a redundant copy).
    fn adapter_names(&self) -> Vec<String> {
        self.adapters.iter().map(|a| a.name.clone()).collect()
    }

    fn ensure_calculated(&self) -> &ComputedEnrichment {
        static DEFAULT: ComputedEnrichment = ComputedEnrichment {
            enrichments: Vec::new(),
            x_labels: Vec::new(),
        };
        self.computed.as_ref().unwrap_or(&DEFAULT)
    }
}

impl AdapterContent {
    fn build_chart_svg(&self) -> String {
        let computed = self.ensure_calculated();

        // Matches Java's `new LineGraph(enrichments, 0, 100, "Position in read (bp)", labels, xLabels, "% Adapter")`
        render_line_graph(&LineGraphData {
            data: computed.enrichments.clone(),
            min_y: 0.0,
            max_y: 100.0,
            x_label: "Position in read (bp)".to_string(),
            series_names: self.adapter_names(),
            x_categories: computed.x_labels.clone(),
            title: "% Adapter".to_string(),
        })
    }
}

impl QCModule for AdapterContent {
    fn process_sequence(&mut self, sequence: &Sequence) {
        self.computed = None;
        self.total_count += 1;

        let seq_len = sequence.sequence.len();

        // Expand adapter positions if sequence is longer than before
        // AND the last matchable position is positive
        if seq_len > self.longest_sequence && seq_len > self.longest_adapter {
            self.longest_sequence = seq_len;
            let new_len = (self.longest_sequence - self.longest_adapter) + 1;
            for adapter in &mut self.adapters {
                adapter.expand_length_to(new_len);
            }
        }

        // Search for each adapter in the sequence using pre-compiled searchers.
        // Uses memchr::memmem::Finder on raw bytes — avoids UTF-8 conversion and
        // re-compiling the search pattern on every call (was 60% of total runtime with str::find).
        let seq_bytes = &sequence.sequence;
        let max_pos = self.longest_sequence.saturating_sub(self.longest_adapter);

        for adapter in &mut self.adapters {
            if let Some(index) = adapter.finder.find(seq_bytes) {
                // Once found at position index, increment all positions
                // from index through longestSequence-longestAdapter
                for i in index..=max_pos {
                    if i < adapter.positions.len() {
                        adapter.increment_count(i);
                    }
                }
            }
        }
    }

    fn finalize(&mut self) {
        self.calculate_enrichment();
    }

    fn name(&self) -> &str {
        "Adapter Content"
    }

    fn description(&self) -> &str {
        "Searches for specific adapter sequences in a library"
    }

    fn reset(&mut self) {
        self.total_count = 0;
        self.longest_sequence = 0;
        self.computed = None;
        for adapter in &mut self.adapters {
            adapter.positions = vec![0; 1];
        }
    }

    fn raises_error(&self) -> bool {
        let threshold = self.limits.threshold("adapter\terror", 10.0);
        let computed = self.ensure_calculated();
        computed
            .enrichments
            .iter()
            .any(|enrichments| enrichments.iter().any(|&val| val > threshold))
    }

    fn raises_warning(&self) -> bool {
        // Warn if adapters are longer than sequences
        if self.longest_adapter > self.longest_sequence {
            return true;
        }

        let threshold = self.limits.threshold("adapter\twarn", 5.0);
        let computed = self.ensure_calculated();
        computed
            .enrichments
            .iter()
            .any(|enrichments| enrichments.iter().any(|&val| val > threshold))
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        self.limits.is_ignored("adapter")
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let computed = self.ensure_calculated();

        // Header line with Position tab and all adapter names
        write!(writer, "#Position")?;
        for adapter in &self.adapters {
            write!(writer, "\t{}", adapter.name)?;
        }
        writeln!(writer)?;

        // One row per base group, columns are Position then each adapter's enrichment
        for (row, x_label) in computed.x_labels.iter().enumerate() {
            write!(writer, "{}", x_label)?;
            for a in 0..self.adapters.len() {
                write!(
                    writer,
                    "\t{}",
                    java_format_double(computed.enrichments[a][row])
                )?;
            }
            writeln!(writer)?;
        }

        Ok(())
    }

    // Image filename matches Java's "adapter_content.png" in Images/
    fn chart_image_name(&self) -> Option<&str> {
        Some("adapter_content")
    }
    fn chart_alt_text(&self) -> Option<&str> {
        Some("Adapter graph")
    }
    fn generate_chart_svg(&self) -> Option<String> {
        Some(self.build_chart_svg())
    }
}
