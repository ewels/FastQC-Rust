// Modern HTML report template — responsive layout with SVG icons and help accordions.
//
// Ported from upstream Java FastQC PR #161 (design refresh).
// Uses compile-time-embedded HTML template fragments with `{{PLACEHOLDER}}` replacement.

use std::collections::HashMap;
use std::io::{self, Write};
use std::sync::LazyLock;

use chrono::Local;

use crate::modules::{ModuleStatus, QCModule};
use crate::report::html::format_java_date;
use crate::report::templates::ReportTemplate;
use crate::VERSION;

// Template fragments
const REPORT_TEMPLATE: &str =
    include_str!("../../../assets/templates/modern/report_template.html");
const MODULE_WRAPPER: &str =
    include_str!("../../../assets/templates/modern/module_wrapper.html");
const SIDEBAR_ITEM: &str = include_str!("../../../assets/templates/modern/sidebar_item.html");
const CSS: &str = include_str!("../../../assets/templates/modern/fastqc.css");

// SVG icons
const ICON_FASTQC_SVG: &str =
    include_str!("../../../assets/templates/modern/icons/fastqc_icon.svg");
const ICON_PASS_SVG: &str = include_str!("../../../assets/templates/modern/icons/pass.svg");
const ICON_WARNING_SVG: &str =
    include_str!("../../../assets/templates/modern/icons/warning.svg");
const ICON_ERROR_SVG: &str = include_str!("../../../assets/templates/modern/icons/error.svg");

// Help text for each module (embedded at compile time)
const HELP_BASIC_STATS: &str = include_str!("../../../assets/help/basic-statistics.html");
const HELP_PER_BASE_QUALITY: &str =
    include_str!("../../../assets/help/per-base-sequence-quality.html");
const HELP_PER_TILE_QUALITY: &str =
    include_str!("../../../assets/help/per-tile-sequence-quality.html");
const HELP_PER_SEQ_QUALITY: &str =
    include_str!("../../../assets/help/per-sequence-quality-scores.html");
const HELP_PER_BASE_CONTENT: &str =
    include_str!("../../../assets/help/per-base-sequence-content.html");
const HELP_GC_CONTENT: &str = include_str!("../../../assets/help/per-sequence-gc-content.html");
const HELP_N_CONTENT: &str = include_str!("../../../assets/help/per-base-n-content.html");
const HELP_SEQ_LENGTH: &str =
    include_str!("../../../assets/help/sequence-length-distribution.html");
const HELP_DUPLICATION: &str = include_str!("../../../assets/help/duplicate-sequences.html");
const HELP_OVERREP: &str =
    include_str!("../../../assets/help/overrepresented-sequences.html");
const HELP_ADAPTER: &str = include_str!("../../../assets/help/adapter-content.html");
const HELP_KMER: &str = include_str!("../../../assets/help/kmer-content.html");

pub struct ModernTemplate;

impl ReportTemplate for ModernTemplate {
    fn write_html_report(
        &self,
        modules: &[Box<dyn QCModule>],
        filename: &str,
        w: &mut dyn Write,
    ) -> io::Result<()> {
        let now = Local::now();
        let date_str = format_java_date(&now);

        // Build sidebar items
        let mut summary_items = String::new();
        for (i, module) in modules.iter().enumerate() {
            if module.ignore_in_report() {
                continue;
            }
            let (status_class, status_text) = match module.status() {
                ModuleStatus::Pass => ("sidebar-pass", "Pass"),
                ModuleStatus::Warn => ("sidebar-warning", "Warn"),
                ModuleStatus::Fail => ("sidebar-error", "Error"),
            };
            let item = SIDEBAR_ITEM
                .replace("{{MODULE_INDEX}}", &i.to_string())
                .replace("{{MODULE_NAME}}", module.name())
                .replace("{{STATUS_CLASS}}", status_class)
                .replace("{{STATUS_TEXT}}", status_text);
            summary_items.push_str(&item);
        }

        // Build module content
        let mut module_content = String::new();
        for (i, module) in modules.iter().enumerate() {
            if module.ignore_in_report() {
                continue;
            }

            // Capture module HTML output into a buffer
            let mut module_buf = Vec::new();
            module.write_html_report(&mut module_buf)?;
            let module_html =
                String::from_utf8(module_buf).map_err(|e| io::Error::other(e.to_string()))?;

            let status_icon = match module.status() {
                ModuleStatus::Pass => ICON_PASS_SVG,
                ModuleStatus::Warn => ICON_WARNING_SVG,
                ModuleStatus::Fail => ICON_ERROR_SVG,
            };

            let help_content = get_help_text(module.name());

            let wrapped = MODULE_WRAPPER
                .replace("{{MODULE_INDEX}}", &i.to_string())
                .replace("{{MODULE_NAME}}", module.name())
                .replace("{{STATUS_ICON}}", status_icon)
                .replace("{{HELP_CONTENT}}", help_content)
                .replace("{{MODULE_CONTENT}}", &module_html);
            module_content.push_str(&wrapped);
        }

        // Assemble the full HTML
        let html = REPORT_TEMPLATE
            .replace("{{TITLE}}", &format!("{} FastQC Report", filename))
            .replace("{{CSS_CONTENT}}", CSS)
            .replace("{{DATE}}", &date_str)
            .replace("{{FILENAME}}", filename)
            .replace("{{VERSION}}", VERSION)
            .replace(
                "{{FASTQC_ICON_SVG_MOBILE}}",
                &make_svg_ids_unique(ICON_FASTQC_SVG, "mobile"),
            )
            .replace(
                "{{FASTQC_ICON_SVG_SIDEBAR}}",
                &make_svg_ids_unique(ICON_FASTQC_SVG, "sidebar"),
            )
            .replace("{{SUMMARY_ITEMS}}", &summary_items)
            .replace("{{MODULE_CONTENT}}", &module_content);

        w.write_all(html.as_bytes())?;
        Ok(())
    }
}

