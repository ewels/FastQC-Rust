/**
 * Copyright Copyright 2024 Simon Andrews
 *
 *    This file is part of FastQC.
 *
 *    FastQC is free software; you can redistribute it and/or modify
 *    it under the terms of the GNU General Public License as published by
 *    the Free Software Foundation; either version 3 of the License, or
 *    (at your option) any later version.
 *
 *    FastQC is distributed in the hope that it will be useful,
 *    but WITHOUT ANY WARRANTY; without even the implied warranty of
 *    MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 *    GNU General Public License for more details.
 *
 *    You should have received a copy of the GNU General Public License
 *    along with FastQC; if not, write to the Free Software
 *    Foundation, Inc., 51 Franklin St, Fifth Floor, Boston, MA  02110-1301  USA
 */
package uk.ac.babraham.FastQC.Utilities;

import java.text.DecimalFormat;
import java.text.DecimalFormatSymbols;

public class EChartsGenerator {

    private static final String[] COLORS = {
        "#882255", "#3322AA", "#117733", "#DDCC77",
        "#44AA99", "#AA4499", "#CC6677", "#88CCEE"
    };

    private static final DecimalFormat df;

    static {
        df = new DecimalFormat("#.##");
        // Force dot as decimal separator for JavaScript compatibility
        DecimalFormatSymbols symbols = df.getDecimalFormatSymbols();
        symbols.setDecimalSeparator('.');
        df.setDecimalFormatSymbols(symbols);
    }

    /**
     * Generate ECharts configuration for a box plot (quality scores)
     */
    public static String generateBoxPlotConfig(String containerId, double[] means, double[] medians,
                                             double[] lowest, double[] highest, double[] lowerQuartile,
                                             double[] upperQuartile, double minY, double maxY,
                                             String[] xLabels, String title) {

        StringBuilder sb = new StringBuilder();
        sb.append("var chart_").append(containerId).append(" = echarts.init(document.getElementById('").append(containerId).append("'));\n");
        sb.append("var option_").append(containerId).append(" = {\n");
        sb.append("  title: { text: '").append(escapeString(title)).append("', left: 'center', top: 10 },\n");
        sb.append("  tooltip: { trigger: 'item' },\n");
        sb.append("  grid: { left: 60, right: 30, top: 60, bottom: 80 },\n");
        sb.append("  xAxis: {\n");
        sb.append("    type: 'category',\n");
        sb.append("    data: [");
        for (int i = 0; i < xLabels.length; i++) {
            if (i > 0) sb.append(", ");
            sb.append("'").append(escapeString(xLabels[i])).append("'");
        }
        sb.append("],\n");
        sb.append("    name: 'Position in read (bp)',\n");
        sb.append("    nameLocation: 'middle',\n");
        sb.append("    nameGap: 30\n");
        sb.append("  },\n");
        sb.append("  yAxis: {\n");
        sb.append("    type: 'value',\n");
        sb.append("    min: ").append(minY).append(",\n");
        sb.append("    max: ").append(maxY).append(",\n");
        sb.append("    axisLine: { show: true },\n");
        sb.append("    axisTick: { show: true }\n");
        sb.append("  },\n");

        // Background zones for quality ranges
        sb.append("  visualMap: {\n");
        sb.append("    show: false,\n");
        sb.append("    pieces: [\n");
        sb.append("      { gte: 28, color: 'rgba(195,230,195,0.3)' },\n");
        sb.append("      { gte: 20, lt: 28, color: 'rgba(230,220,195,0.3)' },\n");
        sb.append("      { lt: 20, color: 'rgba(230,195,195,0.3)' }\n");
        sb.append("    ]\n");
        sb.append("  },\n");

        sb.append("  series: [\n");

        // Box plot series
        sb.append("    {\n");
        sb.append("      type: 'boxplot',\n");
        sb.append("      data: [");
        for (int i = 0; i < medians.length; i++) {
            if (i > 0) sb.append(", ");
            sb.append("[").append(df.format(lowest[i])).append(", ")
              .append(df.format(lowerQuartile[i])).append(", ")
              .append(df.format(medians[i])).append(", ")
              .append(df.format(upperQuartile[i])).append(", ")
              .append(df.format(highest[i])).append("]");
        }
        sb.append("],\n");
        sb.append("      itemStyle: { color: 'rgba(240,240,0,0.8)', borderColor: '#000' },\n");
        sb.append("      tooltip: {\n");
        sb.append("        formatter: function(params) {\n");
        sb.append("          return params.name + '<br/>' +\n");
        sb.append("            'Upper: ' + params.data[4] + '<br/>' +\n");
        sb.append("            'Q3: ' + params.data[3] + '<br/>' +\n");
        sb.append("            'Median: ' + params.data[2] + '<br/>' +\n");
        sb.append("            'Q1: ' + params.data[1] + '<br/>' +\n");
        sb.append("            'Lower: ' + params.data[0];\n");
        sb.append("        }\n");
        sb.append("      }\n");
        sb.append("    },\n");

        // Mean line series
        sb.append("    {\n");
        sb.append("      type: 'line',\n");
        sb.append("      name: 'Mean',\n");
        sb.append("      data: [");
        for (int i = 0; i < means.length; i++) {
            if (i > 0) sb.append(", ");
            sb.append(df.format(means[i]));
        }
        sb.append("],\n");
        sb.append("      lineStyle: { color: '#0000C8', width: 2 },\n");
        sb.append("      symbol: 'none'\n");
        sb.append("    }\n");
        sb.append("  ]\n");
        sb.append("};\n");
        sb.append("chart_").append(containerId).append(".setOption(option_").append(containerId).append(");\n");

        return sb.toString();
    }

