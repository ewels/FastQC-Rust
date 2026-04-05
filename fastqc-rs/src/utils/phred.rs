/// Phred quality encoding detection.
///
/// Replicates the logic from `Sequence/QualityEncoding/PhredEncoding.java`.

#[derive(Debug)]
pub struct PhredEncoding {
    pub name: &'static str,
    pub offset: u8,
}

// These constants match the Java SANGER_ENCODING_OFFSET and
// ILLUMINA_1_3_ENCODING_OFFSET fields exactly.
const SANGER_ENCODING_OFFSET: u8 = 33;
const ILLUMINA_1_3_ENCODING_OFFSET: u8 = 64;

/// Detect the Phred encoding from the lowest ASCII character seen in quality strings.
///
/// Returns the encoding name and offset, or an error if the character is out of range.
///
/// Replicates `PhredEncoding.getFastQEncodingOffset(char)` exactly,
/// including the boundary conditions at 33, 64, 65, and 126.
pub fn detect(lowest_char: u8) -> Result<PhredEncoding, String> {
    if lowest_char < 33 {
        // Java error message format preserved
        Err(format!(
            "No known encodings with chars < 33 (Yours was '{}' with value {})",
            lowest_char as char, lowest_char
        ))
    } else if lowest_char < 64 {
        Ok(PhredEncoding {
            name: "Sanger / Illumina 1.9",
            offset: SANGER_ENCODING_OFFSET,
        })
    } else if lowest_char == ILLUMINA_1_3_ENCODING_OFFSET + 1 {
        // Java checks `== 65` (offset 64 + 1) specifically for Illumina 1.3,
        // which allowed quality value 1 (ASCII 65). From v1.5 onward the minimum was 2.
        Ok(PhredEncoding {
            name: "Illumina 1.3",
            offset: ILLUMINA_1_3_ENCODING_OFFSET,
        })
    } else if lowest_char <= 126 {
        Ok(PhredEncoding {
            name: "Illumina 1.5",
            offset: ILLUMINA_1_3_ENCODING_OFFSET,
        })
    } else {
        // Java error message format preserved
        Err(format!(
            "No known encodings with chars > 126 (Yours was {} with value {})",
            lowest_char as char, lowest_char
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanger_encoding() {
        let enc = detect(b'!').unwrap(); // ASCII 33
        assert_eq!(enc.name, "Sanger / Illumina 1.9");
        assert_eq!(enc.offset, 33);
    }

    #[test]
    fn test_sanger_high_boundary() {
        let enc = detect(63).unwrap(); // just below 64
        assert_eq!(enc.name, "Sanger / Illumina 1.9");
        assert_eq!(enc.offset, 33);
    }

    #[test]
    fn test_illumina_1_3() {
        let enc = detect(65).unwrap(); // exactly 65
        assert_eq!(enc.name, "Illumina 1.3");
        assert_eq!(enc.offset, 64);
    }

    #[test]
    fn test_illumina_1_5() {
        let enc = detect(66).unwrap();
        assert_eq!(enc.name, "Illumina 1.5");
        assert_eq!(enc.offset, 64);
    }

    #[test]
    fn test_illumina_1_5_at_126() {
        let enc = detect(126).unwrap();
        assert_eq!(enc.name, "Illumina 1.5");
        assert_eq!(enc.offset, 64);
    }

    #[test]
    fn test_error_below_33() {
        let err = detect(20).unwrap_err();
        assert!(err.contains("< 33"));
    }

    #[test]
    fn test_error_above_126() {
        let err = detect(127).unwrap_err();
        assert!(err.contains("> 126"));
    }
}
