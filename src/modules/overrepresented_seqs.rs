// Overrepresented Sequences module
// Corresponds to Modules/OverRepresentedSeqs.java

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, Mutex};

use crate::config::{Limits, LimitsExt};
use crate::modules::QCModule;
use crate::sequence::Sequence;
use crate::utils::format::java_format_double;

/// Shared data between OverRepresentedSeqs and DuplicationLevel.
/// In Java, DuplicationLevel directly accesses OverRepresentedSeqs' fields.
#[derive(Default)]
pub struct OverRepresentedData {
    /// Map of truncated sequence -> count
    pub sequences: HashMap<String, u64>,
    /// Total number of sequences processed
    pub count: u64,
    /// Total count at the point where we reached the unique sequence limit
    pub count_at_unique_limit: u64,
}

impl OverRepresentedData {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Result of contaminant matching for an overrepresented sequence.
struct ContaminantHit {
    name: String,
    length: usize,
    percent_id: usize,
}

impl ContaminantHit {
    /// Returns true if this hit is better (longer match or higher identity at same length)
    /// than `other`, or if `other` is None.
    /// Replicates the comparison logic used throughout Contaminant.findMatch().
    fn is_better_than(&self, other: &Option<ContaminantHit>) -> bool {
        match other {
            None => true,
            Some(b) => {
                self.length > b.length
                    || (self.length == b.length && self.percent_id > b.percent_id)
            }
        }
    }
}

impl std::fmt::Display for ContaminantHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Format matches ContaminantHit.toString()
        write!(
            f,
            "{} ({}% over {}bp)",
            self.name, self.percent_id, self.length
        )
    }
}

/// A single overrepresented sequence entry for reporting.
struct OverrepresentedSeq {
    seq: String,
    count: u64,
    percentage: f64,
    contaminant_hit: Option<ContaminantHit>,
}

/// A contaminant entry loaded from the contaminant list.
struct Contaminant {
    name: String,
    forward: Vec<u8>,
    reverse: Vec<u8>,
}

impl Contaminant {
    fn new(name: &str, sequence: &str) -> Self {
        let forward: Vec<u8> = sequence.to_uppercase().bytes().collect();
        // Reverse complement computed exactly as in Contaminant.java
        let mut reverse = vec![0u8; forward.len()];
        for (c, &base) in forward.iter().enumerate() {
            let rev_pos = (forward.len() - 1) - c;
            reverse[rev_pos] = match base {
                b'G' => b'C',
                b'A' => b'T',
                b'T' => b'A',
                b'C' => b'G',
                _ => base,
            };
        }
        Contaminant {
            name: name.to_string(),
            forward,
            reverse,
        }
    }

    /// Replicates Contaminant.findMatch() - for short sequences (<20bp, >=8bp)
    /// checks if query is a substring of the contaminant. For longer sequences, slides with
    /// 1 mismatch tolerance and requires >=20bp match length.
    fn find_match(&self, query: &str) -> Option<ContaminantHit> {
        let query_upper = query.to_uppercase();

        // Special case for short queries (8-19bp) - exact substring match
        if query_upper.len() < 20 && query_upper.len() >= 8 {
            let forward_str = std::str::from_utf8(&self.forward).unwrap_or("");
            let reverse_str = std::str::from_utf8(&self.reverse).unwrap_or("");

            if forward_str.contains(&query_upper) {
                return Some(ContaminantHit {
                    name: self.name.clone(),
                    length: query_upper.len(),
                    percent_id: 100,
                });
            }
            if reverse_str.contains(&query_upper) {
                return Some(ContaminantHit {
                    name: self.name.clone(),
                    length: query_upper.len(),
                    percent_id: 100,
                });
            }
        }

        let q: Vec<u8> = query_upper.bytes().collect();
        let mut best_hit: Option<ContaminantHit> = None;

        // Check forward strand with sliding window and 1 mismatch tolerance
        best_hit = Self::find_strand_match(&self.forward, &q, best_hit, &self.name);
        best_hit = Self::find_strand_match(&self.reverse, &q, best_hit, &self.name);

        best_hit
    }

    fn find_strand_match(
        ca: &[u8],
        cb: &[u8],
        mut best_hit: Option<ContaminantHit>,
        name: &str,
    ) -> Option<ContaminantHit> {
        let min_offset = -(ca.len() as isize - 20);
        let max_offset = cb.len() as isize - 20;

        for offset in min_offset..max_offset {
            if let Some(hit) = Self::find_match_at_offset(ca, cb, offset, name) {
                if hit.is_better_than(&best_hit) {
                    best_hit = Some(hit);
                }
            }
        }
        best_hit
    }

