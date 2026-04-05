use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;

// Embedded default config files match the Java resource files exactly.
// These are the same files shipped in the Java FastQC Configuration/ directory.
const DEFAULT_LIMITS: &str = include_str!("../assets/limits.txt");
const DEFAULT_ADAPTERS: &str = include_str!("../assets/adapter_list.txt");
const DEFAULT_CONTAMINANTS: &str = include_str!("../assets/contaminant_list.txt");

/// Configuration for a FastQC run, mirroring all fields from Java FastQCConfig.
#[derive(Debug, Clone)]
pub struct FastQCConfig {
    pub nogroup: bool,
    pub expgroup: bool,
    pub quiet: bool,
    pub kmer_size: u8,
    pub threads: usize,
    pub output_dir: Option<PathBuf>,
    pub casava: bool,
    pub nano: bool,
    pub nofilter: bool,
    pub do_unzip: Option<bool>,
    pub delete_after_unzip: bool,
    pub sequence_format: Option<String>,
    pub contaminant_file: Option<PathBuf>,
    pub adapter_file: Option<PathBuf>,
    pub limits_file: Option<PathBuf>,
    pub min_length: usize,
    pub dup_length: usize,
    pub svg_output: bool,
    pub temp_dir: Option<PathBuf>,
}

impl Default for FastQCConfig {
    fn default() -> Self {
        Self {
            nogroup: false,
            expgroup: false,
            quiet: false,
            kmer_size: 7,
            threads: 1,
            output_dir: None,
            casava: false,
            nano: false,
            nofilter: false,
            do_unzip: None,
            delete_after_unzip: false,
            sequence_format: None,
            contaminant_file: None,
            adapter_file: None,
            limits_file: None,
            min_length: 0,
            dup_length: 0,
            svg_output: false,
            temp_dir: None,
        }
    }
}

/// A parsed limit entry from limits.txt.
/// The key is "{module}\t{level}" (e.g. "duplication\twarn"), value is the threshold.
///
/// The Java ModuleConfig stores limits as a nested HashMap keyed on
/// module name and then level (warn/error/ignore). We flatten into a single HashMap
/// with a composite key of "module\tlevel" to simplify lookups while keeping the
/// same data accessible.
pub type Limits = HashMap<String, f64>;

/// Extension methods for the `Limits` type to reduce boilerplate in modules.
pub trait LimitsExt {
    /// Get a threshold value for a module/level key, returning a default if not set.
    ///
    /// Replaces the common pattern:
    ///   `self.limits.get("module\tlevel").copied().unwrap_or(default)`
    fn threshold(&self, key: &str, default: f64) -> f64;

    /// Check whether a module is configured to be ignored (ignore value > 0).
    ///
    /// Replaces the common pattern:
    ///   `self.limits.get("module\tignore").copied().unwrap_or(0.0) > 0.0`
    fn is_ignored(&self, module: &str) -> bool;

    /// Check whether a module should be created (not configured to be ignored).
    ///
    /// Replaces the common pattern in create_modules():
    ///   `limits.get("module\tignore").map_or(true, |&v| v == 0.0)`
    fn is_module_enabled(&self, module: &str) -> bool;
}

impl LimitsExt for Limits {
    fn threshold(&self, key: &str, default: f64) -> f64 {
        self.get(key).copied().unwrap_or(default)
    }

    fn is_ignored(&self, module: &str) -> bool {
        let key = format!("{}\tignore", module);
        self.get(&key).copied().unwrap_or(0.0) > 0.0
    }

    fn is_module_enabled(&self, module: &str) -> bool {
        let key = format!("{}\tignore", module);
        self.get(&key).is_none_or(|&v| v == 0.0)
    }
}

impl FastQCConfig {
    /// Load module limits from the configured file or the embedded default.
    ///
    /// Parsing matches `ModuleConfig.java` - lines starting with '#'
    /// are comments, blank lines are skipped, and each data line has whitespace-
    /// separated fields: module level value.
    pub fn load_limits(&self) -> io::Result<Limits> {
        let text = match &self.limits_file {
            Some(path) => fs::read_to_string(path)?,
            None => DEFAULT_LIMITS.to_string(),
        };
        Ok(parse_limits(&text))
    }

