// Quality box plot chart rendering
// Corresponds to Graphs/QualityBoxPlot.java
//
// Generates SVG output that visually matches Java FastQC's QualityBoxPlot.
// This is the signature FastQC chart showing per-base quality with colored zones.

use super::{
    render_centered_title, svg_footer, svg_header, svg_line, svg_rect_filled, svg_rect_stroked,
    ChartColor, ChartLayout,
};

/// Parameters for drawing a quality box plot.
pub struct QualityBoxPlotData {
    pub means: Vec<f64>,
    pub medians: Vec<f64>,
    pub lower_quartile: Vec<f64>,
    pub upper_quartile: Vec<f64>,
    /// 10th percentile (bottom whisker)
    pub lowest: Vec<f64>,
    /// 90th percentile (top whisker)
    pub highest: Vec<f64>,
    pub min_y: f64,
    pub max_y: f64,
    pub y_interval: f64,
    pub x_labels: Vec<String>,
    pub title: String,
}

// Quality zone colors from QualityBoxPlot.java
// These exact RGB values match the Java source.
const GOOD: ChartColor = ChartColor::new(195, 230, 195);
const BAD: ChartColor = ChartColor::new(230, 220, 195);
const UGLY: ChartColor = ChartColor::new(230, 195, 195);
const GOOD_DARK: ChartColor = ChartColor::new(175, 230, 175);
const BAD_DARK: ChartColor = ChartColor::new(230, 215, 175);
const UGLY_DARK: ChartColor = ChartColor::new(230, 175, 175);

// Box fill is yellow (240,240,0)
const BOX_FILL: ChartColor = ChartColor::new(240, 240, 0);
// Median line is red (200,0,0)
const MEDIAN_COLOR: ChartColor = ChartColor::new(200, 0, 0);
// Mean line is blue (0,0,200)
const MEAN_COLOR: ChartColor = ChartColor::new(0, 0, 200);

