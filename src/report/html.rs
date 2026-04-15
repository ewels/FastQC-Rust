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

use crate::config::TemplateName;
use crate::modules::QCModule;
use crate::report::charts::png_to_data_uri;

/// Generate the complete HTML report as a String.
///
/// Delegates to the selected template for the actual HTML structure.
pub fn generate_html_report(
    modules: &[Box<dyn QCModule>],
    filename: &str,
    template_name: TemplateName,
) -> io::Result<String> {
    let template = crate::report::templates::create_template(template_name);
    let mut buf = Vec::new();
    template.write_html_report(modules, filename, &mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Write a chart image for a module that has one.
///
/// In Java, modules with charts call writeDefaultImage() which embeds
/// ONLY the chart image in the HTML — no data table. The data table only appears
/// in fastqc_data.txt. Modules without charts (BasicStats, OverRepresentedSeqs)
/// call writeTable() which renders an HTML table instead.
pub fn write_chart(
    module: &(impl crate::modules::QCModule + ?Sized),
    alt_text: &str,
    w: &mut dyn Write,
) -> io::Result<()> {
    use crate::report::charts::{svg_to_png, CHART_HEIGHT, CHART_WIDTH};

    if let Some(svg) = module.generate_chart_svg() {
        let png_bytes =
            svg_to_png(&svg, CHART_WIDTH as u32, CHART_HEIGHT as u32).map_err(io::Error::other)?;
        let data_uri = png_to_data_uri(&png_bytes);
        write!(
            w,
            "<p><img class=\"indented\" src=\"{}\" alt=\"{}\"/></p>",
            data_uri, alt_text,
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
pub fn write_escaped(w: &mut dyn Write, s: &str) -> io::Result<()> {
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
pub fn format_java_date(dt: &chrono::DateTime<Local>) -> String {
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
        assert_eq!(String::from_utf8(buf).unwrap(), "A &amp; B &lt; C &gt; D");
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
