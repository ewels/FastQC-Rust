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

/// Write a chart as inline SVG for a module that has one.
///
/// Unlike `write_chart` which converts SVG→PNG→base64, this embeds the SVG
/// directly in the HTML for crisper rendering on modern displays.
/// The SVG is minified to reduce file size.
pub fn write_chart_svg(
    module: &(impl crate::modules::QCModule + ?Sized),
    w: &mut dyn Write,
) -> io::Result<()> {
    if let Some(svg) = module.generate_chart_svg() {
        write!(w, "<p>{}</p>", minify_svg(&svg))?;
    }
    Ok(())
}

/// Minify an SVG string for inline HTML embedding.
///
/// The SVG generator produces verbose output optimised for resvg PNG rendering.
/// This function applies several size reductions for inline HTML display:
/// - Strip XML declaration and DOCTYPE
/// - Move repeated attributes (shape-rendering, font-family) to CSS classes
/// - Shorten fill/stroke style attributes
/// - Merge consecutive same-colour `<line>` segments into `<polyline>` elements
/// - Run-length merge consecutive same-colour `<rect>` elements in heatmaps
fn minify_svg(svg: &str) -> String {
    let mut out = String::with_capacity(svg.len());

    // Collect non-declaration lines and apply text replacements
    for line in svg.lines() {
        let t = line.trim();
        if t.starts_with("<?xml") || t.starts_with("<!DOCTYPE") {
            continue;
        }

        // Inject CSS and max-width into the <svg> tag
        if t.starts_with("<svg ") {
            out.push_str(&t.replacen("<svg ", "<svg style=\"max-width:100%\" ", 1));
            out.push('\n');
            out.push_str(
                "<style>\
                          .ce{shape-rendering:crispEdges}\
                          text{font-family:'Liberation Sans',Arial,Helvetica,sans-serif}\
                          </style>\n",
            );
            continue;
        }

        // Collect <line> elements for merging into polylines later
        if t.starts_with("<line ") {
            out.push_str(t);
            out.push('\n');
            continue;
        }

        // Apply attribute shortening to other elements
        let shortened = t
            .replace(" shape-rendering=\"crispEdges\"", " class=\"ce\"")
            .replace(
                " font-family=\"'Liberation Sans', Arial, Helvetica, sans-serif\"",
                "",
            );
        // Filled rects: style="fill:rgb(R,G,B);stroke:none" → fill="rgb(R,G,B)"
        let shortened = shortened
            .replace("style=\"fill:rgb(", "fill=\"rgb(")
            .replace(");stroke:none\"", ")\"");
        // Stroked rects: style="fill:none;stroke-width:1;stroke:rgb(R,G,B)" → attributes
        let shortened = shortened.replace(
            "style=\"fill:none;stroke-width:1;stroke:",
            "fill=\"none\" stroke=\"",
        );
        out.push_str(&shortened);
        out.push('\n');
    }

    // Post-process: merge lines into polylines and RLE-merge rects
    merge_lines_to_polylines(&mut out);
    rle_merge_rects(&mut out);
    out
}

/// Merge consecutive `<line>` elements with the same stroke colour into `<polyline>` elements.
///
/// Data series in line graphs are drawn as many individual `<line>` segments.
/// A polyline with N points is much more compact than N separate line elements.
/// Gridlines (grey or black) are left as-is since they aren't contiguous series.
fn merge_lines_to_polylines(svg: &mut String) {
    struct LineSeg {
        x1: String,
        y1: String,
        x2: String,
        y2: String,
    }

    // (start_byte, end_byte, stroke, width, segments)
    let mut groups: Vec<(usize, usize, String, String, Vec<LineSeg>)> = Vec::new();
    let mut current_group: Vec<LineSeg> = Vec::new();
    let mut group_start = 0;
    let mut group_end = 0;
    let mut last_stroke = "";
    let mut last_width = "";

    let mut search_from = 0;
    while let Some(start) = svg[search_from..].find("<line ") {
        let abs_start = search_from + start;
        let Some(end) = svg[abs_start..].find("/>") else {
            break;
        };
        let abs_end = abs_start + end + 2;
        let tag = &svg[abs_start..abs_end];

        let stroke = extract_attr(tag, "stroke");
        let width = extract_attr(tag, "stroke-width");
        let is_grid = stroke == "rgb(180,180,180)" || stroke == "rgb(0,0,0)";

        if !is_grid && stroke == last_stroke && width == last_width {
            current_group.push(LineSeg {
                x1: extract_attr(tag, "x1").to_string(),
                y1: extract_attr(tag, "y1").to_string(),
                x2: extract_attr(tag, "x2").to_string(),
                y2: extract_attr(tag, "y2").to_string(),
            });
            group_end = abs_end;
        } else {
            if current_group.len() > 2 {
                groups.push((
                    group_start,
                    group_end,
                    last_stroke.to_string(),
                    last_width.to_string(),
                    std::mem::take(&mut current_group),
                ));
            } else {
                current_group.clear();
            }
            if !is_grid {
                group_start = abs_start;
                group_end = abs_end;
                last_stroke = stroke;
                last_width = width;
                current_group.push(LineSeg {
                    x1: extract_attr(tag, "x1").to_string(),
                    y1: extract_attr(tag, "y1").to_string(),
                    x2: extract_attr(tag, "x2").to_string(),
                    y2: extract_attr(tag, "y2").to_string(),
                });
            } else {
                last_stroke = "";
                last_width = "";
            }
        }

        search_from = skip_trailing_newlines(svg, abs_end);
    }
    if current_group.len() > 2 {
        groups.push((
            group_start,
            group_end,
            last_stroke.to_string(),
            last_width.to_string(),
            current_group,
        ));
    }

    // Replace groups back-to-front to preserve offsets
    for (start, end, stroke, width, segs) in groups.into_iter().rev() {
        let mut points = format!("{},{}", segs[0].x1, segs[0].y1);
        for seg in &segs {
            points.push_str(&format!(" {},{}", seg.x2, seg.y2));
        }
        let polyline = format!(
            "<polyline points=\"{}\" stroke=\"{}\" stroke-width=\"{}\" fill=\"none\"/>",
            points, stroke, width
        );
        svg.replace_range(start..skip_trailing_newlines(svg, end), &polyline);
    }
}

