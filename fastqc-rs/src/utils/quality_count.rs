/// Per-position quality score accumulator.
///
/// Replicates the logic from `Utilities/QualityCount.java`.
/// Accumulates quality score counts for a single read position.
///
/// JAVA COMPAT: Uses a fixed 150-slot array indexed by ASCII value, matching
/// the Java `long[] actualCounts = new long[150]` exactly.
pub struct QualityCount {
    actual_counts: [u64; 150],
    total_counts: u64,
}

impl QualityCount {
    pub fn new() -> Self {
        QualityCount {
            actual_counts: [0u64; 150],
            total_counts: 0,
        }
    }

    /// Record a quality character (raw ASCII value, not offset-adjusted).
    ///
    /// Matches `addValue(char c)` which indexes by `(int)c`.
    pub fn add_value(&mut self, quality_char: u8) {
        let idx = quality_char as usize;
        if idx >= self.actual_counts.len() {
            // Java throws ArrayIndexOutOfBoundsException here, crashing
            // the run. We clamp to the last slot instead so that corrupt quality chars
            // don't abort the entire analysis -- the value will be wrong for that
            // position, but the rest of the file can still be processed.
            eprintln!(
                "Warning: quality character '{}' (ASCII {}) exceeds maximum {}; clamping",
                quality_char as char, idx, self.actual_counts.len() - 1
            );
            self.actual_counts[self.actual_counts.len() - 1] += 1;
            self.total_counts += 1;
            return;
        }
        self.actual_counts[idx] += 1;
        self.total_counts += 1;
    }

    pub fn get_total_count(&self) -> u64 {
        self.total_counts
    }

    /// Lowest ASCII character with a non-zero count.
    pub fn get_min_char(&self) -> Option<u8> {
        for i in 0..self.actual_counts.len() {
            if self.actual_counts[i] > 0 {
                return Some(i as u8);
            }
        }
        None
    }

    /// Highest ASCII character with a non-zero count.
    pub fn get_max_char(&self) -> Option<u8> {
        for i in (0..self.actual_counts.len()).rev() {
            if self.actual_counts[i] > 0 {
                return Some(i as u8);
            }
        }
        None
    }

    /// Weighted mean quality score (offset-adjusted).
    ///
    /// Matches `getMean(int offset)`. Iterates from `offset` to 149,
    /// accumulating `count * (index - offset)` and dividing by total count.
    /// Returns NaN when count is 0 (Java's 0.0/0 behaviour).
    pub fn get_mean(&self, offset: u8) -> f64 {
        let mut total: u64 = 0;
        let mut count: u64 = 0;
        let off = offset as usize;

        for i in off..self.actual_counts.len() {
            total += self.actual_counts[i] * (i - off) as u64;
            count += self.actual_counts[i];
        }

        // JAVA COMPAT: Java returns ((double)total)/count which is NaN when count == 0.
        total as f64 / count as f64
    }

    /// Percentile quality score (offset-adjusted).
    ///
    /// Matches `getPercentile(int offset, int percentile)` exactly,
    /// including the use of integer arithmetic for threshold calculation:
    ///   total = totalCounts * percentile / 100   (integer division)
    /// This is critical for byte-exact output matching.
    pub fn get_percentile(&self, offset: u8, percentile: u8) -> f64 {
        // JAVA COMPAT: Java uses `long total = totalCounts; total *= percentile; total /= 100;`
        // which is integer multiplication then integer division (truncating).
        let mut threshold: u64 = self.total_counts;
        threshold *= percentile as u64;
        threshold /= 100;

        let off = offset as usize;
        let mut count: u64 = 0;

        for i in off..self.actual_counts.len() {
            count += self.actual_counts[i];
            if count >= threshold {
                // JAVA COMPAT: Java returns `(char)(i - offset)` cast to double.
                // The char-to-double cast in Java just gives the numeric value.
                return (i - off) as f64;
            }
        }

        // JAVA COMPAT: Java returns -1 when no value found (cast to double = -1.0).
        -1.0
    }
}

