use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::sync_channel;
use std::sync::Arc;
use std::thread;

use rayon::prelude::*;

use crate::config::FastQCConfig;
use crate::modules;
use crate::modules::QCModule;
use crate::progress::{self, FileProgress};
use crate::report;
use crate::sequence::casava;
use crate::sequence::open_sequence_file;
use crate::sequence::{Sequence, SequenceFile, SequenceFileGroup};

/// Upper bound on per-file processor threads in the parallel analysis pipeline.
/// Each processor runs a disjoint subset of the QC modules over every sequence,
/// so the analysis stays single-threaded per module (no in-module locking,
/// byte-identical output) while the work is spread across cores.
///
/// Because the work is split by module, more processors than modules cannot help
/// (the effective count is capped by the number of active modules in
/// `process_group`), and the wall-clock is ultimately bounded by the single
/// heaviest module — for the default set that is Adapter Content at ~24% of the
/// analysis, i.e. a ceiling of roughly 4x however many cores are available. The
/// reader thread only reads and parses records (cheap relative to the analysis,
/// and it overlaps with parallel gzip decompression), so it is not the
/// bottleneck the upstream Java cap assumed; we therefore let a single large file
/// spread across as many workers as there are modules. Beyond this a single file
/// cannot go faster without splitting a module's work, which the order-/float-
/// sensitive modules (overrepresented sequences, per-sequence GC) cannot do while
/// staying byte-identical; extra cores are instead used to analyse more files
/// concurrently.
const MAX_PROCESSORS_PER_FILE: usize = 12;

/// Number of sequences the reader batches before publishing to the processors.
/// Matches the Java pipeline's batch size. Large enough to amortise channel and
/// Arc overhead, small enough to keep the processors fed and memory bounded.
const BATCH_SIZE: usize = 1024;

/// Bounded capacity of each processor's batch queue. Provides backpressure so the
/// reader can run at most this many batches ahead of the slowest processor,
/// keeping peak memory bounded. Matches the Java pipeline's queue capacity.
const QUEUE_CAPACITY: usize = 32;

/// A unit of work: one logical sample to process through all QC modules.
/// Contains a display name and the list of file paths that constitute it.
struct FileGroup {
    /// The display name for reports (CASAVA basename or original filename).
    name: String,
    /// The file paths in this group (usually 1, but >1 for CASAVA groups).
    files: Vec<PathBuf>,
}

