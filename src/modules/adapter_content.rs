// Adapter Content module
// Corresponds to Modules/AdapterContent.java

use std::io;

use aho_corasick::{packed, Span};
use memchr::memmem;

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::report::charts::line_graph::{render_line_graph, LineGraphData};
use crate::report::charts::scaled_chart_width;
use crate::sequence::Sequence;
use crate::utils::base_group::BaseGroup;
use crate::utils::format::java_format_double;

/// A single adapter to search for in sequences.
struct Adapter {
    name: String,
    /// Reads whose first match starts at each position; `cumulative_positions`
    /// gives Java's running totals.
    first_hits: Vec<u64>,
}

impl Adapter {
    fn new(name: &str) -> Self {
        Adapter {
            name: name.to_string(),
            first_hits: vec![0; 1],
        }
    }

    fn record_hit(&mut self, position: usize) {
        // Like Java, a match past the currently tracked positions is dropped,
        // not counted once a longer read extends them.
        if let Some(count) = self.first_hits.get_mut(position) {
            *count += 1;
        }
    }

    /// Java's `positions`: reads with a match at or before each position.
    fn cumulative_positions(&self) -> Vec<u64> {
        self.first_hits
            .iter()
            .scan(0u64, |total, &hits| {
                *total += hits;
                Some(*total)
            })
            .collect()
    }
}

/// Finds each adapter's first match in a read. One SIMD multi-pattern pass
/// (aho-corasick's Teddy) where possible, else a substring search per adapter.
struct AdapterSearch {
    finders: Vec<memmem::Finder<'static>>,
    teddy: Option<packed::Searcher>,
}

impl AdapterSearch {
    fn new(adapters: &[(String, String)]) -> Self {
        let seqs: Vec<&[u8]> = adapters.iter().map(|(_, s)| s.as_bytes()).collect();
        AdapterSearch {
            finders: seqs
                .iter()
                .map(|s| memmem::Finder::new(s).into_owned())
                .collect(),
            teddy: Self::build_teddy(&seqs),
        }
    }

    /// `None` if Teddy can't be built for these patterns, or if one adapter is
    /// a prefix of another: leftmost-first reports one match per start position.
    fn build_teddy(seqs: &[&[u8]]) -> Option<packed::Searcher> {
        if seqs.is_empty() || seqs.len() > 64 || seqs.iter().any(|s| s.is_empty()) {
            return None;
        }
        for (i, a) in seqs.iter().enumerate() {
            for (j, b) in seqs.iter().enumerate() {
                if i != j && b.starts_with(a) {
                    return None;
                }
            }
        }
        packed::Config::new()
            .match_kind(packed::MatchKind::LeftmostFirst)
            .builder()
            .extend(seqs)
            .build()
    }

    /// Calls `on_hit(adapter_index, start)` once for the leftmost match of
    /// each adapter in `seq`.
    fn first_matches(&self, seq: &[u8], mut on_hit: impl FnMut(usize, usize)) {
        let Some(teddy) = &self.teddy else {
            for (a, finder) in self.finders.iter().enumerate() {
                if let Some(start) = finder.find(seq) {
                    on_hit(a, start);
                }
            }
            return;
        };

        let mut remaining: u64 = u64::MAX >> (64 - self.finders.len());
        let mut at = 0;
        while remaining != 0 && at < seq.len() {
            let Some(m) = teddy.find_in(seq, Span::from(at..seq.len())) else {
                return;
            };
            let a = m.pattern().as_usize();
            if remaining & (1 << a) == 0 {
                // A repeat (typically inside a polyA/polyG run) would otherwise
                // cost one restart per base. No adapter is a prefix of another,
                // so nothing unseen starts before here: search the rest singly.
                break;
            }
            remaining &= !(1 << a);
            on_hit(a, m.start());
            // One past the start, not the end, so overlapping matches of other
            // adapters aren't skipped.
            at = m.start() + 1;
        }
        let rest = &seq[at.min(seq.len())..];
        while remaining != 0 {
            let a = remaining.trailing_zeros() as usize;
            remaining &= remaining - 1;
            if let Some(start) = self.finders[a].find(rest) {
                on_hit(a, at + start);
            }
        }
    }
}

