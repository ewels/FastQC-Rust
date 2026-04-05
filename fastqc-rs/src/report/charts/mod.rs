pub mod line_graph;
pub mod quality_boxplot;
pub mod tile_graph;

/// A simple RGB color struct used across all chart types.
#[derive(Debug, Clone, Copy)]
pub struct ChartColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl ChartColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        ChartColor { r, g, b }
    }

    pub fn to_rgb_string(&self) -> String {
        format!("rgb({},{},{})", self.r, self.g, self.b)
    }
}

// Default chart dimensions match Java's JPanel.getPreferredSize() = 800x600
pub const CHART_WIDTH: f64 = 800.0;
pub const CHART_HEIGHT: f64 = 600.0;

// Tol colorblind-safe palette from LineGraph.java
// Note: Java FastQC updated these colours from the original bright palette
// to the Tol scheme at https://davidmathlogic.com/colorblind/
pub const LINE_COLOURS: [ChartColor; 8] = [
    ChartColor::new(136, 34, 85),   // Purple-red
    ChartColor::new(51, 34, 136),   // Indigo
    ChartColor::new(17, 119, 51),   // Green
    ChartColor::new(221, 204, 119), // Yellow-green
    ChartColor::new(68, 170, 153),  // Teal
    ChartColor::new(170, 68, 153),  // Magenta
    ChartColor::new(204, 102, 119), // Pink
    ChartColor::new(136, 204, 238), // Light blue
];

// Java uses Font("Default", Font.PLAIN, 12) for all chart text.
// The SVGGenerator writes font-size="12" but Java AWT renders 12pt at screen DPI
// (typically 96dpi) where it appears slightly larger than SVG's 12px.
const FONT_SIZE: f64 = 12.0;
/// Bold text in Java AWT is approximately 13% wider than plain text.
/// Used when measuring text that will be rendered with font-weight="bold".
pub const BOLD_WIDTH_SCALE: f64 = 1.13;
// Java uses SansSerif which maps to Arial/Helvetica. We use Liberation Sans
// (bundled, metric-compatible with Arial) to avoid system font dependency.
const FONT_FAMILY: &str = "Liberation Sans";

/// Approximate the width of a string in pixels at 12pt Arial.
/// Java uses FontMetrics.stringWidth() which measures actual glyph widths.
/// We approximate with 7px per character (roughly correct for 12pt Arial),
/// which gives close-enough layout to match Java output visually.
pub fn approx_text_width(s: &str) -> f64 {
    // Approximate Java's FontMetrics.stringWidth() for Arial 12pt.
    // Digits and uppercase are ~7px, lowercase ~5.5px, spaces ~3px.
    // This produces correct overlap prevention for axis labels (mostly digits)
    // AND correct centering for titles (mixed case text).
    s.chars()
        .map(|c| match c {
            ' ' => 3.4,
            '.' | ',' | ':' | ';' | '!' | 'i' | 'l' | '|' | '(' | ')' => 3.5,
            'm' | 'w' | 'M' | 'W' => 9.0,
            'A'..='Z' | '0'..='9' | '%' | '+' | '>' | '#' => 8.2,
            _ => 5.7, // lowercase and other chars
        })
        .sum()
}

/// Generate the SVG header.
pub fn svg_header(width: f64, height: f64) -> String {
    // Match the SVG header format from SVGGenerator.java
    format!(
        "<?xml version=\"1.0\" standalone=\"no\"?>\n\
         <!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\n\
         <svg width=\"{}\" height=\"{}\" version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\">\n",
        width as i32, height as i32
    )
}

/// Generate the SVG footer.
pub fn svg_footer() -> &'static str {
    "</svg>\n"
}

/// Escape text for safe embedding in XML (SVG, XSL-FO, etc.).
pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Emit an SVG text element at the default label font size.
pub fn svg_text(x: f64, y: f64, text: &str, color: &ChartColor, bold: bool) -> String {
    svg_text_sized(x, y, text, color, bold, FONT_SIZE)
}

/// Emit an SVG text element at a specific font size.
fn svg_text_sized(x: f64, y: f64, text: &str, color: &ChartColor, bold: bool, size: f64) -> String {
    let weight = if bold { " font-weight=\"bold\"" } else { "" };
    format!(
        "<text x=\"{}\" y=\"{}\" fill=\"{}\" font-family=\"{}\" font-size=\"{}\"{}>{}</text>\n",
        x as i32,
        y as i32,
        color.to_rgb_string(),
        FONT_FAMILY,
        size as i32,
        weight,
        xml_escape(text)
    )
}

