// HTML report generation
// Corresponds to Report/HTMLReportArchive.java
//
// The Java implementation uses XMLStreamWriter which produces XML-style output:
// - Self-closing tags: `<img .../>` (no space before /)
// - Entity escaping: &amp; &lt; &gt; &quot;
// - No pretty-printing / newlines between elements
// We replicate that style here.

use std::io::{self, Write};

use chrono::Local;

use crate::modules::{ModuleStatus, QCModule};
use crate::report::charts::png_to_data_uri;
use crate::VERSION;

// Embed icons at compile time, matching Templates/Icons/ in the Java jar.
const ICON_FASTQC: &[u8] = include_bytes!("../../assets/icons/fastqc_icon.png");
const ICON_TICK: &[u8] = include_bytes!("../../assets/icons/tick.png");
const ICON_WARNING: &[u8] = include_bytes!("../../assets/icons/warning.png");
const ICON_ERROR: &[u8] = include_bytes!("../../assets/icons/error.png");

// CSS is embedded from header_template.html, read at compile time.
const CSS: &str = include_str!("../../assets/header_template.html");

/// Generate the complete HTML report as a String.
///
/// The Java code writes to an XMLStreamWriter backed by a StringWriter,
/// then toString()'s the result. We build the equivalent string directly.
pub fn generate_html_report(
    modules: &[Box<dyn QCModule>],
    filename: &str,
) -> io::Result<String> {
    let mut buf = Vec::new();
    write_html_report(modules, filename, &mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Write the complete HTML report to a writer.
pub fn write_html_report(
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

        write!(w, "<a href=\"#M{}\">" , i)?;
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
        write!(w, "<h2 id=\"M{}\">" , i)?;

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

/// Write a chart image (as PNG) and data table for a module that has charts.
///
/// Java's writeDefaultImage() renders a Swing JPanel to a BufferedImage
/// and embeds it as a base64 PNG in the HTML via ImageToBase64.imageToBase64().
/// We replicate this by generating SVG, converting to PNG via resvg, and embedding
/// the PNG as base64.
/// Write a chart image for a module that has one.
///
/// In Java, modules with charts call writeDefaultImage() which embeds
/// ONLY the chart image in the HTML — no data table. The data table only appears
/// in fastqc_data.txt. Modules without charts (BasicStats, OverRepresentedSeqs)
/// call writeTable() which renders an HTML table instead.
pub fn write_chart(module: &(impl crate::modules::QCModule + ?Sized), alt_text: &str, w: &mut dyn Write) -> io::Result<()> {
    use crate::report::charts::{CHART_WIDTH, CHART_HEIGHT, svg_to_png};

    if let Some(svg) = module.generate_chart_svg() {
        let png_bytes = svg_to_png(&svg, CHART_WIDTH as u32, CHART_HEIGHT as u32)
            .map_err(io::Error::other)?;
        let data_uri = png_to_data_uri(&png_bytes);
        write!(
            w,
            "<p><img class=\"indented\" src=\"{}\" alt=\"{}\"/></p>",
            data_uri,
            alt_text,
        )?;
    }

    Ok(())
}

/// Write an HTML table from tab-delimited text report data.
///
/// This is the default HTML output for modules that use `write_text_report` to
/// produce a tab-delimited table. It parses the text report output and converts
/// it to an HTML table matching Java's `writeXhtmlTable()` output.
///
/// The Java AbstractQCModule.writeXhtmlTable() writes
/// `<table><thead><tr><th>...</th></tr></thead><tbody><tr><td>...</td></tr>...</tbody></table>`
pub fn write_default_html_table(text_report: &str, w: &mut dyn Write) -> io::Result<()> {
    let mut lines = text_report.lines();

    write!(w, "<table>")?;

    // First line is the header (starts with #)
    if let Some(header_line) = lines.next() {
        // Header row starts with '#' in text report
        let header = header_line.trim_start_matches('#');
        let cols: Vec<&str> = header.split('\t').collect();

        write!(w, "<thead>")?;
        write!(w, "<tr>")?;
        for col in &cols {
            write!(w, "<th>")?;
            write_escaped(w, col)?;
            write!(w, "</th>")?;
        }
        write!(w, "</tr>")?;
        write!(w, "</thead>")?;
    }

    write!(w, "<tbody>")?;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let cells: Vec<&str> = line.split('\t').collect();
        write!(w, "<tr>")?;
        for cell in &cells {
            write!(w, "<td>")?;
            write_escaped(w, cell)?;
            write!(w, "</td>")?;
        }
        write!(w, "</tr>")?;
    }
    write!(w, "</tbody>")?;
    write!(w, "</table>")?;

    Ok(())
}

/// Escape special XML/HTML characters, matching Java XMLStreamWriter.writeCharacters().
///
/// XMLStreamWriter escapes &, <, > in character data.
fn write_escaped(w: &mut dyn Write, s: &str) -> io::Result<()> {
    for ch in s.chars() {
        match ch {
            '&' => write!(w, "&amp;")?,
            '<' => write!(w, "&lt;")?,
            '>' => write!(w, "&gt;")?,
            _ => write!(w, "{}", ch)?,
        }
    }
    Ok(())
}

/// Format a date matching Java's `SimpleDateFormat("EEE d MMM yyyy")`.
///
/// Java outputs e.g. "Sun 5 Apr 2026" — abbreviated weekday,
/// unpadded day-of-month, abbreviated month, four-digit year.
fn format_java_date(dt: &chrono::DateTime<Local>) -> String {
    // chrono's %e gives space-padded day, but Java uses unpadded.
    // We use %e and trim the leading space.
    dt.format("%a %e %b %Y").to_string().replace("  ", " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_escape() {
        let mut buf = Vec::new();
        write_escaped(&mut buf, "A & B < C > D").unwrap();
        assert_eq!(
            String::from_utf8(buf).unwrap(),
            "A &amp; B &lt; C &gt; D"
        );
    }

    #[test]
    fn test_format_java_date() {
        use chrono::TimeZone;
        let dt = chrono::FixedOffset::east_opt(0)
            .unwrap()
            .with_ymd_and_hms(2026, 4, 5, 12, 0, 0)
            .unwrap()
            .with_timezone(&Local);
        let formatted = format_java_date(&dt);
        // Day should not be zero-padded
        assert!(formatted.contains(" 5 "), "Got: {}", formatted);
    }

    #[test]
    fn test_default_html_table() {
        let text = "#Measure\tValue\nFilename\ttest.fastq\nTotal\t100\n";
        let mut buf = Vec::new();
        write_default_html_table(text, &mut buf).unwrap();
        let html = String::from_utf8(buf).unwrap();
        assert!(html.starts_with("<table>"));
        assert!(html.contains("<th>Measure</th>"));
        assert!(html.contains("<td>test.fastq</td>"));
        assert!(html.ends_with("</table>"));
    }
}
