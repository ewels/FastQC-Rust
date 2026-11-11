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

/// Iteration order of a `java.util.Hashtable` built with `initial_capacity`
/// and the default 0.75 load factor, after inserting `keys` in order (no
/// duplicates). Returns indices into `keys`.
///
/// Unlike `HashMap`, `Hashtable` takes the raw hash modulo its capacity,
/// prepends to a bucket's chain, grows to `2n + 1` buckets, and iterates
/// buckets from the last to the first.
pub fn hashtable_order(keys: &[&str], initial_capacity: usize) -> Vec<usize> {
    let threshold = |capacity: usize| (capacity as f32 * 0.75) as usize;
    let index =
        |key: usize, capacity: usize| (string_hash(keys[key]) & 0x7FFF_FFFF) as usize % capacity;

    // Each chain is stored tail first, so its head (newest entry) is last.
    let mut capacity = initial_capacity.max(1);
    let mut table: Vec<Vec<usize>> = vec![Vec::new(); capacity];
    for key in 0..keys.len() {
        if key >= threshold(capacity) {
            let new_capacity = capacity * 2 + 1;
            let mut grown = vec![Vec::new(); new_capacity];
            for chain in table.iter().rev() {
                for &entry in chain.iter().rev() {
                    grown[index(entry, new_capacity)].push(entry);
                }
            }
            table = grown;
            capacity = new_capacity;
        }
        table[index(key, capacity)].push(key);
    }

    table
        .iter()
        .rev()
        .flat_map(|chain| chain.iter().rev().copied())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hashtable_order() {
        // Expected values printed by Java 17's Hashtable.values() after
        // putting the keys in order.
        let keys = ["AA", "AC", "CA", "GG"];
        assert_eq!(hashtable_order(&keys, 11), vec![2, 3, 1, 0]);
        // Capacity 4 grows to 9 before the 4th key (threshold 3).
        assert_eq!(hashtable_order(&keys, 4), vec![3, 1, 0, 2]);
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