    /// Replicates the private findMatch(char[], char[], int, int) method.
    fn find_match_at_offset(
        ca: &[u8],
        cb: &[u8],
        offset: isize,
        name: &str,
    ) -> Option<ContaminantHit> {
        let mut best_hit: Option<ContaminantHit> = None;
        let mut mismatch_count: usize = 0;
        // Use isize to avoid underflow when offset causes start > end
        let mut start: isize = 0;
        let mut end: isize = 0;

        // index i used to access both ca[i] and cb[i+offset]
        for (i, &ca_byte) in ca.iter().enumerate() {
            let j = i as isize + offset;
            if j < 0 {
                start = i as isize + 1;
                continue;
            }
            if j >= cb.len() as isize {
                break;
            }

            if ca_byte == cb[j as usize] {
                end = i as isize;
            } else {
                mismatch_count += 1;
                if mismatch_count > 1 {
                    // Check if match so far is worth recording (>20bp)
                    if end >= start {
                        let match_len = (1 + end - start) as usize;
                        if match_len > 20 {
                            let id = ((match_len - (mismatch_count - 1)) * 100) / match_len;
                            let candidate = ContaminantHit {
                                name: name.to_string(),
                                length: match_len,
                                percent_id: id,
                            };
                            if candidate.is_better_than(&best_hit) {
                                best_hit = Some(candidate);
                            }
                        }
                    }
                    start = i as isize + 1;
                    end = i as isize + 1;
                    mismatch_count = 0;
                }
            }
        }

        // Check final stretch
        if end < start {
            return best_hit;
        }
        let match_len = (1 + end - start) as usize;
        if match_len > 20 {
            let id = ((match_len - mismatch_count) * 100) / match_len;
            let candidate = ContaminantHit {
                name: name.to_string(),
                length: match_len,
                percent_id: id,
            };
            if candidate.is_better_than(&best_hit) {
                best_hit = Some(candidate);
            }
        }

        best_hit
    }
}

/// Find the best contaminant match for a query sequence.
/// Replicates ContaminentFinder.findContaminantHit()
fn find_contaminant_hit(query: &str, contaminants: &[Contaminant]) -> Option<ContaminantHit> {
    let mut best_hit: Option<ContaminantHit> = None;

    for contaminant in contaminants {
        if let Some(hit) = contaminant.find_match(query) {
            if hit.is_better_than(&best_hit) {
                best_hit = Some(hit);
            }
        }
    }

    best_hit
}

pub struct OverRepresentedSeqs {
    pub shared_data: Arc<Mutex<OverRepresentedData>>,
    /// Maximum 100000 unique sequences tracked
    unique_sequence_count: usize,
    frozen: bool,
    dup_length: usize,
    contaminants: Vec<Contaminant>,
    limits: Limits,
    // Lazily computed
    computed: Option<Vec<OverrepresentedSeq>>,
}

/// Maximum number of unique sequences to track
const OBSERVATION_CUTOFF: usize = 100_000;

impl OverRepresentedSeqs {
    pub fn new(
        limits: &Limits,
        dup_length: usize,
        contaminant_entries: &[(String, String)],
        shared_data: Arc<Mutex<OverRepresentedData>>,
    ) -> Self {
        let contaminants: Vec<Contaminant> = contaminant_entries
            .iter()
            .map(|(name, seq)| Contaminant::new(name, seq))
            .collect();

        OverRepresentedSeqs {
            shared_data,
            unique_sequence_count: 0,
            frozen: false,
            dup_length,
            contaminants,
            limits: limits.clone(),
            computed: None,
        }
    }