/// Run FastQC analysis on the given input files.
///
/// The Java OfflineRunner iterates over files, creates a
/// SequenceFile reader for each, instantiates all QC modules, feeds every
/// Sequence to each module, then writes the report. With --threads, files
/// are processed in parallel via AnalysisQueue.
pub fn run(config: &FastQCConfig, files: &[PathBuf]) -> Result<(), i32> {
    let limits = config.load_limits().map_err(|e| {
        eprintln!("Failed to load limits: {}", e);
        1
    })?;

    // Validate all files exist before starting processing.
    // For stdin, skip the existence check (Java: `filenames[0].startsWith("stdin")`).
    // For --nano mode, expand directories to find .fast5 files within them.
    let mut valid_files = Vec::new();
    let mut something_failed = false;
    for file_path in files {
        let file_name = file_path.to_string_lossy();
        if !file_name.starts_with("stdin") && !file_path.exists() {
            eprintln!("{} doesn't exist", file_name);
            something_failed = true;
        } else if config.nano && file_path.is_dir() {
            // In --nano mode, directories are recursively searched for .fast5 files.
            // Matches OfflineRunner.java's directory expansion logic.
            match find_fast5_files(file_path) {
                Ok(fast5_files) => {
                    if fast5_files.is_empty() {
                        eprintln!("No .fast5 files found in {}", file_path.display());
                        something_failed = true;
                    } else {
                        valid_files.extend(fast5_files);
                    }
                }
                Err(e) => {
                    eprintln!("Error scanning directory {}: {}", file_path.display(), e);
                    something_failed = true;
                }
            }
        } else {
            valid_files.push(file_path.clone());
        }
    }

    // Group files based on mode (casava, nano, or individual).
    // Java's OfflineRunner.java lines 103-117 handles this branching.
    let file_groups = build_file_groups(config, &valid_files);

    // Split the total thread budget (-t/--threads) between outer concurrency
    // (file groups analysed in parallel) and inner concurrency (the per-file
    // reader + processor pipeline).
    //
    // Files are the cheapest axis to parallelise across — each one scales
    // linearly and needs no coordination — so spread the budget over the files
    // first, then hand whatever is left to each file's internal pipeline:
    //
    //   outer_slots         = min(files, total)          // files run at once
    //   threads_per_file    = max(1, total / outer_slots)
    //   processors_per_file = min(MAX_PROCESSORS_PER_FILE, threads_per_file - 1)
    //
    // So a lone large file gets the whole budget as an internal pipeline (reader
    // + up to MAX workers), while a run with many files mostly scales out across
    // them and only builds a pipeline per file if threads are left over. A budget
    // of 1 (or one thread per file) disables the pipeline entirely: each file is
    // analysed on a single thread, byte-identical to the original unbatched
    // runner. Going wide on one file is what lets a single .fastq.gz benefit from
    // extra threads now that parallel decompression has removed the gzip floor.
    let total_threads = config.threads.max(1);
    let n_files = file_groups.len().max(1);
    let outer_slots = n_files.min(total_threads);
    let threads_per_file = (total_threads / outer_slots).max(1);
    let processors_per_file = if threads_per_file <= 1 {
        0
    } else {
        MAX_PROCESSORS_PER_FILE.min(threads_per_file - 1)
    };

    // Normalise the parallel-gzip (rapidgzip) worker budget. When left on
    // "auto" (0), give each concurrently-decompressed file the cores left over
    // after its analysis pipeline (reader + processors) has taken its share, so
    // decompression and analysis overlap without gross CPU oversubscription.
    // zlib-rs decode is fast per-thread, so a small budget keeps the reader fed
    // once analysis (not gzip) is the bottleneck. An explicit
    // --decompress-threads value is used verbatim. This only matters when the
    // `rapidgzip` feature is compiled in, but the arithmetic is free otherwise.
    let owned_config;
    let config = if config.decompress_threads == 0 {
        let budget = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        // `outer_slots` (>= 1) is the number of files decompressed concurrently.
        let per_file = budget / outer_slots;
        let mut c = config.clone();
        c.decompress_threads = per_file.saturating_sub(processors_per_file).max(1);
        owned_config = c;
        &owned_config
    } else {
        config
    };

    // Build the outer rayon pool: one slot per file group analysed in parallel.
    // Each slot drives a per-file pipeline (a reader plus `processors_per_file`
    // std::thread workers), so the total live thread count stays near the -t
    // budget rather than outer_slots alone.
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(outer_slots)
        .build()
        .map_err(|e| {
            eprintln!("Failed to create thread pool: {}", e);
            1
        })?;

    let failed = AtomicBool::new(something_failed);
    let analysed = AtomicUsize::new(0);

    // The live terminal display: a progress bar per file (or one bar counting
    // files when there are many), plus a live statistics table for small runs.
    // It is inert under --quiet and degrades to plain start/finish lines when
    // stderr is not a terminal, unless FASTQC_PROGRESS says otherwise.
    let names: Vec<String> = file_groups.iter().map(|g| g.name.clone()).collect();
    let progress = progress::ProgressReporter::new(&names, config.quiet);

    pool.install(|| {
        file_groups
            .par_iter()
            .enumerate()
            .for_each(|(index, group)| {
                let file_progress = progress.file(index);
                file_progress.start(&group.name);

                match process_group(config, &limits, group, processors_per_file, file_progress) {
                    Ok(reads) => {
                        analysed.fetch_add(1, Ordering::Relaxed);
                        file_progress.finish(&group.name, reads);
                    }
                    Err(e) => {
                        file_progress.fail();
                        progress.error(&format!("Failed to process {}: {}", group.name, e));
                        failed.store(true, Ordering::Relaxed);
                    }
                }
            });
    });

    progress.finish(analysed.load(Ordering::Relaxed));

    if failed.load(Ordering::Relaxed) {
        Err(1)
    } else {
        Ok(())
    }
}