pub struct AdapterContent {
    adapters: Vec<Adapter>,
    search: AdapterSearch,
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
            adapters.push(Adapter::new(name));
        }

        if adapter_entries
            .iter()
            .any(|(_, seq)| seq.len() != longest_adapter)
        {
            eprintln!("[Warning] You are using adapter sequences with different lengths. Matches will only be reported up to the position where the longest adapter could match. Matches to shorter adapters at the end of sequences will not be recorded.");
        }

        AdapterContent {
            adapters,
            search: AdapterSearch::new(adapter_entries),
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

        let all_positions: Vec<Vec<u64>> = self
            .adapters
            .iter()
            .map(Adapter::cumulative_positions)
            .collect();
        let max_length = all_positions.iter().map(Vec::len).max().unwrap_or(0);

        // Group positions using BaseGroup
        let groups = BaseGroup::make_base_groups(max_length, self.nogroup, self.expgroup);

        let x_labels: Vec<String> = groups.iter().map(|g| g.label()).collect();

        let mut enrichments = vec![vec![0.0f64; groups.len()]; self.adapters.len()];

        for (a, positions) in all_positions.iter().enumerate() {
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

    /// No read was longer than the longest adapter, so Java skips the analysis
    /// (no table or chart) and just warns.
    fn reads_too_short(&self) -> bool {
        self.longest_adapter > self.longest_sequence
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
            width: scaled_chart_width(computed.x_labels.len()),
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
    fn cost_hint(&self) -> u32 {
        9
    }

    fn process_sequence(&mut self, sequence: &Sequence) {
        self.computed = None;
        self.total_count += 1;

        let seq_len = sequence.sequence.len();

        // Java only grows the arrays once a read is longer than the longest adapter.
        if seq_len > self.longest_sequence && seq_len > self.longest_adapter {
            self.longest_sequence = seq_len;
            let new_len = (self.longest_sequence - self.longest_adapter) + 1;
            for adapter in &mut self.adapters {
                adapter.first_hits.resize(new_len, 0);
            }
        }

        let adapters = &mut self.adapters;
        self.search
            .first_matches(&sequence.sequence, |a, start| adapters[a].record_hit(start));
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
            adapter.first_hits = vec![0; 1];
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
        if self.reads_too_short() {
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

    fn write_html_report(&self, writer: &mut dyn io::Write, png: bool) -> io::Result<()> {
        if self.reads_too_short() {
            return write!(
                writer,
                "<p>Can't analyse adapters as read length is too short ({} vs {})</p>",
                self.longest_adapter, self.longest_sequence
            );
        }
        crate::report::html::write_chart(self, "Adapter graph", png, writer)
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let computed = self.ensure_calculated();

        if self.reads_too_short() {
            return Ok(());
        }

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
        if self.reads_too_short() {
            return None;
        }
        Some(self.build_chart_svg())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEFAULT_ADAPTERS: [&str; 6] = [
        "AGATCGGAAGAG",
        "TGGAATTCTCGG",
        "GATCGTCGGACT",
        "CTGTCTCTTATA",
        "AAAAAAAAAAAA",
        "GGGGGGGGGGGG",
    ];

    fn entries(seqs: &[&str]) -> Vec<(String, String)> {
        seqs.iter()
            .map(|s| (s.to_string(), s.to_string()))
            .collect()
    }

    fn memmem_first(adapters: &[(String, String)], seq: &[u8]) -> Vec<Option<usize>> {
        adapters
            .iter()
            .map(|(_, a)| memmem::find(seq, a.as_bytes()))
            .collect()
    }

    fn search_first(search: &AdapterSearch, seq: &[u8]) -> Vec<Option<usize>> {
        let mut found = vec![None; search.finders.len()];
        search.first_matches(seq, |a, start| {
            assert!(found[a].is_none(), "adapter {a} reported twice");
            found[a] = Some(start);
        });
        found
    }

    /// The multi-pattern search must report exactly the first match that a
    /// separate substring search per adapter would.
    #[test]
    fn test_adapter_search_matches_memmem() {
        let adapters = entries(&DEFAULT_ADAPTERS);
        let search = AdapterSearch::new(&adapters);
        assert!(search.teddy.is_some(), "default adapters use Teddy");

        let poly_g = vec![b'G'; 150];
        let mut poly_a_tail = b"ACGT".repeat(500);
        poly_a_tail.extend_from_slice(&[b'A'; 5000]);
        poly_a_tail.extend_from_slice(b"AGATCGGAAGAG");
        let mut seqs: Vec<Vec<u8>> = vec![
            b"".to_vec(),
            b"AGATCGGAAGA".to_vec(),
            b"NNNAGATCGGAAGAGNNN".to_vec(),
            // Adapter 0 twice, overlapping itself.
            b"CCAGATCGGAAGAGATCGGAAGAGTT".to_vec(),
            // PolyA overlaps the last base of adapter 3; restarting at a
            // match's end would miss it.
            b"CTGTCTCTTATAAAAAAAAAAA".to_vec(),
            // Other adapters inside and after homopolymer runs.
            b"GGGGGGGGGGGGGGGGGGGGAGATCGGAAGAGGGGGGGGGGGGGGGAAAAAAAAAAAAAA".to_vec(),
            b"AAAAAAAAAAAAAAAAAAAAAAAAACTGTCTCTTATAAAAAAAAAAAAAAAATGGAATTCTCGG".to_vec(),
            poly_g,
            poly_a_tail,
            b"GGGGGGGGGGGGAAAAAAAAAAAACTGTCTCTTATAGATCGTCGGACTTGGAATTCTCGGAGATCGGAAGAG".to_vec(),
        ];
        // Deterministic pseudo-random reads biased towards adapter fragments
        // and homopolymer runs.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..5000 {
            let len = (next() % 300) as usize;
            let mut seq = Vec::with_capacity(len + 40);
            while seq.len() < len {
                match next() % 16 {
                    0 | 1 => seq.extend_from_slice(adapters[(next() % 6) as usize].1.as_bytes()),
                    2 => {
                        let run = (next() % 40) as usize;
                        seq.extend(std::iter::repeat_n(b"AG"[(next() % 2) as usize], run));
                    }
                    _ => seq.push(b"ACGTN"[(next() % 5) as usize]),
                }
            }
            seqs.push(seq);
        }

        for seq in &seqs {
            assert_eq!(
                search_first(&search, seq),
                memmem_first(&adapters, seq),
                "{}",
                String::from_utf8_lossy(seq)
            );
        }
    }

    /// Leftmost-first cannot report two adapters starting at the same
    /// position, so a prefix pair must fall back to per-adapter search.
    #[test]
    fn test_adapter_search_prefix_fallback() {
        let prefix = entries(&["ACGTACGT", "ACGTACGTAA"]);
        let search = AdapterSearch::new(&prefix);
        assert!(search.teddy.is_none());
        assert_eq!(
            search_first(&search, b"TTACGTACGTAA"),
            vec![Some(2), Some(2)]
        );

        assert!(AdapterSearch::new(&entries(&["ACGTACGT", "ACGTACGT"]))
            .teddy
            .is_none());
        assert!(AdapterSearch::new(&entries(&["ACGTACGT", "CGTACGTA"]))
            .teddy
            .is_some());
    }

    /// Hits are tallied per first-match position and summed at the end; the
    /// cumulative curve must equal incrementing every later position per hit.
    #[test]
    fn test_cumulative_positions() {
        let mut adapter = Adapter::new("x");
        adapter.first_hits = vec![0; 5];
        for p in [0, 2, 2, 4, 9] {
            adapter.record_hit(p);
        }
        assert_eq!(adapter.cumulative_positions(), vec![1, 1, 3, 3, 4]);
    }
}