    /**
     * Generate ECharts configuration for a line graph
     */
    public static String generateLineGraphConfig(String containerId, double[][] data, double minY, double maxY,
                                               String xLabel, String[] xTitles, String[] xCategories, String title) {

        StringBuilder sb = new StringBuilder();
        sb.append("var chart_").append(containerId).append(" = echarts.init(document.getElementById('").append(containerId).append("'));\n");
        sb.append("var option_").append(containerId).append(" = {\n");
        sb.append("  title: { text: '").append(escapeString(title)).append("', left: 'center', top: 10 },\n");
        sb.append("  tooltip: { trigger: 'axis' },\n");
        sb.append("  legend: { data: [");
        for (int i = 0; i < xTitles.length; i++) {
            if (i > 0) sb.append(", ");
            sb.append("'").append(escapeString(xTitles[i])).append("'");
        }
        sb.append("], top: 35 },\n");
        sb.append("  grid: { left: 60, right: 30, top: 80, bottom: 80 },\n");
        sb.append("  xAxis: {\n");
        sb.append("    type: 'category',\n");
        sb.append("    data: [");
        for (int i = 0; i < xCategories.length; i++) {
            if (i > 0) sb.append(", ");
            sb.append("'").append(escapeString(xCategories[i])).append("'");
        }
        sb.append("],\n");
        sb.append("    name: '").append(escapeString(xLabel)).append("',\n");
        sb.append("    nameLocation: 'middle',\n");
        sb.append("    nameGap: 30\n");
        sb.append("  },\n");
        sb.append("  yAxis: {\n");
        sb.append("    type: 'value',\n");
        sb.append("    min: ").append(minY).append(",\n");
        sb.append("    max: ").append(maxY).append("\n");
        sb.append("  },\n");
        sb.append("  series: [\n");

        for (int d = 0; d < data.length; d++) {
            if (d > 0) sb.append(",\n");
            sb.append("    {\n");
            sb.append("      name: '").append(escapeString(xTitles[d])).append("',\n");
            sb.append("      type: 'line',\n");
            sb.append("      data: [");
            for (int i = 0; i < data[d].length; i++) {
                if (i > 0) sb.append(", ");
                sb.append(df.format(data[d][i]));
            }
            sb.append("],\n");
            sb.append("      lineStyle: { color: '").append(COLORS[d % COLORS.length]).append("', width: 2 },\n");
            sb.append("      itemStyle: { color: '").append(COLORS[d % COLORS.length]).append("' },\n");
            sb.append("      symbol: 'none'\n");
            sb.append("    }");
        }

        sb.append("\n  ]\n");
        sb.append("};\n");
        sb.append("chart_").append(containerId).append(".setOption(option_").append(containerId).append(");\n");

        return sb.toString();
    }

    /**
     * Generate ECharts configuration for a line graph with integer x-axis
     */
    public static String generateLineGraphConfig(String containerId, double[][] data, double minY, double maxY,
                                               String xLabel, String[] xTitles, int[] xCategories, String title) {
        String[] xCategoriesStr = new String[xCategories.length];
        for (int i = 0; i < xCategories.length; i++) {
            xCategoriesStr[i] = String.valueOf(xCategories[i]);
        }
        return generateLineGraphConfig(containerId, data, minY, maxY, xLabel, xTitles, xCategoriesStr, title);
    }

