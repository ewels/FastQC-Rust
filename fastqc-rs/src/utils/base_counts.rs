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