/// Make SVG `id` attributes unique by appending a suffix.
///
/// The FastQC logo SVG uses `id` attributes for gradient definitions.
/// When embedded twice in the same page (sidebar + mobile nav), IDs would
/// conflict. This appends a suffix to all `id="..."` and `url(#...)` references.
fn make_svg_ids_unique(svg: &str, suffix: &str) -> String {
    // Replace id="foo" with id="foo_suffix"
    let mut result = String::with_capacity(svg.len() + 256);
    let mut rest = svg;

    // Process id="..." attributes
    while let Some(id_start) = rest.find("id=\"") {
        result.push_str(&rest[..id_start + 4]); // include 'id="'
        rest = &rest[id_start + 4..];
        if let Some(id_end) = rest.find('"') {
            let id_value = &rest[..id_end];
            result.push_str(id_value);
            result.push('_');
            result.push_str(suffix);
            rest = &rest[id_end..];
        }
    }
    result.push_str(rest);

    // Now process url(#...) references in the result
    let input = result;
    let mut result = String::with_capacity(input.len());
    let mut rest = input.as_str();
    while let Some(url_start) = rest.find("url(#") {
        result.push_str(&rest[..url_start + 5]); // include 'url(#'
        rest = &rest[url_start + 5..];
        if let Some(url_end) = rest.find(')') {
            let url_value = &rest[..url_end];
            result.push_str(url_value);
            result.push('_');
            result.push_str(suffix);
            rest = &rest[url_end..];
        }
    }
    result.push_str(rest);

    result
}

/// Pre-stripped help text, computed once on first access.
static STRIPPED_HELP: LazyLock<HashMap<&'static str, String>> = LazyLock::new(|| {
    let entries: &[(&str, &str)] = &[
        ("basic statistics", HELP_BASIC_STATS),
        ("per base sequence quality", HELP_PER_BASE_QUALITY),
        ("per tile sequence quality", HELP_PER_TILE_QUALITY),
        ("per sequence quality scores", HELP_PER_SEQ_QUALITY),
        ("per base sequence content", HELP_PER_BASE_CONTENT),
        ("per sequence gc content", HELP_GC_CONTENT),
        ("per base n content", HELP_N_CONTENT),
        ("sequence length distribution", HELP_SEQ_LENGTH),
        ("sequence duplication levels", HELP_DUPLICATION),
        ("overrepresented sequences", HELP_OVERREP),
        ("adapter content", HELP_ADAPTER),
        ("kmer content", HELP_KMER),
    ];
    entries
        .iter()
        .map(|(name, html)| (*name, strip_help_html(html)))
        .collect()
});

/// Look up help text for a module by its name.
///
/// Returns stripped HTML content suitable for embedding in the help accordion.
/// The module name matching is case-insensitive to handle the mixed capitalisation
/// in module names (some title case, some sentence case).
fn get_help_text(module_name: &str) -> &'static str {
    let lower = module_name.to_ascii_lowercase();
    STRIPPED_HELP
        .get(lower.as_str())
        .map(|s| s.as_str())
        .unwrap_or("<p>Help documentation not available for this module.</p>")
}