    /// Load adapter sequences from the configured file or the embedded default.
    ///
    /// Returns a list of (name, sequence) pairs.
    ///
    /// Parsing matches the Java adapter loading - lines starting with
    /// '#' are comments, blank lines are skipped, and each data line is tab-
    /// delimited with name and sequence columns.
    pub fn load_adapters(&self) -> io::Result<Vec<(String, String)>> {
        let text = match &self.adapter_file {
            Some(path) => fs::read_to_string(path)?,
            None => DEFAULT_ADAPTERS.to_string(),
        };
        Ok(parse_name_sequence_file(&text))
    }

    /// Load contaminant sequences from the configured file or the embedded default.
    ///
    /// Returns a list of (name, sequence) pairs.
    ///
    /// Parsing matches the Java contaminant loading - same format as
    /// adapter files.
    pub fn load_contaminants(&self) -> io::Result<Vec<(String, String)>> {
        let text = match &self.contaminant_file {
            Some(path) => fs::read_to_string(path)?,
            None => DEFAULT_CONTAMINANTS.to_string(),
        };
        Ok(parse_name_sequence_file(&text))
    }
}

/// Parse a limits.txt formatted string into a Limits map.
///
/// Whitespace-separated fields. The Java code splits on arbitrary
/// whitespace (tabs/spaces), so we do the same with split_whitespace().
fn parse_limits(text: &str) -> Limits {
    let mut limits = HashMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        if parts.len() >= 3 {
            let module = parts[0];
            let level = parts[1]; // "warn", "error", or "ignore"
            if let Ok(value) = parts[2].parse::<f64>() {
                let key = format!("{}\t{}", module, level);
                limits.insert(key, value);
            }
        }
    }
    limits
}

/// Parse a tab-delimited name/sequence file (adapters or contaminants).
///
/// Lines starting with '#' are comments, blank lines are skipped.
/// Each data line has a name and sequence separated by one or more tabs.
/// Leading/trailing whitespace on the sequence is trimmed.
fn parse_name_sequence_file(text: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        // The Java code splits on tab and takes the first two fields.
        // Names may contain spaces so we split on tab only.
        if let Some(tab_pos) = trimmed.find('\t') {
            let name = trimmed[..tab_pos].trim().to_string();
            let seq = trimmed[tab_pos + 1..].trim().to_string();
            if !name.is_empty() && !seq.is_empty() {
                entries.push((name, seq));
            }
        }
    }
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_limits_default() {
        let config = FastQCConfig::default();
        let limits = config.load_limits().unwrap();
        // Check a few well-known entries from the default limits.txt
        assert_eq!(limits.get("duplication\twarn"), Some(&70.0));
        assert_eq!(limits.get("duplication\terror"), Some(&50.0));
        assert_eq!(limits.get("kmer\tignore"), Some(&1.0));
        assert_eq!(limits.get("adapter\twarn"), Some(&5.0));
    }

    #[test]
    fn test_parse_adapters_default() {
        let config = FastQCConfig::default();
        let adapters = config.load_adapters().unwrap();
        assert!(!adapters.is_empty());
        // First adapter in the default file
        assert_eq!(adapters[0].0, "Illumina Universal Adapter");
        assert_eq!(adapters[0].1, "AGATCGGAAGAG");
    }

    #[test]
    fn test_parse_contaminants_default() {
        let config = FastQCConfig::default();
        let contaminants = config.load_contaminants().unwrap();
        assert!(!contaminants.is_empty());
        assert_eq!(contaminants[0].0, "Illumina Single End Adapter 1");
        assert_eq!(contaminants[0].1, "GATCGGAAGAGCTCGTATGCCGTCTTCTGCTTG");
    }

    #[test]
    fn test_parse_limits_comments_and_blanks() {
        let text = "# comment\n\nduplication\twarn\t70\n";
        let limits = parse_limits(text);
        assert_eq!(limits.len(), 1);
        assert_eq!(limits.get("duplication\twarn"), Some(&70.0));
    }
}