/// Emit an SVG line element.
pub fn svg_line(x1: f64, y1: f64, x2: f64, y2: f64, color: &ChartColor, stroke_width: f64) -> String {
    // crispEdges disables antialiasing so axis lines and whiskers render pixel-sharp
    format!(
        "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"{}\" shape-rendering=\"crispEdges\"/>\n",
        x1 as i32,
        y1 as i32,
        x2 as i32,
        y2 as i32,
        color.to_rgb_string(),
        stroke_width as i32
    )
}

/// Emit an SVG filled rectangle.
pub fn svg_rect_filled(x: f64, y: f64, width: f64, height: f64, color: &ChartColor) -> String {
    // crispEdges ensures rectangles render pixel-sharp without antialiased edges
    format!(
        "<rect width=\"{}\" height=\"{}\" x=\"{}\" y=\"{}\" style=\"fill:{};stroke:none\" shape-rendering=\"crispEdges\"/>\n",
        width as i32,
        height as i32,
        x as i32,
        y as i32,
        color.to_rgb_string()
    )
}

/// Emit an SVG stroked rectangle (no fill).
pub fn svg_rect_stroked(x: f64, y: f64, width: f64, height: f64, color: &ChartColor) -> String {
    format!(
        "<rect width=\"{}\" height=\"{}\" x=\"{}\" y=\"{}\" style=\"fill:none;stroke-width:1;stroke:{}\" shape-rendering=\"crispEdges\"/>\n",
        width as i32,
        height as i32,
        x as i32,
        y as i32,
        color.to_rgb_string()
    )
}

/// Emit an SVG filled + stroked rectangle.
pub fn svg_rect_both(
    x: f64, y: f64, width: f64, height: f64,
    fill: &ChartColor, stroke: &ChartColor,
) -> String {
    format!(
        "<rect width=\"{}\" height=\"{}\" x=\"{}\" y=\"{}\" style=\"fill:{};stroke-width:1;stroke:{}\" shape-rendering=\"crispEdges\"/>\n",
        width as i32,
        height as i32,
        x as i32,
        y as i32,
        fill.to_rgb_string(),
        stroke.to_rgb_string()
    )
}

/// Replicates findOptimalYInterval() from LineGraph.java.
/// Finds a nice round interval so there are at most 10 gridlines.
pub fn find_optimal_y_interval(max: f64) -> f64 {
    let mut base = 1.0_f64;
    let divisions = [1.0, 2.0, 2.5, 5.0];

    loop {
        for &d in &divisions {
            let tester = base * d;
            if max / tester <= 10.0 {
                return tester;
            }
        }
        base *= 10.0;
    }
}

/// Format a Y-axis label, stripping trailing ".0" to match Java's behaviour.
/// Java uses `(""+i).replaceAll(".0$", "")`.
pub fn format_y_label(value: f64) -> String {
    let s = format!("{}", value);
    if s.ends_with(".0") {
        s[..s.len() - 2].to_string()
    } else {
        s
    }
}

/// Render a bold, centered title in the plot area.
/// Title is bold and centered between x_offset and the right edge (width - 10).
pub fn render_centered_title(svg: &mut String, title: &str, x_offset: f64, width: f64) {
    let black = ChartColor::new(0, 0, 0);
    let title_w = approx_text_width(title) * BOLD_WIDTH_SCALE;
    let plot_area_center = x_offset + (width - x_offset - 10.0) / 2.0;
    let title_x = plot_area_center - title_w / 2.0;
    svg.push_str(&svg_text(title_x, 30.0, title, &black, true));
}

/// Shared layout state for charts with numeric Y-axis and categorical X-axis.
///
/// Encapsulates the common chart setup pattern used by LineGraph.java
/// and QualityBoxPlot.java: computing xOffset from Y-axis label widths, y_start
/// from the Y interval, and the get_y() mapping function.
pub struct ChartLayout {
    pub width: f64,
    pub height: f64,
    pub x_offset: f64,
    pub y_start: f64,
    pub y_interval: f64,
    pub min_y: f64,
    pub max_y: f64,
}

impl ChartLayout {
    /// Create a chart layout by computing x_offset from Y-axis label widths.
    pub fn new(min_y: f64, max_y: f64, y_interval: f64) -> Self {
        let width = CHART_WIDTH;
        let height = CHART_HEIGHT;

        // yStart calculation matches Java
        let y_start = if min_y % y_interval == 0.0 {
            min_y
        } else {
            y_interval * ((min_y / y_interval) as i64 + 1) as f64
        };

        // Calculate xOffset from widest Y-axis label
        let mut x_offset: f64 = 0.0;
        let mut y_val = y_start;
        while y_val <= max_y + y_interval * 0.001 {
            let label = format_y_label(y_val);
            let w = approx_text_width(&label);
            if w > x_offset {
                x_offset = w;
            }
            y_val += y_interval;
        }
        // Add 5px breathing space
        x_offset += 5.0;

        ChartLayout { width, height, x_offset, y_start, y_interval, min_y, max_y }
    }