/// Strip HTML structure from help files, keeping only the body content.
///
/// Matches the Java `convertHelpHtmlToText()` method:
/// - Remove `<html>`, `<head>`, `<body>`, `<h1>` tags and their content
/// - Convert `<h2>` to `<h4>` for proper hierarchy inside the accordion
/// - Remove `<img>` tags and empty `<p>` elements
/// - Clean up excess whitespace
fn strip_help_html(html: &str) -> String {
    let mut content = html.to_string();

    // Remove <head>...</head> block (including <style>)
    if let Some(head_start) = content.find("<head") {
        if let Some(head_end) = content.find("</head>") {
            content = format!(
                "{}{}",
                &content[..head_start],
                &content[head_end + 7..]
            );
        }
    }

    // Remove <html...> tag
    while let Some(start) = content.find("<html") {
        if let Some(end) = content[start..].find('>') {
            content = format!("{}{}", &content[..start], &content[start + end + 1..]);
        } else {
            break;
        }
    }
    content = content.replace("</html>", "");

    // Remove <body...> and </body>
    while let Some(start) = content.find("<body") {
        if let Some(end) = content[start..].find('>') {
            content = format!("{}{}", &content[..start], &content[start + end + 1..]);
        } else {
            break;
        }
    }
    content = content.replace("</body>", "");

    // Remove <h1>...</h1>
    while let Some(start) = content.find("<h1") {
        if let Some(end) = content.find("</h1>") {
            content = format!("{}{}", &content[..start], &content[end + 5..]);
        } else {
            break;
        }
    }

    // Remove <img...> tags (self-closing or not)
    while let Some(start) = content.find("<img") {
        if let Some(end) = content[start..].find('>') {
            content = format!("{}{}", &content[..start], &content[start + end + 1..]);
        } else {
            break;
        }
    }

    // Remove empty <p></p> (with optional whitespace)
    loop {
        let trimmed = content.replace("<p></p>", "");
        // Also handle <p> </p> with whitespace
        let mut changed = false;
        if let Some(start) = trimmed.find("<p>") {
            let after_p = &trimmed[start + 3..];
            if let Some(end) = after_p.find("</p>") {
                let between = &after_p[..end];
                if between.trim().is_empty() {
                    content = format!(
                        "{}{}",
                        &trimmed[..start],
                        &trimmed[start + 3 + end + 4..]
                    );
                    changed = true;
                }
            }
        }
        if !changed {
            content = trimmed;
            break;
        }
    }

    // Convert <h2> to <h4>
    content = content.replace("<h2>", "<h4>");
    content = content.replace("<h2 ", "<h4 ");
    content = content.replace("</h2>", "</h4>");

    // Clean up excess blank lines
    while content.contains("\n\n\n") {
        content = content.replace("\n\n\n", "\n\n");
    }

    content.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_svg_ids_unique() {
        let svg = r#"<svg><defs><linearGradient id="grad1"><stop/></linearGradient></defs><path fill="url(#grad1)"/></svg>"#;
        let result = make_svg_ids_unique(svg, "mobile");
        assert!(result.contains("id=\"grad1_mobile\""));
        assert!(result.contains("url(#grad1_mobile)"));
    }

    #[test]
    fn test_strip_help_html() {
        let html = r#"<html>
<head><title>Test</title><style>body{}</style></head>
<body>
<h1>Module Name</h1>
<h2>Summary</h2>
<p>Some description.</p>
<p><img src="chart.png"></p>
<h2>Warning</h2>
<p>Warning text.</p>
</body>
</html>"#;
        let result = strip_help_html(html);
        assert!(!result.contains("<html"));
        assert!(!result.contains("<head"));
        assert!(!result.contains("<body"));
        assert!(!result.contains("<h1"));
        assert!(!result.contains("<img"));
        assert!(result.contains("<h4>Summary</h4>"));
        assert!(result.contains("<h4>Warning</h4>"));
        assert!(result.contains("Some description."));
    }

    #[test]
    fn test_get_help_text_known_module() {
        let help = get_help_text("Basic Statistics");
        assert!(help.contains("statistics"));
        assert!(!help.contains("<html"));
    }

    #[test]
    fn test_get_help_text_unknown_module() {
        let help = get_help_text("Unknown Module");
        assert!(help.contains("not available"));
    }

    #[test]
    fn test_get_help_text_case_insensitive() {
        // Module names have mixed case; lookup should be case-insensitive
        let help1 = get_help_text("Adapter Content");
        let help2 = get_help_text("adapter content");
        assert_eq!(help1, help2);
    }
}
