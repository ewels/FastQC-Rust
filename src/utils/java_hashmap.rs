//! JAVA COMPAT: Java FastQC's output sometimes depends on `java.util.HashMap`
//! iteration order: float sums are order-sensitive and stable sorts keep it for ties.
//!
//! The order is table bucket, then insertion order within a bucket. Callers
//! approximate insertion order by key. Not modelled: Java also doubles a table
//! below 64 buckets when one bucket reaches 9 entries, and turns larger buckets
//! into trees. Both need many colliding keys, which real data rarely produces.

/// Bucket count of a default-constructed `HashMap` once it holds `len` entries:
/// 16, doubling whenever the load exceeds 0.75.
pub fn table_capacity(len: usize) -> usize {
    let mut capacity = 16;
    while len * 4 > capacity * 3 {
        capacity *= 2;
    }
    capacity
}

/// Bucket that a key with Java `hashCode()` `hash` lands in.
pub fn bucket(hash: i32, capacity: usize) -> usize {
    let h = hash as u32;
    ((h ^ (h >> 16)) as usize) & (capacity - 1)
}

/// `Long.hashCode()`
pub fn long_hash(value: u64) -> i32 {
    (value ^ (value >> 32)) as i32
}

/// `String.hashCode()` for ASCII strings.
pub fn string_hash(s: &str) -> i32 {
    s.bytes()
        .fold(0i32, |h, b| h.wrapping_mul(31).wrapping_add(b as i32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_table_capacity() {
        assert_eq!(table_capacity(0), 16);
        assert_eq!(table_capacity(12), 16);
        assert_eq!(table_capacity(13), 32);
        assert_eq!(table_capacity(100_000), 262_144);
    }

    #[test]
    fn test_hashes_match_java() {
        // Values from Java's "...".hashCode() and Long.hashCode(...)
        assert_eq!(string_hash(""), 0);
        assert_eq!(string_hash("ACGT"), 2_003_087);
        assert_eq!(
            string_hash("GATCGGAAGAGCACACGTCTGAACTCCAGTCACATCACGATCTCGTATGC"),
            -615_524_200
        );
        assert_eq!(long_hash(17), 17);
        assert_eq!(long_hash(1 << 32), 1);
    }

    #[test]
    fn test_bucket_spreads_high_bits() {
        assert_eq!(bucket(17, 16), 1);
        assert_eq!(bucket(1 << 16, 16), 1);
    }
}
