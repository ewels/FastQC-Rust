// SequenceFileGroup: reads from multiple SequenceFile objects sequentially
// When Java processes a CASAVA group, it passes File[] to
// SequenceFactory which creates a SequenceFile backed by multiple files.
// We replicate this with a wrapper that iterates through files in order.

use std::collections::VecDeque;
use std::io;

use super::{Sequence, SequenceFile};

/// Opens one member of a [`SequenceFileGroup`] when its turn comes.
pub type FileOpener = Box<dyn FnOnce() -> io::Result<Box<dyn SequenceFile>> + Send>;

/// A group of sequence files presented as a single logical stream.
///
/// In Java, when CASAVA grouping produces multiple files for
/// one sample, they are passed as `File[]` to `SequenceFactory.getSequenceFile()`,
/// which creates a single SequenceFile that reads all files sequentially.
/// This struct replicates that behavior by advancing to the next file when one
/// is exhausted.
///
/// Each file is opened only when the previous one is exhausted: opening a
/// `.gz` starts its decoder thread reading ahead, which for a sample split
/// into many chunks would otherwise mean a decoder and its buffers per chunk,
/// all idle but one.
pub struct SequenceFileGroup {
    current: Option<Box<dyn SequenceFile>>,
    pending: VecDeque<FileOpener>,
    /// Files fully read, for `percent_complete`.
    finished: usize,
    total: usize,
    name: String,
}

impl SequenceFileGroup {
    /// Create a group with the given display name, reading the files the
    /// `openers` open in order. The first is opened straight away, so a file
    /// that cannot be read at all fails here rather than mid-run.
    pub fn new(name: String, openers: Vec<FileOpener>) -> io::Result<Self> {
        let total = openers.len();
        let mut pending: VecDeque<FileOpener> = openers.into();
        let current = pending.pop_front().map(|open| open()).transpose()?;
        Ok(Self {
            current,
            pending,
            finished: 0,
            total,
            name,
        })
    }
}

impl SequenceFile for SequenceFileGroup {
    fn next(&mut self) -> Option<io::Result<Sequence>> {
        loop {
            if let Some(file) = self.current.as_mut() {
                if let Some(result) = file.next() {
                    return Some(result);
                }
                self.current = None;
                self.finished += 1;
            }
            match self.pending.pop_front()?() {
                Ok(file) => self.current = Some(file),
                Err(e) => {
                    self.finished += 1;
                    return Some(Err(e));
                }
            }
        }
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_colorspace(&self) -> bool {
        // Colorspace is a property of the file format, not the group.
        self.current
            .as_ref()
            .is_some_and(|file| file.is_colorspace())
    }

    fn percent_complete(&self) -> f64 {
        if self.total == 0 {
            return 100.0;
        }
        // Weight each file equally (simplification - Java does similar rough estimation).
        let current = self
            .current
            .as_ref()
            .map_or(0.0, |file| file.percent_complete() / 100.0);
        ((self.finished as f64 + current) / self.total as f64) * 100.0
    }

    fn background_threads(&self) -> usize {
        self.current
            .as_ref()
            .map_or(0, |file| file.background_threads())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A trivial SequenceFile for testing that yields a fixed number of sequences.
    struct MockSequenceFile {
        remaining: usize,
        name: String,
    }

    impl MockSequenceFile {
        fn new(name: &str, count: usize) -> Self {
            Self {
                remaining: count,
                name: name.to_string(),
            }
        }
    }

    impl SequenceFile for MockSequenceFile {
        fn next(&mut self) -> Option<io::Result<Sequence>> {
            if self.remaining == 0 {
                return None;
            }
            self.remaining -= 1;
            Some(Ok(Sequence::new(
                format!("@read_{}", self.remaining),
                b"ACGT".to_vec(),
                b"IIII".to_vec(),
            )))
        }

        fn name(&self) -> &str {
            &self.name
        }

        fn is_colorspace(&self) -> bool {
            false
        }

        fn percent_complete(&self) -> f64 {
            0.0
        }
    }

    fn opener(name: &'static str, count: usize) -> FileOpener {
        Box::new(move || Ok(Box::new(MockSequenceFile::new(name, count)) as Box<dyn SequenceFile>))
    }

    #[test]
    fn test_group_reads_all_files() {
        let mut group = SequenceFileGroup::new(
            "test_group".to_string(),
            vec![opener("a", 2), opener("b", 3)],
        )
        .unwrap();

        let mut count = 0;
        while group.next().is_some() {
            count += 1;
        }
        assert_eq!(count, 5); // 2 + 3
    }

    /// Only the file being read is open; the next is opened once the one
    /// before it is exhausted.
    #[test]
    fn test_group_opens_files_one_at_a_time() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let opened = Arc::new(AtomicUsize::new(0));
        let counting = |count: usize| -> FileOpener {
            let opened = Arc::clone(&opened);
            Box::new(move || {
                opened.fetch_add(1, Ordering::Relaxed);
                Ok(Box::new(MockSequenceFile::new("f", count)) as Box<dyn SequenceFile>)
            })
        };
        let mut group =
            SequenceFileGroup::new("g".to_string(), vec![counting(2), counting(2), counting(2)])
                .unwrap();
        assert_eq!(opened.load(Ordering::Relaxed), 1);
        group.next();
        group.next();
        assert_eq!(opened.load(Ordering::Relaxed), 1);
        group.next();
        assert_eq!(opened.load(Ordering::Relaxed), 2);
        while group.next().is_some() {}
        assert_eq!(opened.load(Ordering::Relaxed), 3);
        assert_eq!(group.percent_complete(), 100.0);
    }

    #[test]
    fn test_group_empty() {
        let mut group = SequenceFileGroup::new("empty".to_string(), vec![]).unwrap();
        assert!(group.next().is_none());
    }

    #[test]
    fn test_group_name() {
        let group = SequenceFileGroup::new("my_sample".to_string(), vec![]).unwrap();
        assert_eq!(group.name(), "my_sample");
    }
}