    /// getY() maps a data value to a pixel Y coordinate.
    /// y = (height-40) - ((height-80)/(maxY-minY)) * (value - minY)
    pub fn get_y(&self, value: f64) -> f64 {
        let plot_height = self.height - 80.0;
        let y_range = self.max_y - self.min_y;
        (self.height - 40.0) - (plot_height / y_range) * (value - self.min_y)
    }

    /// Calculate the width of each data column in the plot area.
    /// Uses floor() to match Java's integer division truncation:
    /// `int baseWidth = (getWidth()-(xOffset+10))/xLabels.length`
    pub fn base_width(&self, num_points: usize) -> f64 {
        ((self.width - self.x_offset - 10.0) / num_points.max(1) as f64).floor().max(1.0)
    }

    /// Render the shared SVG boilerplate: white background, Y-axis labels,
    /// centered title, X-axis labels (with overlap prevention), axes, and
    /// X-axis label text.
    pub fn render_common_elements(
        &self,
        svg: &mut String,
        title: &str,
        x_categories: &[String],
        x_label: &str,
        num_points: usize,
    ) {
        let black = ChartColor::new(0, 0, 0);
        let base_width = self.base_width(num_points);

        // White background
        svg.push_str(&svg_rect_filled(0.0, 0.0, self.width, self.height, &ChartColor::new(255, 255, 255)));

        // Y-axis labels
        let mut y_val = self.y_start;
        while y_val <= self.max_y + self.y_interval * 0.001 {
            let label = format_y_label(y_val);
            let y_pos = self.get_y(y_val);
            // Y-axis labels are left-aligned at x=2, matching Java's
            // g.drawString(label, 2, getY(i)+(ascent/2))
            let label_x = 2.0;
            // Vertically center on gridline by adding ascent/2 (FONT_SIZE/2 ~ 6px)
            svg.push_str(&svg_text(label_x, y_pos + FONT_SIZE / 2.0, &label, &black, false));
            y_val += self.y_interval;
        }

        render_centered_title(svg, title, self.x_offset, self.width);

        // X-axis labels with overlap prevention
        let mut last_x_label_end: f64 = 0.0;
        for i in 0..num_points {
            if i < x_categories.len() {
                let label = &x_categories[i];
                let label_w = approx_text_width(label);
                let label_x = (base_width / 2.0) + self.x_offset + (base_width * i as f64) - (label_w / 2.0);
                if label_x > last_x_label_end {
                    svg.push_str(&svg_text(label_x, self.height - 25.0, label, &black, false));
                    last_x_label_end = label_x + label_w + 5.0;
                }
            }
        }

        // Axes
        svg.push_str(&svg_line(self.x_offset, self.height - 40.0, self.width - 10.0, self.height - 40.0, &black, 1.0));
        svg.push_str(&svg_line(self.x_offset, self.height - 40.0, self.x_offset, 40.0, &black, 1.0));

        // X-axis label centered below axis
        let x_label_w = approx_text_width(x_label);
        let x_label_x = self.width / 2.0 - x_label_w / 2.0;
        svg.push_str(&svg_text(x_label_x, self.height - 5.0, x_label, &black, false));
    }

    /// Render horizontal gridlines at each Y-axis tick.
    pub fn render_gridlines(&self, svg: &mut String) {
        let grid_color = ChartColor::new(180, 180, 180);
        let mut y_val = self.y_start;
        while y_val <= self.max_y + self.y_interval * 0.001 {
            let y_pos = self.get_y(y_val);
            svg.push_str(&svg_line(self.x_offset, y_pos, self.width - 10.0, y_pos, &grid_color, 1.0));
            y_val += self.y_interval;
        }
    }
}

/// Convert raw PNG bytes to a `data:image/png;base64,...` URI.
/// Matches ImageToBase64.imageToBase64() which produces
/// "data:image/png;base64,..." encoding for BufferedImage rendered charts.
pub fn png_to_data_uri(png_bytes: &[u8]) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::STANDARD as BASE64;
    format!("data:image/png;base64,{}", BASE64.encode(png_bytes))
}

/// Convert an SVG string to PNG bytes.
///
/// In Java, writeDefaultImage() renders the Swing JPanel to a
/// BufferedImage at the specified dimensions, then encodes via ImageIO.write("PNG").
/// We replicate this by parsing the SVG with resvg and rasterizing via tiny-skia.
// Bundled fonts — no system font dependency.
// Liberation Sans is metric-compatible with Arial (SIL Open Font License).
const FONT_REGULAR: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Regular.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../../../assets/fonts/LiberationSans-Bold.ttf");