    private static String escapeString(String input) {
        if (input == null) return "";
        // For JavaScript strings, we only need to escape quotes and backslashes
        // Don't escape HTML entities as they will be double-escaped
        return input.replace("\\", "\\\\")
                   .replace("\"", "\\\"")
                   .replace("'", "\\'")
                   .replace("\n", "\\n")
                   .replace("\r", "\\r");
    }

    public static String generateHeatmapConfig(String containerId, double[][] data, String[] xLabels, int[] yLabels, String title) {
        StringBuilder sb = new StringBuilder();
        sb.append("var chart_").append(containerId).append(" = echarts.init(document.getElementById('").append(containerId).append("'));");
        sb.append("var option_").append(containerId).append(" = {");
        sb.append("  title: { text: '").append(escapeString(title)).append("', left: 'center', top: 10 },");
        sb.append("  tooltip: { position: 'top' },");
        sb.append("  grid: { left: 80, right: 80, top: 80, bottom: 80 },");
        sb.append("  xAxis: { type: 'category', data: [");
        for (int i = 0; i < xLabels.length; i++) {
            if (i > 0) sb.append(",");
            sb.append("'").append(escapeString(xLabels[i])).append("'");
        }
        sb.append("], splitArea: { show: true } },");
        sb.append("  yAxis: { type: 'category', data: [");
        for (int i = 0; i < yLabels.length; i++) {
            if (i > 0) sb.append(",");
            sb.append("'").append(yLabels[i]).append("'");
        }
        sb.append("], splitArea: { show: true } },");

        // Use FastQC's quality color scheme: blue (good) -> green -> yellow -> red (bad)
        // Range: 0 (good) to 10 (bad quality deviation)
        sb.append("  visualMap: { ");
        sb.append("min: 0, max: 10, ");
        sb.append("calculable: true, ");
        sb.append("orient: 'horizontal', ");
        sb.append("left: 'center', ");
        sb.append("bottom: 20, ");
        sb.append("inRange: { ");
        sb.append("color: ['#0000C8', '#0080C8', '#00C8C8', '#00C800', '#C8C800', '#C88000', '#C80000'] ");
        sb.append("} },");

        sb.append("  series: [{ name: 'Quality', type: 'heatmap', data: [");

        boolean first = true;
        for (int y = 0; y < data.length; y++) {
            for (int x = 0; x < data[y].length; x++) {
                if (!first) sb.append(",");
                // Transform data like the original: 0 - tileBaseMeans[tile][base]
                // This makes higher quality scores (good) become lower values (blue)
                // and lower quality scores (bad) become higher values (red)
                double transformedValue = 0 - data[y][x];
                sb.append("[").append(x).append(",").append(y).append(",").append(df.format(transformedValue)).append("]");
                first = false;
            }
        }

        sb.append("], emphasis: { itemStyle: { shadowBlur: 10, shadowColor: 'rgba(0, 0, 0, 0.5)' } } }]");
        sb.append("};");
        sb.append("chart_").append(containerId).append(".setOption(option_").append(containerId).append(");");

        return sb.toString();
    }

