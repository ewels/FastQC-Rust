// Line graph chart rendering
// Corresponds to Graphs/LineGraph.java
//
// Generates SVG output that visually matches Java FastQC's LineGraph.
// The Java version renders via Swing Graphics2D and then captures SVG
// through SVGGenerator.java. We produce clean SVG directly.

use super::{
    approx_text_width, find_optimal_y_interval, render_centered_title, svg_footer, svg_header,
    svg_rect_filled, svg_rect_stroked, svg_text, ChartColor, ChartLayout, BOLD_WIDTH_SCALE,
    LINE_COLOURS,
};

/// Parameters for drawing a line graph.
pub struct LineGraphData {
    /// One inner Vec per data series. All series should have the same length.
    pub data: Vec<Vec<f64>>,
    /// Minimum Y-axis value.
    pub min_y: f64,
    /// Maximum Y-axis value.
    pub max_y: f64,
    /// Label below the X axis (e.g. "Position in read (bp)").
    pub x_label: String,
    /// Legend names for each data series.
    pub series_names: Vec<String>,
    /// X-axis category labels (one per data point).
    pub x_categories: Vec<String>,
    /// Chart title.
    pub title: String,
}

/// Render a line graph as SVG.
///
/// Layout closely follows LineGraph.java:paint():
/// - 40px margin at bottom, 40px at top
/// - Y-axis labels right-aligned to axis, with xOffset computed from widest label + 5px
/// - Title centered between xOffset and right edge
/// - Alternating grey/white column backgrounds
/// - X-axis labels placed only when they don't overlap
/// - Gridlines at each Y-axis tick
/// - Data lines with 1px stroke
/// - Legend box at top-right
pub fn render_line_graph(params: &LineGraphData) -> String {
    let y_interval = find_optimal_y_interval(params.max_y);
    let layout = ChartLayout::new(params.min_y, params.max_y, y_interval);

    let num_points = if params.data.is_empty() || params.data[0].is_empty() {
        1
    } else {
        params.data[0].len()
    };
    let base_width = layout.base_width(num_points);
    let half_bw = layout.half_base_width(num_points);

    let mut svg = svg_header(layout.width, layout.height);

    // Match Java's LineGraph.paint() order
    layout.render_background(&mut svg);
    layout.render_y_labels(&mut svg);
    render_centered_title(&mut svg, &params.title, layout.x_offset, layout.width);
    layout.render_axes(&mut svg);
    layout.render_x_axis_label(&mut svg, &params.x_label);

    // Alternating bg rects + x-category labels (interleaved per position, matching Java)
    let mut last_x_label_end: f64 = 0.0;
    for i in 0..num_points {
        if i % 2 != 0 {
            svg.push_str(&svg_rect_filled(
                layout.x_offset + base_width * i as f64,
                40.0,
                base_width,
                layout.height - 80.0,
                &ChartColor::new(230, 230, 230),
            ));
        }
        if i < params.x_categories.len() {
            last_x_label_end = layout.render_x_category_label_at(
                &mut svg,
                &params.x_categories[i],
                i,
                base_width,
                last_x_label_end,
            );
        }
    }

    // Horizontal gridlines
    layout.render_gridlines(&mut svg);

    // Draw data lines as individual line segments to match Java's SVG structure.
    // Java uses BasicStroke(2) for rendering, so visual PNG has 2px-wide lines
    // even though SVG captures stroke-width="1". We use stroke-width="2" for PNG.
    for (d, series) in params.data.iter().enumerate() {
        let color = &LINE_COLOURS[d % LINE_COLOURS.len()];
        if series.len() < 2 {
            continue;
        }
        let mut prev_x = 0i32;
        let mut prev_y = 0i32;
        for (i, &val) in series.iter().enumerate() {
            let x = (half_bw + layout.x_offset + (base_width * i as f64)) as i32;
            let y = layout.get_y(val) as i32;
            if i > 0 {
                svg.push_str(&format!(
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"2\"/>\n",
                    prev_x, prev_y, x, y, color.to_rgb_string()
                ));
            }
            prev_x = x;
            prev_y = y;
        }
    }

    // Legend box at top-right
    // Java computes: widestLabel = max(stringWidth(label)) + 6 (3px padding each side)
    // Box x = (getWidth()-10) - widestLabel, box width = widestLabel
    // Box height = 3 + (20 * xTitles.length)
    // Text x = box_x + 3 (3px inside the box)
    if !params.series_names.is_empty() {
        // Find widest label, accounting for bold rendering.
        // Java uses g.setFont(g.getFont().deriveFont(Font.BOLD)) before measuring,
        // making bold text ~13% wider than plain. We scale our approximation accordingly.
        let mut widest_label: f64 = 0.0;
        for name in &params.series_names {
            let w = approx_text_width(name) * BOLD_WIDTH_SCALE;
            if w > widest_label {
                widest_label = w;
            }
        }
        // Add 6px padding (3px each side)
        widest_label += 6.0;

        // legend_x = (getWidth()-10) - widestLabel
        let legend_x = (layout.width - 10.0) - widest_label;
        // legend_height = 3 + (20 * xTitles.length)
        let legend_height = 3.0 + 20.0 * params.series_names.len() as f64;

        // White fill, light grey border
        svg.push_str(&svg_rect_filled(
            legend_x,
            40.0,
            widest_label,
            legend_height,
            &ChartColor::new(255, 255, 255),
        ));
        svg.push_str(&svg_rect_stroked(
            legend_x,
            40.0,
            widest_label,
            legend_height,
            &ChartColor::new(192, 192, 192),
        ));

        // Labels in bold, colored to match series
        // Java: g.drawString(xTitles[t], ((getWidth()-10)-widestLabel)+3, 35+(20*(t+1)))
        for (t, name) in params.series_names.iter().enumerate() {
            let color = &LINE_COLOURS[t % LINE_COLOURS.len()];
            // text_x = legend_x + 3 (3px inside the box)
            let text_x = legend_x + 3.0;
            // y position = 35 + 20*(t+1)
            let text_y = 35.0 + 20.0 * (t as f64 + 1.0);
            svg.push_str(&svg_text(text_x, text_y, name, color, true));
        }
    }

    svg.push_str(svg_footer());
    svg
}
