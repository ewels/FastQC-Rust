// Classic HTML report template — produces byte-identical output to Java FastQC.
//
// Static HTML/CSS fragments live under assets/templates/classic/ for easier editing.
// Assembly uses `{{PLACEHOLDER}}` replacement; output must remain identical to the
// pre-refactor version (compact XMLStreamWriter-style HTML with no extra whitespace).

use std::io::{self, Write};

use chrono::Local;

use crate::modules::{ModuleStatus, QCModule};
use crate::report::charts::png_to_data_uri;
use crate::report::html::{escape_xml, format_java_date};
use crate::report::templates::ReportTemplate;
use crate::VERSION;

// Embed icons at compile time, matching Templates/Icons/ in the Java jar.
const ICON_FASTQC: &[u8] = include_bytes!("../../../assets/icons/fastqc_icon.png");
const ICON_TICK: &[u8] = include_bytes!("../../../assets/icons/tick.png");
const ICON_WARNING: &[u8] = include_bytes!("../../../assets/icons/warning.png");
const ICON_ERROR: &[u8] = include_bytes!("../../../assets/icons/error.png");

const REPORT_TEMPLATE: &str =
    include_str!("../../../assets/templates/classic/report_template.html");
const MODULE_WRAPPER: &str =
    include_str!("../../../assets/templates/classic/module_wrapper.html");
const SIDEBAR_ITEM: &str = include_str!("../../../assets/templates/classic/sidebar_item.html");
const CSS: &str = include_str!("../../../assets/templates/classic/fastqc.css");

pub struct ClassicTemplate;

impl ReportTemplate for ClassicTemplate {
    fn write_html_report(
        &self,
        modules: &[Box<dyn QCModule>],
        filename: &str,
        w: &mut dyn Write,
    ) -> io::Result<()> {
        let now = Local::now();
        let date_str = format_java_date(&now);
        let fastqc_icon_uri = png_to_data_uri(ICON_FASTQC);

        // Build sidebar items
        let mut summary_items = String::new();
        for (i, module) in modules.iter().enumerate() {
            if module.ignore_in_report() {
                continue;
            }
            let (icon, alt) = match module.status() {
                ModuleStatus::Pass => (ICON_TICK, "[PASS]"),
                ModuleStatus::Warn => (ICON_WARNING, "[WARNING]"),
                ModuleStatus::Fail => (ICON_ERROR, "[FAIL]"),
            };
            let item = SIDEBAR_ITEM
                .trim_end()
                .replace("{{MODULE_INDEX}}", &i.to_string())
                .replace("{{ICON_URI}}", &png_to_data_uri(icon))
                .replace("{{ALT_TEXT}}", alt)
                .replace("{{MODULE_NAME}}", &escape_xml(module.name()));
            summary_items.push_str(&item);
        }

        // Build module content
        let mut module_content = String::new();
        for (i, module) in modules.iter().enumerate() {
            if module.ignore_in_report() {
                continue;
            }

            let (icon, alt) = match module.status() {
                ModuleStatus::Pass => (ICON_TICK, "[OK]"),
                ModuleStatus::Warn => (ICON_WARNING, "[WARN]"),
                ModuleStatus::Fail => (ICON_ERROR, "[FAIL]"),
            };

            let mut module_buf = Vec::new();
            module.write_html_report(&mut module_buf)?;
            let module_html =
                String::from_utf8(module_buf).map_err(|e| io::Error::other(e.to_string()))?;

            let wrapped = MODULE_WRAPPER
                .trim_end()
                .replace("{{MODULE_INDEX}}", &i.to_string())
                .replace("{{ICON_URI}}", &png_to_data_uri(icon))
                .replace("{{ALT_TEXT}}", alt)
                .replace("{{MODULE_NAME}}", &escape_xml(module.name()))
                .replace("{{MODULE_CONTENT}}", &module_html);
            module_content.push_str(&wrapped);
        }

        // CSS is entity-escaped like Java XMLStreamWriter.writeCharacters()
        let css_escaped = escape_xml(CSS);

        let html = REPORT_TEMPLATE
            .trim_end()
            .replace("{{TITLE}}", &escape_xml(filename))
            .replace("{{CSS_CONTENT}}", &css_escaped)
            .replace("{{FASTQC_ICON_URI}}", &fastqc_icon_uri)
            .replace("{{DATE}}", &escape_xml(&date_str))
            .replace("{{FILENAME}}", &escape_xml(filename))
            .replace("{{SUMMARY_ITEMS}}", &summary_items)
            .replace("{{MODULE_CONTENT}}", &module_content)
            .replace("{{VERSION}}", VERSION);

        w.write_all(html.as_bytes())?;
        Ok(())
    }
}
