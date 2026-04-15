// Classic HTML report template — produces byte-identical output to Java FastQC.
//
// This is a mechanical extraction of the original html::write_html_report() body.
// No logic changes — the output must remain identical to the pre-refactor version.

use std::io::{self, Write};

use chrono::Local;

use crate::modules::{ModuleStatus, QCModule};
use crate::report::charts::png_to_data_uri;
use crate::report::html::{format_java_date, write_escaped};
use crate::report::templates::ReportTemplate;
use crate::VERSION;

// Embed icons at compile time, matching Templates/Icons/ in the Java jar.
const ICON_FASTQC: &[u8] = include_bytes!("../../../assets/icons/fastqc_icon.png");
const ICON_TICK: &[u8] = include_bytes!("../../../assets/icons/tick.png");
const ICON_WARNING: &[u8] = include_bytes!("../../../assets/icons/warning.png");
const ICON_ERROR: &[u8] = include_bytes!("../../../assets/icons/error.png");

// CSS is embedded from header_template.html, read at compile time.
const CSS: &str = include_str!("../../../assets/header_template.html");

pub struct ClassicTemplate;

impl ReportTemplate for ClassicTemplate {
    fn write_html_report(
        &self,
        modules: &[Box<dyn QCModule>],
        filename: &str,
        w: &mut dyn Write,
    ) -> io::Result<()> {
        // XMLStreamWriter.writeDTD() outputs the DTD literally, then
        // writeStartElement("html") follows immediately with no whitespace.
        write!(w, "<!DOCTYPE html>")?;
        write!(w, "<html>")?;

        // <head>
        write!(w, "<head>")?;
        write!(w, "<title>")?;
        write_escaped(w, filename)?;
        write!(w, " FastQC Report")?;
        write!(w, "</title>")?;

        // Inline CSS
        // The Java code reads header_template.html as raw bytes and writes
        // them via writeCharacters(), which entity-escapes the content. Since the CSS
        // doesn't contain &, <, >, or " characters that need escaping, the result is
        // identical to writing it raw.
        write!(w, "<style type=\"text/css\">")?;
        write_escaped(w, CSS)?;
        write!(w, "</style>")?;

        write!(w, "</head>")?;

        // <body>
        write!(w, "<body>")?;

        // Header
        write!(w, "<div class=\"header\">")?;
        write!(w, "<div id=\"header_title\">")?;
        // writeEmptyElement("img") produces a self-closing <img ... />
        write!(
            w,
            "<img src=\"{}\" alt=\"FastQC\"/>",
            png_to_data_uri(ICON_FASTQC)
        )?;
        write!(w, "FastQC Report")?;
        write!(w, "</div>")?;

        // Date and filename
        write!(w, "<div id=\"header_filename\">")?;
        // SimpleDateFormat("EEE d MMM yyyy") e.g. "Sun 5 Apr 2026"
        let now = Local::now();
        let date_str = format_java_date(&now);
        write_escaped(w, &date_str)?;
        // writeEmptyElement("br") produces <br />
        write!(w, "<br/>")?;
        write_escaped(w, filename)?;
        write!(w, "</div>")?;
        write!(w, "</div>")?;

        // Summary sidebar
        write!(w, "<div class=\"summary\">")?;
        write!(w, "<h2>")?;
        write!(w, "Summary")?;
        write!(w, "</h2>")?;
        write!(w, "<ul>")?;

        for (i, module) in modules.iter().enumerate() {
            if module.ignore_in_report() {
                continue;
            }
            write!(w, "<li>")?;

            // Summary icons use different alt text than module header icons.
            // Summary: [PASS], [WARNING], [FAIL]
            // Module headers: [OK], [WARN], [FAIL]
            let (icon, alt) = match module.status() {
                ModuleStatus::Pass => (ICON_TICK, "[PASS]"),
                ModuleStatus::Warn => (ICON_WARNING, "[WARNING]"),
                ModuleStatus::Fail => (ICON_ERROR, "[FAIL]"),
            };
            write!(
                w,
                "<img src=\"{}\" alt=\"{}\"/>",
                png_to_data_uri(icon),
                alt
            )?;

            write!(w, "<a href=\"#M{}\">", i)?;
            write_escaped(w, module.name())?;
            write!(w, "</a>")?;
            write!(w, "</li>")?;
        }

        write!(w, "</ul>")?;
        write!(w, "</div>")?;

        // Main content
        write!(w, "<div class=\"main\">")?;

        for (i, module) in modules.iter().enumerate() {
            if module.ignore_in_report() {
                continue;
            }

            write!(w, "<div class=\"module\">")?;
            write!(w, "<h2 id=\"M{}\">", i)?;

            // Module header icons use [OK]/[WARN]/[FAIL] alt text
            let (icon, alt) = match module.status() {
                ModuleStatus::Pass => (ICON_TICK, "[OK]"),
                ModuleStatus::Warn => (ICON_WARNING, "[WARN]"),
                ModuleStatus::Fail => (ICON_ERROR, "[FAIL]"),
            };
            write!(
                w,
                "<img src=\"{}\" alt=\"{}\"/>",
                png_to_data_uri(icon),
                alt
            )?;

            write_escaped(w, module.name())?;
            write!(w, "</h2>")?;

            // Module content (table or chart)
            module.write_html_report(w)?;

            write!(w, "</div>")?;
        }

        write!(w, "</div>")?;

        // Footer
        // Two spaces before "(version" matches the Java concatenation:
        // "  (version "+FastQCApplication.VERSION+")"
        write!(w, "<div class=\"footer\">")?;
        write!(w, "Produced by ")?;
        write!(
            w,
            "<a href=\"http://www.bioinformatics.babraham.ac.uk/projects/fastqc/\">"
        )?;
        write!(w, "FastQC")?;
        write!(w, "</a>")?;
        write!(w, "  (version {})", VERSION)?;
        write!(w, "</div>")?;

        write!(w, "</body>")?;
        write!(w, "</html>")?;

        Ok(())
    }
}
