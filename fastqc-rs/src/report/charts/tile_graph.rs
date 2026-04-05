// Tile quality heatmap rendering
// Corresponds to Graphs/TileGraph.java
//
// Generates SVG output that visually matches Java FastQC's TileGraph.
// Uses a blue-green-red color gradient (HotColdColourGradient) to show
// per-tile quality deviations from the position average.

use super::{
    ChartColor, CHART_HEIGHT, CHART_WIDTH,
    approx_text_width, render_centered_title,
    svg_footer, svg_header, svg_line, svg_rect_filled, svg_text,
};

/// Parameters for drawing a tile heatmap.
pub struct TileGraphData {
    pub x_labels: Vec<String>,
    /// Sorted tile IDs
    pub tiles: Vec<i32>,
    /// tile_base_means[tile_idx][base_idx] = deviation from average quality
    /// (already normalized: positive = better, negative = worse)
    pub tile_base_means: Vec<Vec<f64>>,
    /// The error threshold from module config, used for color scaling.
    /// TileGraph.getColour() uses ModuleConfig.getParam("tile","error")
    /// as the max value for the gradient.
    pub color_scale_max: f64,
}

/// Replicates HotColdColourGradient from
/// uk/ac/babraham/FastQC/Utilities/HotColdColourGradient.java.
///
/// The gradient maps values to colors via a two-step process:
/// 1. Pre-build 100 colors using a sqrt-adjusted scale (to emphasize extremes)
/// 2. Map a value to a percentage of the min..max range and pick the color
///
/// The color spectrum goes: Blue -> Green -> Red
struct HotColdGradient {
    colors: [(u8, u8, u8); 100],
}

impl HotColdGradient {
    fn new() -> Self {
        let mut colors = [(0u8, 0u8, 0u8); 100];

        // makeColors() from HotColdColourGradient.java
        let min = -(50.0_f64.sqrt());
        let max = (99.0 - 50.0_f64).sqrt();

        for (c, color) in colors.iter_mut().enumerate() {
            let actual_c = (c as f64 - 50.0).abs();
            let mut corrected = actual_c.sqrt();
            if c < 50 && corrected > 0.0 {
                corrected = -corrected;
            }
            let (r, g, b) = Self::get_rgb(corrected, min, max);
            *color = (r, g, b);
        }

        HotColdGradient { colors }
    }

    /// getRGB() from HotColdColourGradient.java
    /// Maps a value in [min, max] to an RGB color on a blue-green-red gradient.
    fn get_rgb(value: f64, min: f64, max: f64) -> (u8, u8, u8) {
        let diff = max - min;

        let (red, green, blue);

        if value < min + diff * 0.25 {
            // First quarter: blue -> cyan (blue=200, green ramps up, red=0)
            red = 0;
            blue = 200;
            green = (200.0 * ((value - min) / (diff * 0.25))) as i32;
        } else if value < min + diff * 0.5 {
            // Second quarter: cyan -> green (green=200, blue ramps down, red=0)
            red = 0;
            green = 200;
            blue = (200.0 - 200.0 * ((value - (min + diff * 0.25)) / (diff * 0.25))) as i32;
        } else if value < min + diff * 0.75 {
            // Third quarter: green -> yellow (green=200, red ramps up, blue=0)
            blue = 0;
            green = 200;
            red = (200.0 * ((value - (min + diff * 0.5)) / (diff * 0.25))) as i32;
        } else {
            // Fourth quarter: yellow -> red (red=200, green ramps down, blue=0)
            red = 200;
            blue = 0;
            green = (200.0 - 200.0 * ((value - (min + diff * 0.75)) / (diff * 0.25))) as i32;
        }

        (
            red.clamp(0, 255) as u8,
            green.clamp(0, 255) as u8,
            blue.clamp(0, 255) as u8,
        )
    }

    /// getColor() from HotColdColourGradient.java
    fn get_color(&self, value: f64, min: f64, max: f64) -> ChartColor {
        let percentage = (((100.0 * (value - min)) / (max - min)) as i32).clamp(1, 100);
        let (r, g, b) = self.colors[(percentage - 1) as usize];
        ChartColor::new(r, g, b)
    }
}