/// Build file groups based on the current mode (casava, or individual files).
///
/// Matches the grouping logic in OfflineRunner.java lines 103-117.
/// - If `--casava`: group by CASAVA basename
/// - Otherwise: each file is its own group
fn build_file_groups(config: &FastQCConfig, files: &[PathBuf]) -> Vec<FileGroup> {
    if config.casava {
        // CasavaBasename.getCasavaGroups() groups files by their
        // extracted basename. Files that don't match the pattern become singletons.
        let casava_groups = casava::get_casava_groups(files);
        casava_groups
            .into_iter()
            .map(|(name, paths)| FileGroup { name, files: paths })
            .collect()
    } else {
        // Default mode - each file is processed individually.
        // Java creates `fileGroups = new File[files.size()][1]` with one file per group.
        files
            .iter()
            .map(|path| {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.to_string_lossy().into_owned());
                FileGroup {
                    name,
                    files: vec![path.clone()],
                }
            })
            .collect()
    }
}

/// Process a file group (one or more files) through all QC modules and generate reports.
///
/// When a group has multiple files (CASAVA), they are combined
/// into a SequenceFileGroup that reads them sequentially as one logical sample.
fn process_group(
    config: &FastQCConfig,
    limits: &crate::config::Limits,
    group: &FileGroup,
    processors_per_file: usize,
    file_progress: FileProgress<'_>,
) -> io::Result<u64> {
    // Open the sequence file(s)
    let mut seq_file: Box<dyn SequenceFile> = if group.files.len() == 1 {
        // Single file - open directly
        // Uses format detection logic from SequenceFactory.java
        open_sequence_file(config, &group.files[0])?
    } else {
        // Multiple files in a CASAVA group - wrap them in a
        // SequenceFileGroup that reads all files sequentially as one stream.
        let mut readers: Vec<Box<dyn SequenceFile>> = Vec::new();
        for path in &group.files {
            readers.push(open_sequence_file(config, path)?);
        }
        Box::new(SequenceFileGroup::new(group.name.clone(), readers))
    };

    let file_display_name = group.name.clone();

    // Create module instances
    let mut modules = modules::create_modules(config, limits);

    // Set the filename on all modules (BasicStats uses it for the report)
    for module in modules.iter_mut() {
        module.set_filename(&file_display_name);
    }

    // If the terminal display is showing a live statistics table, give
    // BasicStats somewhere to publish its running counters. Only BasicStats
    // acts on this; nothing about the analysis changes.
    if let Some(live) = file_progress.live_stats() {
        for module in modules.iter_mut() {
            module.attach_live_stats(Arc::clone(&live));
        }
    }

    // Feed every sequence through all modules. With a budget of one thread we
    // take the original single-threaded path (byte-identical, no pipeline
    // overhead); otherwise a reader thread batches records and hands them to
    // worker threads that each own a disjoint subset of the modules. Either way
    // each module sees every sequence exactly once, in file order, on a single
    // thread, so the accumulated state -- and the resulting report -- is
    // identical regardless of the thread budget.
    //
    // There is no point in more workers than there are modules, so cap the
    // worker count by the number of active modules.
    let num_processors = processors_per_file.min(modules.len());
    let read_count = if num_processors == 0 {
        process_sequences_sequential(seq_file.as_mut(), &mut modules, file_progress)?
    } else {
        let (rebuilt, count) =
            process_sequences_parallel(seq_file.as_mut(), modules, num_processors, file_progress)?;
        modules = rebuilt;
        count
    };

    // Reading is done; the remaining work (chart rendering, HTML, zip) is not
    // measured as a fraction of the file, so the bar switches to a stage label.
    file_progress.stage("report");

    // Finalize all modules (lazy computation)
    for module in modules.iter_mut() {
        module.finalize();
    }

    // Generate output filename
    // For CASAVA groups, the display name is used as the base for
    // output files. For single files, it's the filename.
    // Strip extensions in order: .gz, .bz2, .txt, .fastq, .fq, .csfastq, .sam, .bam, .ubam
    let base_name = strip_extensions(&file_display_name.replace("stdin:", ""));

    // For output directory, use --outdir if specified, otherwise
    // use the parent directory of the first file in the group.
    let output_dir = if let Some(ref dir) = config.output_dir {
        dir.clone()
    } else {
        group
            .files
            .first()
            .and_then(|f| f.parent())
            .unwrap_or_else(|| Path::new("."))
            .to_path_buf()
    };

    // The Java code creates files at:
    //   {output_dir}/{base_name}_fastqc.html  (standalone HTML)
    //   {output_dir}/{base_name}_fastqc.zip   (zip archive)
    let html_path = output_dir.join(format!("{}_fastqc.html", base_name));
    let zip_path = output_dir.join(format!("{}_fastqc.zip", base_name));

    // Generate HTML report as a string (used for both standalone file and zip entry)
    let html_content =
        report::html::generate_html_report(&modules, &file_display_name, config.template)?;

    // Write standalone HTML file
    // The Java code writes the HTML via PrintWriter after creating the zip
    std::fs::write(&html_path, &html_content)?;

    // Create zip archive, reusing the already-generated HTML content
    report::archive::create_zip_archive(
        &modules,
        &file_display_name,
        &base_name,
        &zip_path,
        &html_content,
        config.svg_output,
        config.template,
    )?;

    // Handle --extract flag
    // If do_unzip is true, extract the zip file to the output directory.
    // If do_unzip is None (not specified), do not extract (matches Java default).
    if config.do_unzip == Some(true) {
        report::archive::extract_zip(&zip_path)?;

        // Handle --delete flag (only effective when --extract is also used)
        // Matches FastQCConfig.delete_after_unzip behavior
        if config.delete_after_unzip {
            std::fs::remove_file(&zip_path)?;
        }
    }

    Ok(read_count)
}

