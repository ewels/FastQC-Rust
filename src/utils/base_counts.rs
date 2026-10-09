/// Lookup table: maps ASCII byte to base index.
/// 0=A, 1=C, 2=G, 3=T, 4=N, 5=other
///
/// Using a lookup table instead of match/if-else eliminates branch misprediction
/// on random DNA data (where each branch has ~25% probability, the worst case
/// for branch predictors).
pub const BASE_INDEX: [u8; 256] = {
    let mut table = [5u8; 256];
    table[b'A' as usize] = 0;
    table[b'C' as usize] = 1;
    table[b'G' as usize] = 2;
    table[b'T' as usize] = 3;
    table[b'N' as usize] = 4;
    table
};

/// Index constants for readability.
pub const IDX_A: usize = 0;
pub const IDX_C: usize = 1;
pub const IDX_G: usize = 2;
pub const IDX_T: usize = 3;
pub const IDX_N: usize = 4;

/// Counts of uppercase `A`, `C`, `G` and `T` bytes.
///
/// Per-chunk byte counters let all four counts vectorise; indexing a counter
/// array per base is a serial dependency chain.
pub fn count_acgt(seq: &[u8]) -> [u64; 4] {
    let mut totals = [0u64; 4];
    for chunk in seq.chunks(u8::MAX as usize) {
        let mut counts = [0u8; 4];
        for &b in chunk {
            counts[0] += (b == b'A') as u8;
            counts[1] += (b == b'C') as u8;
            counts[2] += (b == b'G') as u8;
            counts[3] += (b == b'T') as u8;
        }
        for (total, count) in totals.iter_mut().zip(counts) {
            *total += count as u64;
        }
    }
    totals
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count_acgt_matches_lookup_table() {
        // Lengths either side of the 255-byte chunk, with N and other bytes.
        for len in [0, 1, 254, 255, 256, 600, 10_000] {
            let seq: Vec<u8> = (0..len).map(|i| b"ACGTNACGGX."[i * 7 % 11]).collect();
            let mut expected = [0u64; 6];
            for &b in &seq {
                expected[BASE_INDEX[b as usize] as usize] += 1;
            }
            assert_eq!(count_acgt(&seq), expected[..4], "length {len}");
        }
    }
}
