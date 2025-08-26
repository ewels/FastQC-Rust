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

public class EChartsGenerator {

    private static final String[] COLORS = {
        "#882255", "#3322AA", "#117733", "#DDCC77",
        "#44AA99", "#AA4499", "#CC6677", "#88CCEE"
    };

    private static final DecimalFormat df = new DecimalFormat("#.##");

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
        sb.append("  visualMap: { min: 0, max: 40, calculable: true, orient: 'horizontal', left: 'center', bottom: 20 },");
        sb.append("  series: [{ name: 'Quality', type: 'heatmap', data: [");

        boolean first = true;
        for (int y = 0; y < data.length; y++) {
            for (int x = 0; x < data[y].length; x++) {
                if (!first) sb.append(",");
                sb.append("[").append(x).append(",").append(y).append(",").append(data[y][x]).append("]");
                first = false;
            }
        }

        sb.append("], emphasis: { itemStyle: { shadowBlur: 10, shadowColor: 'rgba(0, 0, 0, 0.5)' } } }]");
        sb.append("};");
        sb.append("chart_").append(containerId).append(".setOption(option_").append(containerId).append(");");

        return sb.toString();
    }
}