/// Feed one sequence to one module, honouring the module's request to skip
/// sequences flagged as filtered. Shared by the sequential and parallel paths so
/// the filtered-skip rule lives in one place.
#[inline]
fn feed_module(module: &mut dyn QCModule, seq: &Sequence) {
    if seq.is_filtered && module.ignore_filtered_sequences() {
        return;
    }
    module.process_sequence(seq);
}

/// How many records to read between updates of the terminal progress display.
/// The display throttles its own redraws, so this only needs to be frequent
/// enough that the bars look smooth.
const PROGRESS_INTERVAL: u64 = 1000;

/// Single-threaded analysis path: read each sequence and feed it to every module
/// in order. Used when the thread budget is 1, and byte-identical to the original
/// unbatched runner (AnalysisRunner.runSequential in the Java pipeline).
///
/// Returns the number of records read.
fn process_sequences_sequential(
    seq_file: &mut dyn SequenceFile,
    modules: &mut [Box<dyn QCModule>],
    file_progress: FileProgress<'_>,
) -> io::Result<u64> {
    let mut sequence_count: u64 = 0;

    loop {
        match seq_file.next() {
            Some(Ok(seq)) => {
                sequence_count += 1;

                for module in modules.iter_mut() {
                    feed_module(module.as_mut(), &seq);
                }

                // `percent_complete` costs a seek on the input file, so only
                // pay for it when there is a bar that will move.
                if sequence_count.is_multiple_of(PROGRESS_INTERVAL)
                    && file_progress.wants_position()
                {
                    file_progress.update(seq_file.percent_complete(), sequence_count);
                }
            }
            Some(Err(e)) => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, e));
            }
            None => break, // EOF
        }
    }

    Ok(sequence_count)
}

