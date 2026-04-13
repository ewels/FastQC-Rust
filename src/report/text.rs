// Text report generation (fastqc_data.txt and summary.txt)
// Corresponds to HTMLReportArchive.java text output sections.

use std::io::{self, Write};

use crate::modules::{ModuleStatus, QCModule};
use crate::VERSION;

/// Write the complete fastqc_data.txt content.
///
/// The Java code builds this via a StringBuffer in HTMLReportArchive,
/// appending each module's data section between >>ModuleName and >>END_MODULE markers.
pub fn write_fastqc_data(modules: &[Box<dyn QCModule>], writer: &mut dyn Write) -> io::Result<()> {
    // Version header must match exactly
    writeln!(writer, "##FastQC\t{}", VERSION)?;

    for module in modules {
        if module.ignore_in_report() {
            continue;
        }

        // Status is lowercase in fastqc_data.txt (pass/warn/fail)
        let status_str = match module.status() {
            ModuleStatus::Pass => "pass",
            ModuleStatus::Warn => "warn",
            ModuleStatus::Fail => "fail",
        };

        writeln!(writer, ">>{}\t{}", module.name(), status_str)?;
        module.write_text_report(writer)?;
        writeln!(writer, ">>END_MODULE")?;
    }

    Ok(())
}

/// Write the complete summary.txt content.
///
/// Each line is STATUS\tModuleName\tFilename with platform line separator.
/// On Linux/macOS this is \n.
pub fn write_summary(
    modules: &[Box<dyn QCModule>],
    filename: &str,
    writer: &mut dyn Write,
) -> io::Result<()> {
    for module in modules {
        if module.ignore_in_report() {
            continue;
        }

        // Status is UPPERCASE in summary.txt (PASS/WARN/FAIL)
        writeln!(
            writer,
            "{}\t{}\t{}",
            module.status(),
            module.name(),
            filename
        )?;
    }

    Ok(())
}
