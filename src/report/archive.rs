// ZIP archive creation for reports
// Corresponds to the zip-writing portion of Report/HTMLReportArchive.java
//
// The Java code creates a zip file with this structure:
//   {basename}_fastqc/
//   {basename}_fastqc/Icons/
//   {basename}_fastqc/Images/
//   {basename}_fastqc/Icons/fastqc_icon.png
//   {basename}_fastqc/Icons/warning.png
//   {basename}_fastqc/Icons/error.png
//   {basename}_fastqc/Icons/tick.png
//   {basename}_fastqc/summary.txt
//   {basename}_fastqc/Images/{module}.svg  (SVG chart for each module with a chart)
//   {basename}_fastqc/Images/{module}.png  (PNG chart for each module with a chart)
//   {basename}_fastqc/fastqc_report.html
//   {basename}_fastqc/fastqc_data.txt
//   {basename}_fastqc/fastqc.fo     (XSL-FO document for PDF rendering)

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use zip::write::SimpleFileOptions;
use zip::ZipWriter;

use crate::modules::QCModule;
use crate::report::charts::{svg_to_png, xml_escape, CHART_HEIGHT, CHART_WIDTH};
use crate::report::text;

// Embed icon files at compile time, same PNGs as in Templates/Icons/
const ICON_FASTQC: &[u8] = include_bytes!("../../assets/icons/fastqc_icon.png");
const ICON_WARNING: &[u8] = include_bytes!("../../assets/icons/warning.png");
const ICON_ERROR: &[u8] = include_bytes!("../../assets/icons/error.png");
const ICON_TICK: &[u8] = include_bytes!("../../assets/icons/tick.png");

/// Create the FastQC zip archive at the given path.
///
/// The Java HTMLReportArchive constructor writes entries in this order:
///   1. Directory entries for folder/, Icons/, Images/
///   2. Icon PNG files (fastqc_icon.png, warning.png, error.png, tick.png)
///   3. summary.txt (written during startDocument, before modules)
///   4. Module content is written to HTML and data buffers
///   5. fastqc_report.html
///   6. fastqc_data.txt
///   7. fastqc.fo (XSL-FO transform)
///
/// We preserve this entry order for compatibility.
///
/// `html_content` is the pre-generated HTML report string, passed in to avoid
/// regenerating the report (and its expensive SVG->PNG chart conversions) a second time.
pub fn create_zip_archive(
    modules: &[Box<dyn QCModule>],
    filename: &str,
    base_name: &str,
    zip_path: &Path,
    html_content: &str,
    svg_output: bool,
) -> io::Result<()> {
    let file = fs::File::create(zip_path)?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let folder = format!("{}_fastqc", base_name);

    // Directory entries are added first, in this exact order
    zip.add_directory(format!("{}/", folder), options)
        .map_err(zip_err)?;
    zip.add_directory(format!("{}/Icons/", folder), options)
        .map_err(zip_err)?;
    zip.add_directory(format!("{}/Images/", folder), options)
        .map_err(zip_err)?;

    // Icon files are written in this exact order
    let icons: &[(&str, &[u8])] = &[
        ("fastqc_icon.png", ICON_FASTQC),
        ("warning.png", ICON_WARNING),
        ("error.png", ICON_ERROR),
        ("tick.png", ICON_TICK),
    ];
    for (name, data) in icons {
        zip.start_file(format!("{}/Icons/{}", folder, name), options)
            .map_err(zip_err)?;
        zip.write_all(data)?;
    }

    // summary.txt
    // summary.txt is written during startDocument(), before module content
    zip.start_file(format!("{}/summary.txt", folder), options)
        .map_err(zip_err)?;
    text::write_summary(modules, filename, &mut zip)?;

    // writeDefaultImage() writes SVG first, then PNG for each module
    // with a chart, into the Images/ directory inside the zip.
    for module in modules.iter() {
        if module.ignore_in_report() {
            continue;
        }
        if let (Some(image_name), Some(svg)) =
            (module.chart_image_name(), module.generate_chart_svg())
        {
            // SVG files are only written when --svg flag is passed.
            // PNGs are always written.
            if svg_output {
                zip.start_file(format!("{}/Images/{}.svg", folder, image_name), options)
                    .map_err(zip_err)?;
                zip.write_all(svg.as_bytes())?;
            }

            // Write PNG file
            // Java renders the Swing JPanel at 800x600 to a BufferedImage,
            // then writes via ImageIO.write(b, "PNG", zip).
            let png_bytes = svg_to_png(&svg, CHART_WIDTH as u32, CHART_HEIGHT as u32)
                .map_err(io::Error::other)?;
            zip.start_file(format!("{}/Images/{}.png", folder, image_name), options)
                .map_err(zip_err)?;
            zip.write_all(&png_bytes)?;
        }
    }

    // fastqc_report.html
    zip.start_file(format!("{}/fastqc_report.html", folder), options)
        .map_err(zip_err)?;
    zip.write_all(html_content.as_bytes())?;

    // fastqc_data.txt
    zip.start_file(format!("{}/fastqc_data.txt", folder), options)
        .map_err(zip_err)?;
    text::write_fastqc_data(modules, &mut zip)?;

    // fastqc.fo is an XSL-FO transform of the HTML report, generated
    // by applying Templates/fastqc2fo.xsl to the XHTML. The FO contains module titles,
    // image references (for chart modules), and data tables (for table-only modules).
    let fo_content = generate_fo(modules);
    zip.start_file(format!("{}/fastqc.fo", folder), options)
        .map_err(zip_err)?;
    zip.write_all(fo_content.as_bytes())?;

    zip.finish().map_err(zip_err)?;

    Ok(())
}