/// Render a tile quality heatmap as SVG.
///
/// Layout follows TileGraph.java:paint():
/// - Y-axis shows tile IDs (skipping when labels overlap)
/// - X-axis shows base position groups
/// - Each cell colored by deviation from average quality
/// - Color gradient: blue (good) -> green (neutral) -> red (bad)
pub fn render_tile_graph(params: &TileGraphData) -> String {
    let width = CHART_WIDTH;
    let height = CHART_HEIGHT;
    let num_tiles = params.tiles.len();
    let num_bases = params.x_labels.len();

    if num_tiles == 0 || num_bases == 0 {
        // Return minimal SVG for empty data
        let mut svg = svg_header(width, height);
        svg.push_str(&svg_rect_filled(0.0, 0.0, width, height, &ChartColor::new(255, 255, 255)));
        svg.push_str(svg_footer());
        return svg;
    }

    let gradient = HotColdGradient::new();

    // getY(y) = (height-40) - (int)(((height-80)/(double)tiles.length) * y)
    // The (int) cast truncates to integer, eliminating sub-pixel gaps between tile rows.
    let plot_height = height - 80.0;
    let get_y = |y: f64| -> f64 {
        (height - 40.0) - ((plot_height / num_tiles as f64) * y).floor()
    };

    let black = ChartColor::new(0, 0, 0);

    let mut svg = svg_header(width, height);
    svg.push_str(&svg_rect_filled(0.0, 0.0, width, height, &ChartColor::new(255, 255, 255)));

    // Calculate xOffset from tile ID label widths
    let mut x_offset: f64 = 0.0;
    for &tile in &params.tiles {
        let label = format!("{}", tile);
        let w = approx_text_width(&label);
        if w > x_offset {
            x_offset = w;
        }
    }
    x_offset += 5.0;

    // Draw Y-axis tile labels, skipping when they would overlap
    // Left-align labels at x=2, vertically center on gridline
    {
        let font_size = 12.0_f64;
        let mut last_y = 0.0_f64;
        let ascent = 10.0; // approximate font ascent
        for (i, &tile) in params.tiles.iter().enumerate() {
            let label = format!("{}", tile);
            let this_y = get_y(i as f64);
            // Skip if label would overlap previous (thisY + ascent > lastY)
            if i > 0 && this_y + ascent > last_y {
                continue;
            }
            // Left-align labels at x=2, matching Java's g.drawString(label, 2, ...)
            let label_x = 2.0;
            svg.push_str(&svg_text(label_x, this_y + font_size / 2.0, &label, &black, false));
            last_y = this_y;
        }
    }

    // Title is hardcoded "Quality per tile"
    render_centered_title(&mut svg, "Quality per tile", x_offset, width);

    // Draw axes
    svg.push_str(&svg_line(x_offset, height - 40.0, width - 10.0, height - 40.0, &black, 1.0));
    svg.push_str(&svg_line(x_offset, height - 40.0, x_offset, 40.0, &black, 1.0));

    // X-axis label
    {
        let x_label = "Position in read (bp)";
        let x_label_w = approx_text_width(x_label);
        svg.push_str(&svg_text(width / 2.0 - x_label_w / 2.0, height - 5.0, x_label, &black, false));
    }

    // Uses floor() to match Java's integer division truncation:
    // `int baseWidth = (getWidth()-(xOffset+10))/xLabels.length`
    // This eliminates sub-pixel gaps between adjacent heatmap cells.
    let base_width = ((width - x_offset - 10.0) / num_bases as f64).floor().max(1.0);

    // X-axis labels with overlap prevention
    {
        let mut last_x_label_end: f64 = 0.0;
        for (base, label) in params.x_labels.iter().enumerate() {
            let label_w = approx_text_width(label);
            let label_x = (base_width / 2.0) + x_offset + (base_width * base as f64) - (label_w / 2.0);
            if label_x > last_x_label_end {
                svg.push_str(&svg_text(label_x, height - 25.0, label, &black, false));
                last_x_label_end = label_x + label_w + 5.0;
            }
        }
    }

    // Draw heatmap cells
    // The gradient maps deviation values where:
    // - Input to getColor: (0 - deviation), min=0, max=colorScaleMax
    // - So deviation=0 maps to the middle (green), negative deviation maps toward red,
    //   positive deviation maps toward blue
    let color_max = params.color_scale_max;
    for tile in 0..num_tiles {
        for base in 0..num_bases {
            // TileGraph.getColour: gradient.getColor(0-tileBaseMeans[tile][base], 0, error)
            let deviation = params.tile_base_means[tile][base];
            let color_value = -deviation; // 0 - deviation
            let color = gradient.get_color(color_value, 0.0, color_max);

            let x = x_offset + base_width * base as f64;
            // y = getY(tile+1), height = getY(tile) - getY(tile+1)
            let y = get_y((tile + 1) as f64);
            let cell_height = get_y(tile as f64) - y;
            svg.push_str(&svg_rect_filled(x, y, base_width, cell_height, &color));
        }
    }

    svg.push_str(svg_footer());
    svg
}