pub fn svg_to_png(svg: &str, width: u32, height: u32) -> Result<Vec<u8>, String> {
    use resvg::usvg;
    use tiny_skia::Pixmap;

    // Load bundled fonts — no system font dependency
    let mut fontdb = usvg::fontdb::Database::new();
    fontdb.load_font_data(FONT_REGULAR.to_vec());
    fontdb.load_font_data(FONT_BOLD.to_vec());

    let options = usvg::Options {
        fontdb: std::sync::Arc::new(fontdb),
        ..Default::default()
    };

    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|e| format!("Failed to parse SVG: {}", e))?;

    // Create pixel buffer at target dimensions
    let mut pixmap = Pixmap::new(width, height)
        .ok_or_else(|| "Failed to create pixel buffer".to_string())?;

    pixmap.fill(tiny_skia::Color::WHITE);

    // Render at identity transform (1:1 pixel mapping) since our SVG dimensions
    // match the target pixmap exactly. Any non-identity scale causes antialiased
    // sub-pixel interpolation that blurs lines and rectangles.
    resvg::render(&tree, tiny_skia::Transform::identity(), &mut pixmap.as_mut());

    // Encode to PNG
    let mut png_buf = Vec::new();
    {
        let mut encoder = png::Encoder::new(std::io::Cursor::new(&mut png_buf), width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header()
            .map_err(|e| format!("PNG header error: {}", e))?;
        writer.write_image_data(pixmap.data())
            .map_err(|e| format!("PNG write error: {}", e))?;
    }

    Ok(png_buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_optimal_y_interval() {
        // Verify interval calculation matches Java's findOptimalYInterval
        assert_eq!(find_optimal_y_interval(10.0), 1.0);
        assert_eq!(find_optimal_y_interval(20.0), 2.0);
        assert_eq!(find_optimal_y_interval(25.0), 2.5);
        assert_eq!(find_optimal_y_interval(50.0), 5.0);
        assert_eq!(find_optimal_y_interval(100.0), 10.0);
        assert_eq!(find_optimal_y_interval(200.0), 20.0);
    }

    #[test]
    fn test_format_y_label() {
        // Trailing .0 should be stripped
        assert_eq!(format_y_label(10.0), "10");
        assert_eq!(format_y_label(2.5), "2.5");
        assert_eq!(format_y_label(0.0), "0");
    }

    #[test]
    fn test_chart_color_rgb_string() {
        let c = ChartColor::new(255, 128, 0);
        assert_eq!(c.to_rgb_string(), "rgb(255,128,0)");
    }

    #[test]
    fn test_line_graph_renders_valid_svg() {
        use crate::report::charts::line_graph::{LineGraphData, render_line_graph};

        let svg = render_line_graph(&LineGraphData {
            data: vec![vec![1.0, 5.0, 3.0]],
            min_y: 0.0,
            max_y: 10.0,
            x_label: "X".to_string(),
            series_names: vec!["Series 1".to_string()],
            x_categories: vec!["A".to_string(), "B".to_string(), "C".to_string()],
            title: "Test Graph".to_string(),
        });

        assert!(svg.starts_with("<?xml version"));
        assert!(svg.contains("<svg "));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("Test Graph"));
        assert!(svg.contains("Series 1"));
    }

    #[test]
    fn test_quality_boxplot_renders_valid_svg() {
        use crate::report::charts::quality_boxplot::{QualityBoxPlotData, render_quality_boxplot};

        let svg = render_quality_boxplot(&QualityBoxPlotData {
            means: vec![30.0, 28.0],
            medians: vec![31.0, 29.0],
            lower_quartile: vec![25.0, 24.0],
            upper_quartile: vec![35.0, 33.0],
            lowest: vec![20.0, 18.0],
            highest: vec![38.0, 36.0],
            min_y: 0.0,
            max_y: 40.0,
            y_interval: 2.0,
            x_labels: vec!["1".to_string(), "2".to_string()],
            title: "Test Quality".to_string(),
        });

        assert!(svg.starts_with("<?xml version"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("Test Quality"));
        // Check for quality zone colors
        assert!(svg.contains("rgb(195,230,195)")); // GOOD color
        assert!(svg.contains("rgb(240,240,0)"));   // BOX_FILL color
    }

    #[test]
    fn test_tile_graph_renders_valid_svg() {
        use crate::report::charts::tile_graph::{TileGraphData, render_tile_graph};

        let svg = render_tile_graph(&TileGraphData {
            x_labels: vec!["1".to_string(), "2".to_string()],
            tiles: vec![1101, 1102],
            tile_base_means: vec![
                vec![0.5, -0.3],
                vec![-1.0, 0.2],
            ],
            color_scale_max: 5.0,
        });

        assert!(svg.starts_with("<?xml version"));
        assert!(svg.contains("</svg>"));
        assert!(svg.contains("Quality per tile"));
    }
}