/// Extract a zip archive to its parent directory.
///
/// Matches HTMLReportArchive.unzipZipFile() which extracts
/// entries relative to the zip file's parent directory.
pub fn extract_zip(zip_path: &Path) -> io::Result<()> {
    let parent = zip_path.parent().unwrap_or_else(|| Path::new("."));

    let file = fs::File::open(zip_path)?;
    let mut archive = zip::ZipArchive::new(file).map_err(zip_err)?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(zip_err)?;
        let out_path = parent.join(entry.name());

        if entry.is_dir() {
            // Create directory if it doesn't exist, skip if it does
            if !out_path.exists() {
                fs::create_dir_all(&out_path)?;
            }
        } else {
            // Ensure parent directory exists
            if let Some(p) = out_path.parent() {
                if !p.exists() {
                    fs::create_dir_all(p)?;
                }
            }
            let mut out_file = fs::File::create(&out_path)?;
            io::copy(&mut entry, &mut out_file)?;
        }
    }

    Ok(())
}

/// Generate the XSL-FO document for PDF rendering.
///
/// The Java version applies Templates/fastqc2fo.xsl to the XHTML report
/// to produce this FO output. The XSL transform generates:
/// - A title page with "FASTQC-Report" in 48pt
/// - For each module: a page break, 48pt title, and either an image reference
///   (for chart modules) or a data table (for table-only modules like BasicStats
///   and OverRepresentedSeqs).
///
/// We generate the FO directly instead of performing an XSL transform, but match
/// the Java output structure exactly.
fn generate_fo(modules: &[Box<dyn QCModule>]) -> String {
    let mut fo = String::new();

    // XSL-FO header matches the output of fastqc2fo.xsl transform
    fo.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    fo.push_str("<fo:root xmlns:fo=\"http://www.w3.org/1999/XSL/Format\" xmlns:fox=\"http://xml.apache.org/fop/extensions\">");
    fo.push_str("<fo:layout-master-set>");
    fo.push_str("<fo:simple-page-master master-name=\"page-layout\">");
    fo.push_str("<fo:region-body margin=\"2.5cm\" region-name=\"body\"/>");
    fo.push_str("</fo:simple-page-master>");
    fo.push_str("</fo:layout-master-set>");
    fo.push_str("<fo:page-sequence master-reference=\"page-layout\">");
    fo.push_str("<fo:flow flow-name=\"body\">");

    // Title block with specific whitespace and inline element
    fo.push_str("<fo:block font-size=\"48pt\" text-align=\"center\">\n");
    fo.push_str("              FASTQC-Report\n");
    fo.push_str("            <fo:inline wrap-option=\"no-wrap\"/>");
    fo.push_str("</fo:block>");

    for module in modules.iter() {
        if module.ignore_in_report() {
            continue;
        }

        // Each module starts with a page break and 48pt title
        fo.push_str(&format!(
            "<fo:block font-size=\"48pt\" page-break-before=\"always\" text-align=\"center\">{}</fo:block>",
            xml_escape(module.name())
        ));

        if module.chart_image_name().is_some() {
            // Chart modules only get a title page in the FO output.
            // Java's XSL transform does NOT include image references or data tables
            // for chart modules -- just the 48pt title block above.
        } else {
            // Table-only modules (BasicStats, OverRepresentedSeqs) get
            // their data rendered as fo:table elements.
            let mut text_buf = Vec::new();
            if module.write_text_report(&mut text_buf).is_ok() {
                if let Ok(text) = String::from_utf8(text_buf) {
                    write_fo_table(&mut fo, &text);
                }
            }
        }
    }

    fo.push_str("</fo:flow>");
    fo.push_str("</fo:page-sequence>");
    fo.push_str("</fo:root>");

    fo
}

/// Write a tab-delimited text report as an XSL-FO table.
///
/// Matches the fo:table structure from fastqc2fo.xsl, with
/// 200pt column widths for each column in the data.
fn write_fo_table(fo: &mut String, text: &str) {
    let mut lines = text.lines();

    if let Some(header_line) = lines.next() {
        let header = header_line.trim_start_matches('#');
        let cols: Vec<&str> = header.split('\t').collect();
        let col_count = cols.len();

        fo.push_str("<fo:table>");
        for _ in 0..col_count {
            // Each column gets 200pt width in the XSL transform
            fo.push_str("<fo:table-column column-width=\"200pt\"/>");
        }
        fo.push_str("<fo:table-body>");

        // Header row (included as regular row in FO, matching Java output)
        // The XSL transform does NOT emit a separate header row;
        // it starts directly with data rows. The header line (starting with #)
        // is stripped by the text report, so we skip it here too.

        // Data rows
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let cells: Vec<&str> = line.split('\t').collect();
            fo.push_str("<fo:table-row>");
            for cell in &cells {
                fo.push_str("<fo:table-cell><fo:block>");
                fo.push_str(&xml_escape(cell));
                fo.push_str("</fo:block></fo:table-cell>");
            }
            fo.push_str("</fo:table-row>");
        }

        fo.push_str("</fo:table-body>");
        fo.push_str("</fo:table>");
    }
}

/// Convert a zip library error into an io::Error.
fn zip_err(e: impl std::fmt::Display) -> io::Error {
    io::Error::other(e.to_string())
}