/// Multi-threaded analysis path: a reader on the calling thread batches sequences
/// and publishes each batch to `num_processors` worker threads, each of which owns
/// a disjoint subset of the modules and runs them over every sequence.
///
/// This is a Rust port of the upstream Java three-stage pipeline
/// (AnalysisRunner.runParallel). The key property is that the work is split *by
/// module*, not by data: every module still sees the entire sequence stream, in
/// file order, on a single thread, so no module needs merge logic and the output
/// is byte-identical to the sequential path. The parallelism comes from running
/// different modules concurrently, and from overlapping analysis with the reader
/// (which drives decompression and record parsing).
///
/// Modules are distributed across workers by longest-processing-time bin-packing
/// (see the partition below) so the few expensive modules (overrepresented
/// sequences, adapter and k-mer content) land on different workers, giving an even
/// load balance. Ownership of each module is returned to the caller in the original
/// order for finalisation and reporting.
///
/// Returns the modules in report order along with the number of records read.
fn process_sequences_parallel(
    seq_file: &mut dyn SequenceFile,
    modules: Vec<Box<dyn QCModule>>,
    num_processors: usize,
    file_progress: FileProgress<'_>,
) -> io::Result<(Vec<Box<dyn QCModule>>, u64)> {
    // Partition the modules across the workers to balance their estimated cost,
    // using the classic longest-processing-time (LPT) greedy: consider modules
    // heaviest-first and drop each onto the currently-lightest worker. This keeps
    // the few expensive modules (adapter, GC, per-base quality, ...) on separate
    // workers rather than letting a naive split pile two of them together, which
    // is what bounds the makespan of the pipeline. Modules are tagged with their
    // original index so report order can be restored afterwards; the assignment
    // never changes results, only how evenly the work is spread.
    let mut order: Vec<(usize, Box<dyn QCModule>)> = modules.into_iter().enumerate().collect();
    order.sort_by(|a, b| b.1.cost_hint().cmp(&a.1.cost_hint()));

    let mut groups: Vec<Vec<(usize, Box<dyn QCModule>)>> =
        (0..num_processors).map(|_| Vec::new()).collect();
    let mut loads = vec![0u64; num_processors];
    for (idx, module) in order {
        // Pick the lightest worker; ties break to the lowest index for
        // deterministic assignment.
        let target = loads
            .iter()
            .enumerate()
            .min_by_key(|&(_, &load)| load)
            .map(|(i, _)| i)
            .unwrap_or(0);
        loads[target] += module.cost_hint() as u64;
        groups[target].push((idx, module));
    }

    // One bounded queue per worker. The reader publishes the same Arc-shared
    // batch to every queue; workers only read from the sequences, never mutate
    // them, so sharing is safe. Bounded capacity gives backpressure so the
    // reader runs at most QUEUE_CAPACITY batches ahead of the slowest worker.
    let mut senders = Vec::with_capacity(num_processors);
    let mut receivers = Vec::with_capacity(num_processors);
    for _ in 0..num_processors {
        let (tx, rx) = sync_channel::<Arc<Vec<Sequence>>>(QUEUE_CAPACITY);
        senders.push(tx);
        receivers.push(rx);
    }

    let mut reader_error: Option<io::Error> = None;
    let mut sequence_count: u64 = 0;

    // thread::scope lets the workers borrow from this stack frame and guarantees
    // they are all joined before the scope returns.
    let processed: Vec<Vec<(usize, Box<dyn QCModule>)>> = thread::scope(|scope| {
        let handles: Vec<_> = groups
            .into_iter()
            .zip(receivers)
            .map(|(mut group, rx)| {
                scope.spawn(move || {
                    // Drain batches until the reader drops its senders (EOF) or
                    // an error closes the channel.
                    while let Ok(batch) = rx.recv() {
                        for seq in batch.iter() {
                            for (_, module) in group.iter_mut() {
                                feed_module(module.as_mut(), seq);
                            }
                        }
                    }
                    group
                })
            })
            .collect();

        // Reader loop runs on the calling thread: pull records, fill a batch,
        // and publish it to every worker queue.
        let mut batch: Vec<Sequence> = Vec::with_capacity(BATCH_SIZE);
        'read: loop {
            match seq_file.next() {
                Some(Ok(seq)) => {
                    batch.push(seq);
                    if batch.len() == BATCH_SIZE {
                        let full = std::mem::replace(&mut batch, Vec::with_capacity(BATCH_SIZE));
                        let shared = Arc::new(full);
                        for tx in &senders {
                            // A send error means a worker panicked and dropped
                            // its receiver; stop reading so we don't deadlock on
                            // a full queue, and surface the panic via join below.
                            if tx.send(Arc::clone(&shared)).is_err() {
                                break 'read;
                            }
                        }

                        sequence_count += BATCH_SIZE as u64;
                        if file_progress.wants_position() {
                            file_progress.update(seq_file.percent_complete(), sequence_count);
                        }
                    }
                }
                Some(Err(e)) => {
                    reader_error = Some(io::Error::new(io::ErrorKind::InvalidData, e));
                    break;
                }
                None => break, // EOF
            }
        }

        // Publish the final partial batch (unless a read error aborted the run).
        if reader_error.is_none() && !batch.is_empty() {
            sequence_count += batch.len() as u64;
            let shared = Arc::new(batch);
            for tx in &senders {
                let _ = tx.send(Arc::clone(&shared));
            }
        }

        // Dropping the senders signals EOF; workers exit their recv loop and
        // return their (now fully-processed) module group.
        drop(senders);

        handles
            .into_iter()
            .map(|h| h.join().expect("analysis worker thread panicked"))
            .collect()
    });

    if let Some(e) = reader_error {
        return Err(e);
    }

    // Reassemble the modules in their original report order.
    let mut rebuilt: Vec<(usize, Box<dyn QCModule>)> = processed.into_iter().flatten().collect();
    rebuilt.sort_by_key(|(idx, _)| *idx);
    Ok((
        rebuilt.into_iter().map(|(_, module)| module).collect(),
        sequence_count,
    ))
}