/// Render a quality box plot as SVG.
///
/// Layout closely follows QualityBoxPlot.java:paint():
/// - Same 40px top/bottom margins
/// - Green (>28), Yellow (20-28), Red (<20) background zones
/// - Alternating light/dark within zones
/// - Yellow boxes for IQR, whiskers for 10th/90th percentile
/// - Red median line, blue mean line connecting all positions
pub fn render_quality_boxplot(params: &QualityBoxPlotData) -> String {
    let layout = ChartLayout::new(params.min_y, params.max_y, params.y_interval);

    let num_positions = params.means.len();
    let base_width = layout.base_width(num_positions);

    let mut svg = svg_header(layout.width, layout.height);

    // Render shared elements: background, Y-axis labels, title, X-axis labels, axes
    // Match Java's QualityBoxPlot.paint() element order
    layout.render_background(&mut svg);
    layout.render_y_labels(&mut svg);
    render_centered_title(&mut svg, &params.title, layout.x_offset, layout.width);

    let black = ChartColor::new(0, 0, 0);

    // Zone backgrounds + x-labels interleaved per position (matching Java paint order)
    let mut last_x_label_end: f64 = 0.0;
    for i in 0..num_positions {
        let x = layout.x_offset + base_width * i as f64;

        // Alternating colors - odd positions get the lighter variant
        let (ugly, bad, good) = if i % 2 != 0 {
            (&UGLY, &BAD, &GOOD)
        } else {
            (&UGLY_DARK, &BAD_DARK, &GOOD_DARK)
        };

        // Red zone: from yStart to quality 20
        let ugly_top = layout.get_y(20.0);
        let ugly_bottom = layout.get_y(layout.y_start);
        if ugly_bottom > ugly_top {
            svg.push_str(&svg_rect_filled(
                x,
                ugly_top,
                base_width,
                ugly_bottom - ugly_top,
                ugly,
            ));
        }

        // Yellow zone: from quality 20 to 28
        let bad_top = layout.get_y(28.0);
        let bad_bottom = layout.get_y(20.0);
        if bad_bottom > bad_top {
            svg.push_str(&svg_rect_filled(
                x,
                bad_top,
                base_width,
                bad_bottom - bad_top,
                bad,
            ));
        }

        // Green zone: from quality 28 to maxY
        let good_top = layout.get_y(params.max_y);
        let good_bottom = layout.get_y(28.0);
        if good_bottom > good_top {
            svg.push_str(&svg_rect_filled(
                x,
                good_top,
                base_width,
                good_bottom - good_top,
                good,
            ));
        }

        // X-category label for this position (interleaved with zone rects)
        if i < params.x_labels.len() {
            last_x_label_end = layout.render_x_category_label_at(
                &mut svg,
                &params.x_labels[i],
                i,
                base_width,
                last_x_label_end,
            );
        }
    }

    // Axes and x-axis label (after zones + x-labels, matching Java)
    layout.render_axes(&mut svg);
    layout.render_x_axis_label(&mut svg, "Position in read (bp)");

    // Draw box plots for each position
    for i in 0..num_positions {
        let box_x = layout.x_offset + base_width * i as f64;
        let box_top_y = layout.get_y(params.upper_quartile[i]);
        let box_bottom_y = layout.get_y(params.lower_quartile[i]);
        let upper_whisker_y = layout.get_y(params.highest[i]);
        let lower_whisker_y = layout.get_y(params.lowest[i]);
        let median_y = layout.get_y(params.medians[i]);
        let center_x = box_x + base_width / 2.0;

        // Box body (yellow fill, black stroke), inset 2px from each side
        let box_inset = 2.0;
        let box_w = base_width - 4.0;
        let box_h = box_bottom_y - box_top_y;
        svg.push_str(&svg_rect_filled(
            box_x + box_inset,
            box_top_y,
            box_w,
            box_h,
            &BOX_FILL,
        ));
        svg.push_str(&svg_rect_stroked(
            box_x + box_inset,
            box_top_y,
            box_w,
            box_h,
            &black,
        ));

        // Upper whisker - vertical line from box top to whisker, horizontal cap
        svg.push_str(&svg_line(
            center_x,
            upper_whisker_y,
            center_x,
            box_top_y,
            &black,
            1.0,
        ));
        svg.push_str(&svg_line(
            box_x + box_inset,
            upper_whisker_y,
            box_x + base_width - box_inset,
            upper_whisker_y,
            &black,
            1.0,
        ));

        // Lower whisker
        svg.push_str(&svg_line(
            center_x,
            lower_whisker_y,
            center_x,
            box_bottom_y,
            &black,
            1.0,
        ));
        svg.push_str(&svg_line(
            box_x + box_inset,
            lower_whisker_y,
            box_x + base_width - box_inset,
            lower_whisker_y,
            &black,
            1.0,
        ));

        // Median line (red)
        svg.push_str(&svg_line(
            box_x + box_inset,
            median_y,
            box_x + base_width - box_inset,
            median_y,
            &MEDIAN_COLOR,
            1.0,
        ));
    }

    // Mean line (blue), connecting all positions as individual line segments
    let half_bw = layout.half_base_width(num_positions);
    if num_positions >= 2 {
        let mut prev_x = 0i32;
        let mut prev_y = 0i32;
        for i in 0..num_positions {
            let x = (half_bw + layout.x_offset + (base_width * i as f64)) as i32;
            let y = layout.get_y(params.means[i]) as i32;
            if i > 0 {
                svg.push_str(&format!(
                    "<line x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" stroke=\"{}\" stroke-width=\"1\"/>\n",
                    prev_x, prev_y, x, y, MEAN_COLOR.to_rgb_string()
                ));
            }
            prev_x = x;
            prev_y = y;
        }
    }

    svg.push_str(svg_footer());
    svg
}
