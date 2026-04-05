/// Java Double.toString() compatible formatting.
///
/// This module provides `java_format_double` which produces output matching
/// Java's `Double.toString(double)` for the value ranges encountered in FastQC
/// output. This is critical for byte-exact output matching.
/// Format an f64 value to match Java's `Double.toString(double)` output.
///
/// JAVA COMPAT: Java.s Double.toString always includes at least one digit after
/// the decimal point (e.g., "9.0" not "9"), uses "NaN" (capital N-a-N), and
/// switches to scientific notation with uppercase "E" for very large/small values.
///
/// Key differences from Rust's default Display:
/// - Rust prints `9` for 9.0f64; Java prints "9.0"
/// - Rust prints `NaN`; Java prints "NaN" (same, fortunately)
/// - Rust prints `inf`; Java prints "Infinity"
/// - Rust prints `-inf`; Java prints "-Infinity"
pub fn java_format_double(v: f64) -> String {
    // Handle special cases
    if v.is_nan() {
        return "NaN".to_string();
    }
    if v.is_infinite() {
        return if v.is_sign_positive() {
            // JAVA COMPAT: Java uses "Infinity" not "inf"
            "Infinity".to_string()
        } else {
            "-Infinity".to_string()
        };
    }

    // JAVA COMPAT: Java.s Double.toString(-0.0) returns "-0.0"
    if v == 0.0 && v.is_sign_negative() {
        return "-0.0".to_string();
    }

    let abs = v.abs();

    // JAVA COMPAT: Java uses scientific notation for |v| >= 1e7 or (|v| < 1e-3 and |v| > 0).
    // For values in FastQC output, this is rarely needed, but we handle it for correctness.
    if abs != 0.0 && !(1e-3..1e7).contains(&abs) {
        return format_scientific(v);
    }

    // For normal range values, format with Rust and ensure decimal point is present.
    let s = format!("{}", v);

    // JAVA COMPAT: Rust.s Display for f64 omits ".0" for integer-valued doubles.
    // Java always includes at least one decimal digit.
    if s.contains('.') {
        s
    } else {
        format!("{}.0", s)
    }
}

/// Format a value in Java-style scientific notation.
///
/// JAVA COMPAT: Java uses uppercase "E" and formats the exponent without leading
/// zeros (e.g., "1.5E-4" not "1.5e-04"). The mantissa uses the shortest
/// representation that uniquely identifies the double, but always includes at
/// least one digit after the decimal point (e.g., "1.0E7" not "1E7").
fn format_scientific(v: f64) -> String {
    // Use Rust's {:e} format then fixup to match Java conventions.
    let s = format!("{:e}", v);

    // Rust uses lowercase 'e'; Java uses uppercase 'E'.
    let s = s.replace('e', "E");

    // Parse mantissa and exponent to reformat both.
    if let Some(e_pos) = s.find('E') {
        let (mantissa, exp_part) = s.split_at(e_pos);
        let exp_str = &exp_part[1..]; // skip 'E'

        // JAVA COMPAT: Ensure mantissa has a decimal point (e.g., "1.0" not "1").
        let mantissa = if mantissa.contains('.') {
            mantissa.to_string()
        } else {
            format!("{}.0", mantissa)
        };

        // JAVA COMPAT: No leading zeros on exponent (e.g., "E7" not "E07").
        if let Ok(exp_val) = exp_str.parse::<i32>() {
            return format!("{}E{}", mantissa, exp_val);
        }
    }

    s
}

#[cfg(test)]
mod tests {
    use super::*;

    // Values directly observed in FastQC approved output files

    #[test]
    fn test_integer_values() {
        assert_eq!(java_format_double(9.0), "9.0");
        assert_eq!(java_format_double(100.0), "100.0");
        assert_eq!(java_format_double(0.0), "0.0");
        assert_eq!(java_format_double(1.0), "1.0");
        assert_eq!(java_format_double(5.0), "5.0");
        assert_eq!(java_format_double(20.0), "20.0");
    }

    #[test]
    fn test_fractional_values() {
        assert_eq!(java_format_double(0.5), "0.5");
        assert_eq!(java_format_double(2.5), "2.5");
    }

    #[test]
    fn test_nan() {
        assert_eq!(java_format_double(f64::NAN), "NaN");
    }

    #[test]
    fn test_infinity() {
        // JAVA COMPAT: Java uses "Infinity" not "inf"
        assert_eq!(java_format_double(f64::INFINITY), "Infinity");
        assert_eq!(java_format_double(f64::NEG_INFINITY), "-Infinity");
    }

    #[test]
    fn test_negative_zero() {
        // JAVA COMPAT: Java preserves the sign of negative zero
        assert_eq!(java_format_double(-0.0), "-0.0");
    }

    #[test]
    fn test_negative_values() {
        assert_eq!(java_format_double(-1.0), "-1.0");
        assert_eq!(java_format_double(-0.5), "-0.5");
    }

    #[test]
    fn test_scientific_large() {
        // JAVA COMPAT: Java switches to scientific notation at >= 1e7
        assert_eq!(java_format_double(1e7), "1.0E7");
        assert_eq!(java_format_double(1.5e10), "1.5E10");
    }

    #[test]
    fn test_scientific_small() {
        // JAVA COMPAT: Java switches to scientific notation for |v| < 1e-3
        assert_eq!(java_format_double(1e-4), "1.0E-4");
        assert_eq!(java_format_double(1.5e-4), "1.5E-4");
    }

    #[test]
    fn test_normal_range_boundary() {
        // Values just inside the normal range
        assert_eq!(java_format_double(0.001), "0.001");
        assert_eq!(java_format_double(9999999.0), "9999999.0");
    }

    #[test]
    fn test_typical_fastqc_doubles() {
        // More values commonly seen in FastQC output
        assert_eq!(java_format_double(25.0), "25.0");
        assert_eq!(java_format_double(50.0), "50.0");
        assert_eq!(java_format_double(75.0), "75.0");
        assert_eq!(java_format_double(12.345), "12.345");
        assert_eq!(java_format_double(99.99), "99.99");
    }

    #[test]
    fn test_very_precise_decimal() {
        // Verify that standard precision is maintained
        let val = 33.333333333333336;
        let formatted = java_format_double(val);
        // Should contain the decimal point
        assert!(formatted.contains('.'));
        // Should round-trip parse to the same value
        let parsed: f64 = formatted.parse().unwrap();
        assert_eq!(parsed, val);
    }
}
