pub mod adapter_content;
pub mod basic_stats;
pub mod duplication_level;
pub mod gc_content;
pub mod kmer_content;
pub mod n_content;
pub mod overrepresented_seqs;
pub mod per_base_quality;
pub mod per_base_sequence_content;
pub mod per_sequence_quality;
pub mod per_tile_quality;
pub mod sequence_length_distribution;

use std::fmt;
use std::io;
use std::sync::{Arc, Mutex};

use crate::config::{FastQCConfig, Limits, LimitsExt};
use crate::sequence::Sequence;

/// Status of a QC module after analysis, matching Java FastQC's pass/warn/fail icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleStatus {
    Pass,
    Warn,
    Fail,
}

impl fmt::Display for ModuleStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // These exact strings appear in the text report summary section
        match self {
            ModuleStatus::Pass => write!(f, "PASS"),
            ModuleStatus::Warn => write!(f, "WARN"),
            ModuleStatus::Fail => write!(f, "FAIL"),
        }
    }
}

/// Trait that all QC analysis modules must implement.
///
/// Mirrors the `QCModule` Java interface method-for-method
/// (minus Swing GUI methods like `getResultsPanel()`).
pub trait QCModule: Send {
    /// Process a single sequence record, accumulating statistics.
    fn process_sequence(&mut self, sequence: &Sequence);

    /// The display name of this module as shown in the report.
    fn name(&self) -> &str;

    /// A longer description of what this module checks.
    fn description(&self) -> &str;

    /// Reset the module to its initial state.
    fn reset(&mut self);

    /// Set the source filename for this module.
    /// BasicStats uses this to display the filename in the report.
    /// Other modules ignore it.
    fn set_filename(&mut self, _name: &str) {}

    /// Finalize calculations after all sequences have been processed.
    ///
    /// In Java, modules lazily compute results in synchronized
    /// methods called from getResultsPanel()/makeReport()/raisesError()/raisesWarning().
    /// In Rust, we separate the mutable computation phase (finalize) from the
    /// immutable reporting phase so that raises_error/raises_warning/write_text_report
    /// can take &self.
    fn finalize(&mut self) {}

    /// Whether the module results should trigger a failure status.
    fn raises_error(&self) -> bool;

    /// Whether the module results should trigger a warning status.
    fn raises_warning(&self) -> bool;

    /// Whether this module should skip sequences flagged as filtered.
    fn ignore_filtered_sequences(&self) -> bool;

    /// Whether this module should be excluded from the report.
    /// Matches `ignoreInReport()` - used e.g. by PerTileQuality when
    /// there are no tile IDs in the data.
    fn ignore_in_report(&self) -> bool;

    /// The current status of this module.
    fn status(&self) -> ModuleStatus {
        if self.raises_error() {
            ModuleStatus::Fail
        } else if self.raises_warning() {
            ModuleStatus::Warn
        } else {
            ModuleStatus::Pass
        }
    }

    /// Return the base filename for this module's chart image (without extension).
    /// Matches the filenames used in Images/ directory of the zip archive,
    /// e.g. "per_base_quality" for per_base_quality.png and per_base_quality.svg.
    fn chart_image_name(&self) -> Option<&str> {
        None
    }

    /// Return the alt text for this module's chart image, if it has one.
    /// Modules that produce charts should override this to return Some("...").
    /// The default `write_html_report` uses this to call `write_chart_and_table`
    /// automatically, so modules with charts only need to implement this method
    /// instead of overriding `write_html_report`.
    fn chart_alt_text(&self) -> Option<&str> {
        None
    }

    /// Generate the SVG chart content for this module, if applicable.
    /// In Java, writeDefaultImage() renders the Swing JPanel to both SVG
    /// and PNG. Here we generate SVG first, then convert to PNG via resvg.
    fn generate_chart_svg(&self) -> Option<String> {
        None
    }

    /// Write this module's section to the text report.
    fn write_text_report(&self, writer: &mut dyn io::Write) -> io::Result<()>;