    public static String generateBoxPlotWithQualityZonesConfig(String containerId, double[] means, double[] medians,
                                                             double[] lowest, double[] highest, double[] lowerQuartile,
                                                             double[] upperQuartile, double minY, double maxY,
                                                             String[] xLabels, String title) {
        StringBuilder sb = new StringBuilder();
        sb.append("var chart_").append(containerId).append(" = echarts.init(document.getElementById('").append(containerId).append("'));");
        sb.append("var option_").append(containerId).append(" = {");
        sb.append("  title: { text: '").append(escapeString(title)).append("', left: 'center', top: 10 },");
        sb.append("  tooltip: { trigger: 'axis' },");
        sb.append("  grid: { left: 80, right: 80, top: 80, bottom: 80 },");

        sb.append("  xAxis: { type: 'category', name: 'Position in read (bp)', nameLocation: 'middle', nameGap: 30, data: [");
        for (int i = 0; i < xLabels.length; i++) {
            if (i > 0) sb.append(",");
            sb.append("'").append(escapeString(xLabels[i])).append("'");
        }
        sb.append("] },");

        sb.append("  yAxis: { type: 'value', name: 'Quality Score', nameLocation: 'middle', nameGap: 50, min: 0, max: 40 },");

        // Add quality zone backgrounds using markArea
        sb.append("  series: [");

                // Background zones - using invisible line series with markArea (no labels)
        sb.append("{ name: 'Quality Zones', type: 'line', data: [], showInLegend: false, ");
        sb.append("markArea: { silent: true, itemStyle: { opacity: 0.8 }, label: { show: false }, data: [");
        sb.append("[{ yAxis: 0, itemStyle: { color: '#f0c6bc' } }, { yAxis: 20 }],");  // Bottom zone (poor quality)
        sb.append("[{ yAxis: 20, itemStyle: { color: '#e8d28f' } }, { yAxis: 28 }],"); // Middle zone (moderate quality)
        sb.append("[{ yAxis: 28, itemStyle: { color: '#afe0b7' } }, { yAxis: 40 }]");  // Top zone (good quality)
        sb.append("] } },");

        // Mean quality line (blue)
        sb.append("{ name: 'Mean', type: 'line', ");
        sb.append("lineStyle: { color: '#0066CC', width: 2 }, ");
        sb.append("itemStyle: { color: '#0066CC' }, ");
        sb.append("symbol: 'none', ");
        sb.append("data: [");
        for (int i = 0; i < means.length; i++) {
            if (i > 0) sb.append(",");
            sb.append(df.format(means[i]));
        }
        sb.append("] },");

        // Box plot data with custom styling
        sb.append("{ name: 'Quality Scores', type: 'boxplot', ");
        sb.append("boxWidth: ['7', '99%'], ");
        sb.append("itemStyle: { ");
        sb.append("color: '#FFFF00', ");           // Yellow box fill
        sb.append("borderColor: '#000000', ");     // Black box outline
        sb.append("borderWidth: 1 ");
        sb.append("}, ");
        sb.append("emphasis: { ");
        sb.append("itemStyle: { ");
        sb.append("color: '#FFFF00', ");
        sb.append("borderColor: '#000000', ");
        sb.append("borderWidth: 2 ");
        sb.append("} }, ");
        // Configure median line color
        sb.append("medianStyle: { ");
        sb.append("color: '#FF0000', ");           // Red median line
        sb.append("width: 2 ");
        sb.append("}, ");
        sb.append("data: [");

        for (int i = 0; i < means.length; i++) {
            if (i > 0) sb.append(",");
            // Box plot data: [min, Q1, median, Q3, max]
            sb.append("[").append(df.format(lowest[i])).append(",").append(df.format(lowerQuartile[i])).append(",")
              .append(df.format(medians[i])).append(",").append(df.format(upperQuartile[i])).append(",").append(df.format(highest[i])).append("]");
        }

                sb.append("] }");
        sb.append("] };");
        sb.append("chart_").append(containerId).append(".setOption(option_").append(containerId).append(");");

        return sb.toString();
    }

