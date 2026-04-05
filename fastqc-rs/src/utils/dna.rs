// DNA sequence utilities shared across modules.

/// Complement a single DNA base (uppercase).
///
/// Matches the complement logic used in both BAMFile.java's
/// reverseComplement() and Contaminant.java's reverse complement computation.
/// Non-ACGT bases (like N) are returned unchanged.
pub fn complement_base(base: u8) -> u8 {
    match base {
        b'A' => b'T',
        b'T' => b'A',
        b'C' => b'G',
        b'G' => b'C',
        other => other,
    }
}

/// Reverse complement a DNA sequence.
///
/// Replicates reverseComplement() from BAMFile.java and the
/// reverse complement computation in Contaminant.java. Input is assumed to
/// be uppercase (or is uppercased before complementing).
pub fn reverse_complement(seq: &[u8]) -> Vec<u8> {
    seq.iter()
        .rev()
        .map(|&b| complement_base(b.to_ascii_uppercase()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_complement_base() {
        assert_eq!(complement_base(b'A'), b'T');
        assert_eq!(complement_base(b'T'), b'A');
        assert_eq!(complement_base(b'C'), b'G');
        assert_eq!(complement_base(b'G'), b'C');
        assert_eq!(complement_base(b'N'), b'N');
    }

    #[test]
    fn test_reverse_complement_basic() {
        assert_eq!(reverse_complement(b"ACGT"), b"ACGT");
        assert_eq!(reverse_complement(b"AAAA"), b"TTTT");
        assert_eq!(reverse_complement(b"CCCC"), b"GGGG");
        assert_eq!(reverse_complement(b"ATCG"), b"CGAT");
    }

    #[test]
    fn test_reverse_complement_with_n() {
        // N is kept as N (matches Java's default case)
        assert_eq!(reverse_complement(b"ANCG"), b"CGNT");
    }

    #[test]
    fn test_reverse_complement_empty() {
        assert_eq!(reverse_complement(b""), b"");
    }

    #[test]
    fn test_reverse_complement_single() {
        assert_eq!(reverse_complement(b"A"), b"T");
        assert_eq!(reverse_complement(b"N"), b"N");
    }
}