/// Find the global minimum and maximum quality characters across any iterable
/// of QualityCount values (owned or references).
///
/// Replicates the `calculateOffsets()` pattern used in
/// PerBaseQualityScores.java and PerTileQualityScores.java.
/// Returns (0, 0) if no quality data has been recorded.
///
/// Accepts `&[QualityCount]`, `&[&QualityCount]`, or any iterator that yields
/// items borrowable as `&QualityCount`.
pub fn calculate_offsets<I, Q>(counts: I) -> (u8, u8)
where
    I: IntoIterator<Item = Q>,
    Q: std::borrow::Borrow<QualityCount>,
{
    let mut min_char: u8 = 0;
    let mut max_char: u8 = 0;
    let mut first = true;

    for item in counts {
        let qc = item.borrow();
        if first {
            if let (Some(lo), Some(hi)) = (qc.get_min_char(), qc.get_max_char()) {
                min_char = lo;
                max_char = hi;
                first = false;
            }
        } else {
            if let Some(mc) = qc.get_min_char() {
                if mc < min_char {
                    min_char = mc;
                }
            }
            if let Some(mc) = qc.get_max_char() {
                if mc > max_char {
                    max_char = mc;
                }
            }
        }
    }

    (min_char, max_char)
}

impl Default for QualityCount {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_counts() {
        let qc = QualityCount::new();
        assert_eq!(qc.get_total_count(), 0);
        assert!(qc.get_min_char().is_none());
        assert!(qc.get_max_char().is_none());
        assert!(qc.get_mean(33).is_nan());
    }

    #[test]
    fn test_single_value() {
        let mut qc = QualityCount::new();
        // ASCII 73 = 'I', with Sanger offset 33 this is quality 40
        qc.add_value(b'I');
        assert_eq!(qc.get_total_count(), 1);
        assert_eq!(qc.get_min_char(), Some(b'I'));
        assert_eq!(qc.get_max_char(), Some(b'I'));
        assert!((qc.get_mean(33) - 40.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_multiple_values() {
        let mut qc = QualityCount::new();
        // Add quality chars for Sanger: offset 33
        // ASCII 53 = '5' -> quality 20
        // ASCII 63 = '?' -> quality 30
        qc.add_value(53);
        qc.add_value(63);
        assert_eq!(qc.get_total_count(), 2);
        assert_eq!(qc.get_min_char(), Some(53));
        assert_eq!(qc.get_max_char(), Some(63));
        // Mean = (20 + 30) / 2 = 25.0
        assert!((qc.get_mean(33) - 25.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_percentile_integer_division() {
        // This test verifies that the integer division behavior matches Java.
        // With 1 count: threshold = 1 * 25 / 100 = 0 (integer division).
        // The loop starts at offset (33). At i=33, count = actual_counts[33] = 0.
        // Since 0 >= 0 is true, Java returns (char)(33-33) = (char)0 = 0.0.
        // This is an artifact of the integer division producing threshold=0.
        let mut qc = QualityCount::new();
        qc.add_value(b'I'); // ASCII 73
        let p25 = qc.get_percentile(33, 25);
        // threshold = 1 * 25 / 100 = 0 in integer division
        // First iteration: i=33, count=0, 0 >= 0 -> returns (33-33) = 0.0
        assert!((p25 - 0.0).abs() < f64::EPSILON);

        // The 100th percentile still works correctly:
        // threshold = 1 * 100 / 100 = 1
        // Accumulates until i=73 where count=1, 1 >= 1 -> returns (73-33) = 40.0
        let p100 = qc.get_percentile(33, 100);
        assert!((p100 - 40.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_percentile_multiple() {
        let mut qc = QualityCount::new();
        // 3 values at quality 20 (ASCII 53), 7 values at quality 30 (ASCII 63)
        for _ in 0..3 {
            qc.add_value(53);
        }
        for _ in 0..7 {
            qc.add_value(63);
        }
        // Median (50th percentile): threshold = 10 * 50 / 100 = 5
        // Count at 53: 3 (< 5), count at 63: 10 (>= 5) -> quality 30
        assert!((qc.get_percentile(33, 50) - 30.0).abs() < f64::EPSILON);

        // 25th percentile: threshold = 10 * 25 / 100 = 2
        // Count at 53: 3 (>= 2) -> quality 20
        assert!((qc.get_percentile(33, 25) - 20.0).abs() < f64::EPSILON);

        // 90th percentile: threshold = 10 * 90 / 100 = 9
        // Count at 53: 3 (< 9), count at 63: 10 (>= 9) -> quality 30
        assert!((qc.get_percentile(33, 90) - 30.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_add_value_overflow_clamps() {
        let mut qc = QualityCount::new();
        // Values >= 150 are clamped to the last slot instead of panicking
        qc.add_value(200);
        assert_eq!(qc.get_total_count(), 1);
        // The count should be in the last slot (index 149)
        assert_eq!(qc.get_max_char(), Some(149));
    }
}
