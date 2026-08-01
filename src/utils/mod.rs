pub mod base_counts;
pub mod base_group;
pub mod dna;
pub mod format;
pub mod phred;
pub mod quality_count;

/// The machine's available parallelism (usable CPU count), falling back to 1
/// when it cannot be determined. Single home for the CPU-count policy shared by
/// the runner's decompression-thread budgeting and the rapidgzip reader.
pub fn available_parallelism() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}