/// Run-length merge consecutive same-colour `<rect>` elements.
///
/// Heatmaps (like per-tile quality) draw thousands of small rects where many
/// adjacent cells have the same colour. Merging runs of identical-colour rects
/// on the same row into a single wider rect dramatically reduces element count.
fn rle_merge_rects(svg: &mut String) {
    struct RectInfo {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        fill: String,
        has_ce_class: bool,
        start: usize,
        end: usize,
    }

    let mut rects: Vec<RectInfo> = Vec::new();
    let mut search_from = 0;
    while let Some(start) = svg[search_from..].find("<rect ") {
        let abs_start = search_from + start;
        let Some(end) = svg[abs_start..].find("/>") else {
            break;
        };
        let abs_end = abs_start + end + 2;
        let tag = &svg[abs_start..abs_end];

        let fill = extract_attr(tag, "fill");
        if fill.starts_with("rgb(") && !tag.contains("stroke") {
            let w: i32 = extract_attr(tag, "width").parse().unwrap_or(0);
            let h: i32 = extract_attr(tag, "height").parse().unwrap_or(0);
            let x: i32 = extract_attr(tag, "x").parse().unwrap_or(0);
            let y: i32 = extract_attr(tag, "y").parse().unwrap_or(0);

            // Skip large background rects
            if w <= 100 && h <= 100 {
                rects.push(RectInfo {
                    x,
                    y,
                    width: w,
                    height: h,
                    fill: fill.to_string(),
                    has_ce_class: tag.contains("class=\"ce\""),
                    start: abs_start,
                    end: abs_end,
                });
            }
        }

        search_from = abs_end;
    }

    let mut replacements: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0;
    while i < rects.len() {
        let mut run_end = i + 1;
        while run_end < rects.len()
            && rects[run_end].y == rects[i].y
            && rects[run_end].height == rects[i].height
            && rects[run_end].fill == rects[i].fill
            && rects[run_end].has_ce_class == rects[i].has_ce_class
            && rects[run_end].x == rects[i].x + rects[i].width * (run_end - i) as i32
        {
            run_end += 1;
        }

        if run_end > i + 1 {
            let merged_width = rects[i].width * (run_end - i) as i32;
            let class_attr = if rects[i].has_ce_class {
                " class=\"ce\""
            } else {
                ""
            };
            let merged = format!(
                "<rect width=\"{}\" height=\"{}\" x=\"{}\" y=\"{}\" fill=\"{}\"{}/>",
                merged_width, rects[i].height, rects[i].x, rects[i].y, rects[i].fill, class_attr
            );
            replacements.push((rects[i].start, rects[run_end - 1].end, merged));
        }

        i = run_end;
    }

    for (start, end, replacement) in replacements.into_iter().rev() {
        svg.replace_range(start..skip_trailing_newlines(svg, end), &replacement);
    }
}

/// Skip past trailing newline characters from a position in the string.
fn skip_trailing_newlines(s: &str, pos: usize) -> usize {
    let mut end = pos;
    while end < s.len() && s.as_bytes().get(end).is_some_and(|&b| b == b'\n') {
        end += 1;
    }
    end
}

/// Extract an XML attribute value from a tag string, returning a borrowed slice.
fn extract_attr<'a>(tag: &'a str, attr: &str) -> &'a str {
    let pattern = format!("{}=\"", attr);
    if let Some(start) = tag.find(&pattern) {
        let val_start = start + pattern.len();
        if let Some(end) = tag[val_start..].find('"') {
            return &tag[val_start..val_start + end];
        }
    }
    ""
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