/// Strip known sequencing file extensions from a filename.
///
/// Matches the exact chain of replaceAll calls in OfflineRunner.java:181
fn strip_extensions(name: &str) -> String {
    let mut result = name.to_string();
    // Strip in this exact order, matching Java's replaceAll chain
    for ext in &[
        ".gz", ".bz2", ".txt", ".fastq", ".fq", ".csfastq", ".sam", ".bam", ".ubam", ".fast5",
    ] {
        if result.ends_with(ext) {
            result = result[..result.len() - ext.len()].to_string();
        }
    }
    result
}

/// Recursively find all .fast5 files within a directory.
///
/// In --nano mode, Java's OfflineRunner recursively searches directories
/// for .fast5 files to process.
fn find_fast5_files(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    find_fast5_files_recursive(dir, &mut files)?;
    files.sort(); // Deterministic ordering
    Ok(files)
}

fn find_fast5_files_recursive(dir: &Path, files: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            find_fast5_files_recursive(&path, files)?;
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("fast5"))
        {
            files.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ModuleStatus;

    /// An in-memory SequenceFile that replays a fixed list of sequences, so the
    /// analysis paths can be exercised without touching the filesystem.
    struct MockSeqFile {
        seqs: Vec<Sequence>,
        pos: usize,
    }

    impl SequenceFile for MockSeqFile {
        fn next(&mut self) -> Option<io::Result<Sequence>> {
            if self.pos < self.seqs.len() {
                let s = self.seqs[self.pos].clone();
                self.pos += 1;
                Some(Ok(s))
            } else {
                None
            }
        }
        fn name(&self) -> &str {
            "mock.fastq"
        }
        fn is_colorspace(&self) -> bool {
            false
        }
        fn percent_complete(&self) -> f64 {
            if self.seqs.is_empty() {
                100.0
            } else {
                (self.pos as f64 / self.seqs.len() as f64) * 100.0
            }
        }
    }

    /// Build a deterministic but varied set of sequences: a rotating base
    /// pattern, a recurring adapter-like motif, and a handful of exact
    /// duplicates, so the quality, content, adapter, overrepresented and
    /// duplication modules all have something non-trivial to accumulate.
    fn make_test_sequences(count: usize, len: usize) -> Vec<Sequence> {
        let alphabet = b"ACGTACGTGGCCATN";
        let adapter = b"AGATCGGAAGAGC";
        let mut seqs = Vec::with_capacity(count);
        for i in 0..count {
            let mut bases = vec![0u8; len];
            for (p, b) in bases.iter_mut().enumerate() {
                *b = alphabet[(i * 7 + p * 3) % alphabet.len()];
            }
            // Splice an adapter motif near the 3' end of every 12th read.
            if i % 12 == 0 && len > adapter.len() + 5 {
                let start = len - adapter.len() - (i % 5);
                bases[start..start + adapter.len()].copy_from_slice(adapter);
            }
            // Force some exact duplicates for the overrepresented/duplication path.
            if i % 50 == 0 {
                for (p, b) in bases.iter_mut().enumerate() {
                    *b = b"ACGT"[p % 4];
                }
            }
            // Quality declines toward the 3' end, clamped to a valid range.
            let quality: Vec<u8> = (0..len)
                .map(|p| {
                    let q = 40i32 - (18 * p as i32) / len as i32;
                    33 + q.clamp(2, 40) as u8
                })
                .collect();
            seqs.push(Sequence::new(format!("READ{}", i), bases, quality));
        }
        seqs
    }

    /// The parallel analysis pipeline must produce output that is byte-identical
    /// to the single-threaded path for every module, across a range of processor
    /// counts (which change how modules are distributed across worker threads).
    #[test]
    fn test_parallel_matches_sequential() {
        let config = FastQCConfig::default();
        let limits = config.load_limits().expect("load limits");
        let seqs = make_test_sequences(4000, 100);

        // Reference: single-threaded path.
        let mut mods_seq = modules::create_modules(&config, &limits);
        for m in mods_seq.iter_mut() {
            m.set_filename("mock.fastq");
        }
        let reporter = progress::ProgressReporter::hidden();
        let mut seq_file = MockSeqFile {
            seqs: seqs.clone(),
            pos: 0,
        };
        let sequential_reads =
            process_sequences_sequential(&mut seq_file, &mut mods_seq, reporter.file(0))
                .expect("sequential run");
        assert_eq!(sequential_reads, seqs.len() as u64);
        for m in mods_seq.iter_mut() {
            m.finalize();
        }
        let reference: Vec<(String, Vec<u8>, ModuleStatus)> = mods_seq
            .iter()
            .map(|m| {
                let mut buf = Vec::new();
                m.write_text_report(&mut buf).expect("text report");
                (m.name().to_string(), buf, m.status())
            })
            .collect();

        for num_processors in 1..=MAX_PROCESSORS_PER_FILE {
            let mut mods_par = modules::create_modules(&config, &limits);
            for m in mods_par.iter_mut() {
                m.set_filename("mock.fastq");
            }
            let mut seq_file = MockSeqFile {
                seqs: seqs.clone(),
                pos: 0,
            };
            let (mut mods_par, parallel_reads) = process_sequences_parallel(
                &mut seq_file,
                mods_par,
                num_processors,
                reporter.file(0),
            )
            .expect("parallel run");
            assert_eq!(parallel_reads, sequential_reads);
            for m in mods_par.iter_mut() {
                m.finalize();
            }

            assert_eq!(mods_par.len(), reference.len());
            for (module, (name, ref_text, ref_status)) in mods_par.iter().zip(&reference) {
                // Module order must be preserved after the round-robin split.
                assert_eq!(module.name(), name, "module order changed");
                let mut buf = Vec::new();
                module.write_text_report(&mut buf).expect("text report");
                assert_eq!(
                    &buf, ref_text,
                    "module `{}` text report differs with {} processor(s)",
                    name, num_processors
                );
                assert_eq!(
                    module.status(),
                    *ref_status,
                    "module `{}` status differs with {} processor(s)",
                    name,
                    num_processors
                );
            }
        }
    }

    #[test]
    fn test_strip_extensions() {
        assert_eq!(strip_extensions("sample.fastq"), "sample");
        assert_eq!(strip_extensions("sample.fastq.gz"), "sample");
        assert_eq!(strip_extensions("sample.fq.bz2"), "sample");
        assert_eq!(strip_extensions("sample.bam"), "sample");
        assert_eq!(strip_extensions("sample.sam"), "sample");
        assert_eq!(strip_extensions("sample.txt.gz"), "sample");
        assert_eq!(strip_extensions("minimal.fastq"), "minimal");
    }

    #[test]
    fn test_build_file_groups_default() {
        let config = FastQCConfig::default();
        let files = vec![PathBuf::from("a.fastq"), PathBuf::from("b.fastq")];
        let groups = build_file_groups(&config, &files);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].name, "a.fastq");
        assert_eq!(groups[0].files.len(), 1);
        assert_eq!(groups[1].name, "b.fastq");
        assert_eq!(groups[1].files.len(), 1);
    }

    #[test]
    fn test_build_file_groups_casava() {
        let config = FastQCConfig {
            casava: true,
            ..FastQCConfig::default()
        };
        let files = vec![
            PathBuf::from("Sample_S1_L001_R1_001.fastq.gz"),
            PathBuf::from("Sample_S1_L001_R1_002.fastq.gz"),
            PathBuf::from("Other_S2_L001_R1_001.fastq.gz"),
        ];
        let groups = build_file_groups(&config, &files);
        assert_eq!(groups.len(), 2);

        // Find the Sample group
        let sample_group = groups
            .iter()
            .find(|g| g.name == "Sample_S1_L001_R1.fastq.gz")
            .unwrap();
        assert_eq!(sample_group.files.len(), 2);

        // Find the Other group
        let other_group = groups
            .iter()
            .find(|g| g.name == "Other_S2_L001_R1.fastq.gz")
            .unwrap();
        assert_eq!(other_group.files.len(), 1);
    }

    #[test]
    fn test_build_file_groups_stdin() {
        let config = FastQCConfig::default();
        let files = vec![PathBuf::from("stdin")];
        let groups = build_file_groups(&config, &files);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "stdin");
    }
}
