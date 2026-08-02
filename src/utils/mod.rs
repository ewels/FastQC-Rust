pub mod base_counts;
pub mod base_group;
pub mod dna;
pub mod format;
pub mod phred;
pub mod quality_count;

/// The machine's available parallelism (usable CPU count), falling back to 1
/// when it cannot be determined.
///
/// This is what the runner's thread planning budgets against. It honours cgroup
/// quotas and CPU affinity, so a container or a scheduler-pinned job sees its
/// own allowance rather than the host's core count.
pub fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