    fn get_overrepresented_seqs(&mut self) {
        if self.computed.is_some() {
            return;
        }

        let warn_threshold = self.limits.threshold("overrepresented\twarn", 0.1);

        // Use into_inner on PoisonError to recover the guard even after a panic in
        // another module -- losing the analysis is worse than using slightly stale data.
        let data = self.shared_data.lock().unwrap_or_else(|e| e.into_inner());
        let total_count = data.count;

        let mut keepers: Vec<OverrepresentedSeq> = Vec::new();

        for (seq, &count) in &data.sequences {
            let percentage = (count as f64 / total_count as f64) * 100.0;
            if percentage > warn_threshold {
                let hit = find_contaminant_hit(seq, &self.contaminants);
                keepers.push(OverrepresentedSeq {
                    seq: seq.clone(),
                    count,
                    percentage,
                    contaminant_hit: hit,
                });
            }
        }

        // JAVA COMPAT: Sort by count descending, then by sequence ascending as tiebreaker.
        // Java's Arrays.sort is stable and preserves HashMap iteration order for equal counts,
        // but Rust's HashMap has different iteration order. Using sequence as tiebreaker
        // ensures deterministic output regardless of HashMap order.
        keepers.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.seq.cmp(&b.seq)));

        self.computed = Some(keepers);
    }

    fn ensure_calculated(&self) -> &[OverrepresentedSeq] {
        self.computed.as_deref().unwrap_or(&[])
    }
}

impl QCModule for OverRepresentedSeqs {
    fn process_sequence(&mut self, sequence: &Sequence) {
        self.computed = None;

        let mut data = self.shared_data.lock().unwrap_or_else(|e| e.into_inner());
        data.count += 1;

        // Truncate sequence to dup_length or 50bp if longer than 50
        // Safety: sequence bytes are ASCII (uppercased in Sequence::new), so slicing is valid UTF-8
        let seq_bytes = &sequence.sequence;
        let truncate_len = if self.dup_length != 0 && seq_bytes.len() > self.dup_length {
            self.dup_length
        } else if seq_bytes.len() > 50 {
            // Default truncation to 50bp for sequences longer than 50
            50
        } else {
            seq_bytes.len()
        };
        let seq = std::str::from_utf8(&seq_bytes[..truncate_len]).unwrap_or("");

        if let Some(count) = data.sequences.get_mut(seq) {
            *count += 1;
            // Keep updating countAtUniqueLimit while not frozen
            if !self.frozen {
                data.count_at_unique_limit = data.count;
            }
        } else if !self.frozen {
            data.sequences.insert(seq.to_string(), 1);
            self.unique_sequence_count += 1;
            data.count_at_unique_limit = data.count;
            if self.unique_sequence_count == OBSERVATION_CUTOFF {
                self.frozen = true;
            }
        }
    }

    fn finalize(&mut self) {
        self.get_overrepresented_seqs();
    }

    fn name(&self) -> &str {
        "Overrepresented sequences"
    }

    fn description(&self) -> &str {
        "Identifies sequences which are overrepresented in the set"
    }

    fn reset(&mut self) {
        let mut data = self.shared_data.lock().unwrap_or_else(|e| e.into_inner());
        data.count = 0;
        data.count_at_unique_limit = 0;
        data.sequences.clear();
        self.unique_sequence_count = 0;
        self.frozen = false;
        self.computed = None;
    }

    fn raises_error(&self) -> bool {
        let error_threshold = self.limits.threshold("overrepresented\terror", 1.0);
        let seqs = self.ensure_calculated();
        // Check if the top sequence exceeds error threshold
        seqs.first().is_some_and(|s| s.percentage > error_threshold)
    }

    fn raises_warning(&self) -> bool {
        let seqs = self.ensure_calculated();
        // Any overrepresented sequence triggers a warning
        !seqs.is_empty()
    }

    fn ignore_filtered_sequences(&self) -> bool {
        true
    }

    fn ignore_in_report(&self) -> bool {
        self.limits.is_ignored("overrepresented")
    }

    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        let seqs = self.ensure_calculated();

        // When there are no overrepresented sequences, Java writes nothing
        // to the data section (no header, no rows). The header is only output when
        // writeTable() is called, which is skipped when overrepresntedSeqs.length == 0.
        if seqs.is_empty() {
            return Ok(());
        }

        writeln!(writer, "#Sequence\tCount\tPercentage\tPossible Source")?;
        for s in seqs {
            let source = match &s.contaminant_hit {
                Some(hit) => hit.to_string(),
                None => "No Hit".to_string(),
            };
            // The Java ResultsTable.getValueAt() for percentage does
            // JAVA COMPAT: Math.round(percentage * 100.0) / 100.0, rounding to 2 decimal places.
            // The text report then calls String.valueOf() on the Double, producing
            // Java's Double.toString() format.
            let rounded_pct = (s.percentage * 100.0).round() / 100.0;
            writeln!(
                writer,
                "{}\t{}\t{}\t{}",
                s.seq,
                s.count,
                java_format_double(rounded_pct),
                source
            )?;
        }

        Ok(())
    }
}