    public static String generateQualityDistributionConfig(String containerId, double[] data, int[] xCategories, double maxY, String title) {
        StringBuilder sb = new StringBuilder();
        sb.append("var chart_").append(containerId).append(" = echarts.init(document.getElementById('").append(containerId).append("'));");
        sb.append("var option_").append(containerId).append(" = {");
        sb.append("  title: { text: '").append(escapeString(title)).append("', left: 'center', top: 10 },");
        sb.append("  tooltip: { trigger: 'axis', formatter: function(params) { return params[0].name + '<br/>Count: ' + params[0].value; } },");
        sb.append("  grid: { left: 80, right: 80, top: 80, bottom: 80 },");

        // Add background quality zones
        sb.append("  xAxis: { type: 'category', name: 'Mean Sequence Quality (Phred Score)', nameLocation: 'middle', nameGap: 30, data: [");
        for (int i = 0; i < xCategories.length; i++) {
            if (i > 0) sb.append(",");
            sb.append("'").append(xCategories[i]).append("'");
        }
        sb.append("] },");

        sb.append("  yAxis: { type: 'value', name: 'Count', nameLocation: 'middle', nameGap: 50, max: ").append((int)Math.ceil(maxY)).append(" },");

        // Add quality zone backgrounds
        sb.append("  series: [");

        // Red zone (0-20)
        sb.append("{ name: 'Poor Quality', type: 'line', stack: 'zones', areaStyle: { color: 'rgba(255, 0, 0, 0.2)' }, ");
        sb.append("lineStyle: { width: 0 }, symbol: 'none', data: [");
        for (int i = 0; i < xCategories.length; i++) {
            if (i > 0) sb.append(",");
            if (xCategories[i] <= 20) {
                sb.append(maxY);
            } else {
                sb.append("0");
            }
        }
        sb.append("] },");

        // Yellow zone (20-28)
        sb.append("{ name: 'Moderate Quality', type: 'line', stack: 'zones', areaStyle: { color: 'rgba(255, 255, 0, 0.2)' }, ");
        sb.append("lineStyle: { width: 0 }, symbol: 'none', data: [");
        for (int i = 0; i < xCategories.length; i++) {
            if (i > 0) sb.append(",");
            if (xCategories[i] > 20 && xCategories[i] <= 28) {
                sb.append(maxY);
            } else {
                sb.append("0");
            }
        }
        sb.append("] },");

        // Green zone (28+)
        sb.append("{ name: 'Good Quality', type: 'line', stack: 'zones', areaStyle: { color: 'rgba(0, 255, 0, 0.2)' }, ");
        sb.append("lineStyle: { width: 0 }, symbol: 'none', data: [");
        for (int i = 0; i < xCategories.length; i++) {
            if (i > 0) sb.append(",");
            if (xCategories[i] > 28) {
                sb.append(maxY);
            } else {
                sb.append("0");
            }
        }
        sb.append("] },");

        // Actual data line
        sb.append("{ name: 'Average Quality per read', type: 'line', ");
        sb.append("lineStyle: { color: '#0000FF', width: 2 }, ");
        sb.append("itemStyle: { color: '#0000FF' }, ");
        sb.append("data: [");
        for (int i = 0; i < data.length; i++) {
            if (i > 0) sb.append(",");
            sb.append(data[i]);
        }
        sb.append("] }");

        sb.append("] };");
        sb.append("chart_").append(containerId).append(".setOption(option_").append(containerId).append(");");

        return sb.toString();
    }

    public static String generateContinuousLineGraphConfig(String containerId, double[][] data, double minY, double maxY,
                                                         String xLabel, String[] seriesNames, int[] xValues, String title) {
        StringBuilder sb = new StringBuilder();
        sb.append("var chart_").append(containerId).append(" = echarts.init(document.getElementById('").append(containerId).append("'));\n");
        sb.append("var option_").append(containerId).append(" = {\n");
        sb.append("  title: { text: '").append(escapeString(title)).append("', left: 'center', top: 10 },\n");
        sb.append("  tooltip: { trigger: 'axis' },\n");
        sb.append("  legend: { data: [");
        for (int i = 0; i < seriesNames.length; i++) {
            if (i > 0) sb.append(", ");
            sb.append("'").append(escapeString(seriesNames[i])).append("'");
        }
        sb.append("], top: 35 },\n");
        sb.append("  grid: { left: 60, right: 30, top: 80, bottom: 80 },\n");
        sb.append("  xAxis: {\n");
        sb.append("    type: 'value',\n");  // Use 'value' instead of 'category' for continuous data
        sb.append("    name: '").append(escapeString(xLabel)).append("',\n");
        sb.append("    nameLocation: 'middle',\n");
        sb.append("    nameGap: 30,\n");
        sb.append("    min: 0,\n");
        sb.append("    max: 100\n");
        sb.append("  },\n");
        sb.append("  yAxis: {\n");
        sb.append("    type: 'value',\n");
        sb.append("    min: ").append(minY).append(",\n");
        sb.append("    max: ").append(maxY).append("\n");
        sb.append("  },\n");
        sb.append("  series: [\n");

        for (int d = 0; d < data.length; d++) {
            if (d > 0) sb.append(",\n");
            sb.append("    {\n");
            sb.append("      name: '").append(escapeString(seriesNames[d])).append("',\n");
            sb.append("      type: 'line',\n");
            sb.append("      data: [");
            for (int i = 0; i < data[d].length; i++) {
                if (i > 0) sb.append(", ");
                // For continuous axis, data should be [x, y] pairs
                sb.append("[").append(xValues[i]).append(", ").append(df.format(data[d][i])).append("]");
            }
            sb.append("],\n");
            sb.append("      lineStyle: { color: '").append(COLORS[d % COLORS.length]).append("', width: 2 },\n");
            sb.append("      itemStyle: { color: '").append(COLORS[d % COLORS.length]).append("' },\n");
            sb.append("      symbol: 'none'\n");
            sb.append("    }");
        }

        sb.append("\n  ]\n");
        sb.append("};\n");
        sb.append("chart_").append(containerId).append(".setOption(option_").append(containerId).append(");\n");

        return sb.toString();
    }
}
