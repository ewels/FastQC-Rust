// SequenceFileGroup: reads from multiple SequenceFile objects sequentially
// When Java processes a CASAVA group, it passes File[] to
// SequenceFactory which creates a SequenceFile backed by multiple files.
// We replicate this with a wrapper that iterates through files in order.

use std::io;

use super::{Sequence, SequenceFile};

/// A group of sequence files presented as a single logical stream.
///
/// In Java, when CASAVA grouping produces multiple files for
/// one sample, they are passed as `File[]` to `SequenceFactory.getSequenceFile()`,
/// which creates a single SequenceFile that reads all files sequentially.
/// This struct replicates that behavior by wrapping multiple `SequenceFile`
/// objects and advancing to the next when one is exhausted.
pub struct SequenceFileGroup {
    files: Vec<Box<dyn SequenceFile>>,
    current: usize,
    name: String,
}

impl SequenceFileGroup {
    /// Create a new group with the given display name and sequence files.
    ///
    /// The files are read in order: when the first file reaches EOF, reading
    /// continues from the second file, and so on.
    pub fn new(name: String, files: Vec<Box<dyn SequenceFile>>) -> Self {
        Self {
            files,
            current: 0,
            name,
        }
    }
}

impl SequenceFile for SequenceFileGroup {
    fn next(&mut self) -> Option<io::Result<Sequence>> {
        // Read from the current file. When it returns None (EOF),
        // advance to the next file in the group and try again.
        while self.current < self.files.len() {
            match self.files[self.current].next() {
                Some(result) => return Some(result),
                None => {
                    // Current file exhausted, move to next
                    self.current += 1;
                }
            }
        }
        None // All files exhausted
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn is_colorspace(&self) -> bool {
        // Colorspace is a property of the file format, not the group.
        // Return the colorspace status of the current (or first) file.
        if self.current < self.files.len() {
            self.files[self.current].is_colorspace()
        } else if !self.files.is_empty() {
            self.files[0].is_colorspace()
        } else {
            false
        }
    }

    fn percent_complete(&self) -> f64 {
        if self.files.is_empty() {
            return 100.0;
        }
        // Estimate progress across all files in the group.
        // Weight each file equally (simplification - Java does similar rough estimation).
        let total = self.files.len() as f64;
        let completed = self.current as f64;
        let current_progress = if self.current < self.files.len() {
            self.files[self.current].percent_complete() / 100.0
        } else {
            0.0
        };
        ((completed + current_progress) / total) * 100.0
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

    #[test]
    fn test_group_reads_all_files() {
        let f1: Box<dyn SequenceFile> = Box::new(MockSequenceFile::new("a", 2));
        let f2: Box<dyn SequenceFile> = Box::new(MockSequenceFile::new("b", 3));

        let mut group = SequenceFileGroup::new("test_group".to_string(), vec![f1, f2]);

        let mut count = 0;
        while group.next().is_some() {
            count += 1;
        }
        assert_eq!(count, 5); // 2 + 3
    }

    #[test]
    fn test_group_empty() {
        let mut group = SequenceFileGroup::new("empty".to_string(), vec![]);
        assert!(group.next().is_none());
    }

    #[test]
    fn test_group_name() {
        let group = SequenceFileGroup::new("my_sample".to_string(), vec![]);
        assert_eq!(group.name(), "my_sample");
    }
}