    /// Write this module's HTML content (table and/or chart image) to the report.
    ///
    /// In Java, each module's makeReport() calls writeTable() which
    /// calls writeXhtmlTable() then writeTextTable(). Some modules also call
    /// writeDefaultImage() to embed a chart. The default implementation here
    /// checks for `chart_alt_text()` -- if present, it renders the chart and table
    /// via `write_chart_and_table`; otherwise it renders just an HTML table from
    /// the text report output, matching writeXhtmlTable().
    fn write_html_report(&self, writer: &mut dyn io::Write) -> io::Result<()> {
        // Modules with charts show only the chart in HTML, not the data table.
        // The data table only goes into fastqc_data.txt.
        if let Some(alt_text) = self.chart_alt_text() {
            return crate::report::html::write_chart(self, alt_text, writer);
        }
        // Default: render the text report data as an HTML table
        let mut text_buf = Vec::new();
        self.write_text_report(&mut text_buf)?;
        let text = String::from_utf8(text_buf)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        crate::report::html::write_default_html_table(&text, writer)
    }
}

/// Create the standard set of QC modules based on configuration.
///
/// Module order matches `ModuleFactory.java` instantiation order,
/// which determines the order they appear in the report.
/// The order is: BasicStats, PerBaseQuality, PerTileQuality, PerSequenceQuality,
/// PerBaseContent, PerSequenceGC, NContent, SequenceLength, DuplicationLevel,
/// OverRepresentedSeqs, AdapterContent, KmerContent.
///
/// Note: DuplicationLevel is listed BEFORE OverRepresentedSeqs in the report,
/// but shares data with it. In Java, OverRepresentedSeqs creates DuplicationLevel
/// and adds it to the module list before itself.
pub fn create_modules(config: &FastQCConfig, limits: &Limits) -> Vec<Box<dyn QCModule>> {
    let mut modules: Vec<Box<dyn QCModule>> = Vec::new();

    let ng = config.nogroup;
    let eg = config.expgroup;

    // Load adapter and contaminant lists
    let adapters = config.load_adapters().unwrap_or_default();
    let contaminants = config.load_contaminants().unwrap_or_default();

    // 1. BasicStats
    modules.push(Box::new(basic_stats::BasicStats::new(limits)));

    // 2. PerBaseQualityScores
    if limits.is_module_enabled("quality_base") {
        modules.push(Box::new(per_base_quality::PerBaseQualityScores::new(
            limits, ng, eg,
        )));
    }

    // 3. PerTileQualityScores
    if limits.is_module_enabled("tile") {
        modules.push(Box::new(per_tile_quality::PerTileQualityScores::new(
            limits, ng, eg,
        )));
    }

    // 4. PerSequenceQualityScores
    if limits.is_module_enabled("quality_sequence") {
        modules.push(Box::new(
            per_sequence_quality::PerSequenceQualityScores::new(limits),
        ));
    }

    // 5. PerBaseSequenceContent
    if limits.is_module_enabled("sequence") {
        modules.push(Box::new(
            per_base_sequence_content::PerBaseSequenceContent::new(limits, ng, eg),
        ));
    }

    // 6. PerSequenceGCContent
    if limits.is_module_enabled("gc_sequence") {
        modules.push(Box::new(gc_content::PerSequenceGCContent::new(limits)));
    }

    // 7. NContent
    if limits.is_module_enabled("n_content") {
        modules.push(Box::new(n_content::NContent::new(limits, ng, eg)));
    }

    // 8. SequenceLengthDistribution
    if limits.is_module_enabled("sequence_length") {
        modules.push(Box::new(
            sequence_length_distribution::SequenceLengthDistribution::new(limits, ng),
        ));
    }

    // 9 & 10. DuplicationLevel and OverRepresentedSeqs (shared data)
    // OverRepresentedSeqs creates DuplicationLevel in its constructor.
    // DuplicationLevel appears BEFORE OverRepresentedSeqs in the module list.
    let shared_data = Arc::new(Mutex::new(overrepresented_seqs::OverRepresentedData::new()));

    if limits.is_module_enabled("duplication") {
        modules.push(Box::new(duplication_level::DuplicationLevel::new(
            shared_data.clone(),
            limits,
        )));
    }

    if limits.is_module_enabled("overrepresented") {
        modules.push(Box::new(overrepresented_seqs::OverRepresentedSeqs::new(
            limits,
            config.dup_length,
            &contaminants,
            shared_data.clone(),
        )));
    }

    // 11. AdapterContent
    if limits.is_module_enabled("adapter") {
        modules.push(Box::new(adapter_content::AdapterContent::new(
            limits, &adapters, ng, eg,
        )));
    }

    // 12. KmerContent
    // Kmer is ignored by default (limits.txt: kmer ignore 1)
    if limits.is_module_enabled("kmer") {
        modules.push(Box::new(kmer_content::KmerContent::new(
            limits,
            config.kmer_size,
            ng,
            eg,
        )));
    }

    modules
}
