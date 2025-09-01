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

import java.io.IOException;
import java.io.InputStream;
import java.io.StringWriter;
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
     * Load a JavaScript template from the Templates directory
     */
    private static String loadTemplate(String templatePath) throws IOException {
        InputStream templateStream = EChartsGenerator.class.getResourceAsStream(templatePath);
        if (templateStream == null) {
            throw new IOException("Template not found: " + templatePath);
        }
        StringWriter templateWriter = new StringWriter();
        byte[] buffer = new byte[1024];
        int nRead;
        while ((nRead = templateStream.read(buffer)) != -1) {
            templateWriter.write(new String(buffer, 0, nRead));
        }
        templateStream.close();
        return templateWriter.toString();
    }

    /**
     * Generate ECharts configuration for a box plot (quality scores)
     */
    public static String generateBoxPlotConfig(String containerId, double[] means, double[] medians,
                                             double[] lowest, double[] highest, double[] lowerQuartile,
                                             double[] upperQuartile, double minY, double maxY,
                                             String[] xLabels, String title) {
        try {
            String template = loadTemplate("/Templates/Plots/boxplot.js");

            // Build X labels
            StringBuilder xLabelsStr = new StringBuilder();
            for (int i = 0; i < xLabels.length; i++) {
                if (i > 0) xLabelsStr.append(", ");
                xLabelsStr.append("'").append(escapeString(xLabels[i])).append("'");
            }

            // Build boxplot data
            StringBuilder boxplotData = new StringBuilder();
            for (int i = 0; i < medians.length; i++) {
                if (i > 0) boxplotData.append(", ");
                boxplotData.append("[").append(df.format(lowest[i])).append(", ")
                          .append(df.format(lowerQuartile[i])).append(", ")
                          .append(df.format(medians[i])).append(", ")
                          .append(df.format(upperQuartile[i])).append(", ")
                          .append(df.format(highest[i])).append("]");
            }

            // Build mean data
            StringBuilder meanData = new StringBuilder();
            for (int i = 0; i < means.length; i++) {
                if (i > 0) meanData.append(", ");
                meanData.append(df.format(means[i]));
            }

            return template.replace("{{CONTAINER_ID}}", containerId)
                          .replace("{{TITLE}}", escapeString(title))
                          .replace("{{X_LABELS}}", xLabelsStr.toString())
                          .replace("{{MIN_Y}}", String.valueOf(minY))
                          .replace("{{MAX_Y}}", String.valueOf(maxY))
                          .replace("{{BOXPLOT_DATA}}", boxplotData.toString())
                          .replace("{{MEAN_DATA}}", meanData.toString())
;
        } catch (IOException e) {
            throw new RuntimeException("Failed to load chart template", e);
        }
    }

    /**
     * Generate ECharts configuration for a line graph
     */
    public static String generateLineGraphConfig(String containerId, double[][] data, double minY, double maxY,
                                               String xLabel, String[] xTitles, String[] xCategories, String title) {
        try {
            String template = loadTemplate("/Templates/Plots/line_graph.js");
            String seriesTemplate = loadTemplate("/Templates/Plots/line_series.js");

            // Build legend data
            StringBuilder legendData = new StringBuilder();
            for (int i = 0; i < xTitles.length; i++) {
                if (i > 0) legendData.append(", ");
                legendData.append("'").append(escapeString(xTitles[i])).append("'");
            }

            // Build X categories
            StringBuilder xCategoriesStr = new StringBuilder();
            for (int i = 0; i < xCategories.length; i++) {
                if (i > 0) xCategoriesStr.append(", ");
                xCategoriesStr.append("'").append(escapeString(xCategories[i])).append("'");
            }

            // Build series data
            StringBuilder seriesData = new StringBuilder();
            for (int d = 0; d < data.length; d++) {
                if (d > 0) seriesData.append(",\n");

                // Build data array for this series
                StringBuilder dataArray = new StringBuilder();
                for (int i = 0; i < data[d].length; i++) {
                    if (i > 0) dataArray.append(", ");
                    dataArray.append(df.format(data[d][i]));
                }

                String series = seriesTemplate.replace("{{SERIES_NAME}}", escapeString(xTitles[d]))
                                              .replace("{{SERIES_DATA}}", dataArray.toString())
                                              .replace("{{COLOR}}", COLORS[d % COLORS.length]);
                seriesData.append(series);
            }

            return template.replace("{{CONTAINER_ID}}", containerId)
                          .replace("{{TITLE}}", escapeString(title))
                          .replace("{{LEGEND_DATA}}", legendData.toString())
                          .replace("{{X_CATEGORIES}}", xCategoriesStr.toString())
                          .replace("{{X_LABEL}}", escapeString(xLabel))
                          .replace("{{MIN_Y}}", String.valueOf(minY))
                          .replace("{{MAX_Y}}", String.valueOf(maxY))
                          .replace("{{SERIES_DATA}}", seriesData.toString())
;
        } catch (IOException e) {
            throw new RuntimeException("Failed to load chart template", e);
        }
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
        try {
            String template = loadTemplate("/Templates/Plots/heatmap.js");

            // Build X labels
            StringBuilder xLabelsStr = new StringBuilder();
            for (int i = 0; i < xLabels.length; i++) {
                if (i > 0) xLabelsStr.append(",");
                xLabelsStr.append("'").append(escapeString(xLabels[i])).append("'");
            }

            // Build Y labels
            StringBuilder yLabelsStr = new StringBuilder();
            for (int i = 0; i < yLabels.length; i++) {
                if (i > 0) yLabelsStr.append(",");
                yLabelsStr.append("'").append(yLabels[i]).append("'");
            }

            // Build heatmap data
            StringBuilder heatmapData = new StringBuilder();
            boolean first = true;
            for (int y = 0; y < data.length; y++) {
                for (int x = 0; x < data[y].length; x++) {
                    if (!first) heatmapData.append(",");
                    // Transform data like the original: 0 - tileBaseMeans[tile][base]
                    // This makes higher quality scores (good) become lower values (blue)
                    // and lower quality scores (bad) become higher values (red)
                    double transformedValue = 0 - data[y][x];
                    heatmapData.append("[").append(x).append(",").append(y).append(",").append(df.format(transformedValue)).append("]");
                    first = false;
                }
            }

            return template.replace("{{CONTAINER_ID}}", containerId)
                          .replace("{{TITLE}}", escapeString(title))
                          .replace("{{X_LABELS}}", xLabelsStr.toString())
                          .replace("{{Y_LABELS}}", yLabelsStr.toString())
                          .replace("{{HEATMAP_DATA}}", heatmapData.toString())
;
        } catch (IOException e) {
            throw new RuntimeException("Failed to load chart template", e);
        }
    }

    public static String generateBoxPlotWithQualityZonesConfig(String containerId, double[] means, double[] medians,
                                                             double[] lowest, double[] highest, double[] lowerQuartile,
                                                             double[] upperQuartile, double minY, double maxY,
                                                             String[] xLabels, String title) {
        try {
            String template = loadTemplate("/Templates/Plots/boxplot_quality_zones.js");

            // Build X labels
            StringBuilder xLabelsStr = new StringBuilder();
            for (int i = 0; i < xLabels.length; i++) {
                if (i > 0) xLabelsStr.append(",");
                xLabelsStr.append("'").append(escapeString(xLabels[i])).append("'");
            }

            // Build mean data
            StringBuilder meanData = new StringBuilder();
            for (int i = 0; i < means.length; i++) {
                if (i > 0) meanData.append(",");
                meanData.append(df.format(means[i]));
            }

            // Build boxplot data
            StringBuilder boxplotData = new StringBuilder();
            for (int i = 0; i < means.length; i++) {
                if (i > 0) boxplotData.append(",");
                // Box plot data: [min, Q1, median, Q3, max]
                boxplotData.append("[").append(df.format(lowest[i])).append(",").append(df.format(lowerQuartile[i])).append(",")
                          .append(df.format(medians[i])).append(",").append(df.format(upperQuartile[i])).append(",").append(df.format(highest[i])).append("]");
            }

            return template.replace("{{CONTAINER_ID}}", containerId)
                          .replace("{{TITLE}}", escapeString(title))
                          .replace("{{X_LABELS}}", xLabelsStr.toString())
                          .replace("{{MEAN_DATA}}", meanData.toString())
                          .replace("{{BOXPLOT_DATA}}", boxplotData.toString())
;
        } catch (IOException e) {
            throw new RuntimeException("Failed to load chart template", e);
        }
    }

    public static String generateQualityDistributionConfig(String containerId, double[] data, int[] xCategories, double maxY, String title) {
        try {
            String template = loadTemplate("/Templates/Plots/quality_distribution.js");

            // Build X categories
            StringBuilder xCategoriesStr = new StringBuilder();
            for (int i = 0; i < xCategories.length; i++) {
                if (i > 0) xCategoriesStr.append(",");
                xCategoriesStr.append("'").append(xCategories[i]).append("'");
            }

            // Build quality zone data arrays
            StringBuilder poorQualityData = new StringBuilder();
            StringBuilder moderateQualityData = new StringBuilder();
            StringBuilder goodQualityData = new StringBuilder();

            for (int i = 0; i < xCategories.length; i++) {
                if (i > 0) {
                    poorQualityData.append(",");
                    moderateQualityData.append(",");
                    goodQualityData.append(",");
                }

                if (xCategories[i] <= 20) {
                    poorQualityData.append(maxY);
                } else {
                    poorQualityData.append("0");
                }

                if (xCategories[i] > 20 && xCategories[i] <= 28) {
                    moderateQualityData.append(maxY);
                } else {
                    moderateQualityData.append("0");
                }

                if (xCategories[i] > 28) {
                    goodQualityData.append(maxY);
                } else {
                    goodQualityData.append("0");
                }
            }

            // Build actual data
            StringBuilder actualData = new StringBuilder();
            for (int i = 0; i < data.length; i++) {
                if (i > 0) actualData.append(",");
                actualData.append(data[i]);
            }

            return template.replace("{{CONTAINER_ID}}", containerId)
                          .replace("{{TITLE}}", escapeString(title))
                          .replace("{{X_CATEGORIES}}", xCategoriesStr.toString())
                          .replace("{{MAX_Y}}", String.valueOf((int)Math.ceil(maxY)))
                          .replace("{{POOR_QUALITY_DATA}}", poorQualityData.toString())
                          .replace("{{MODERATE_QUALITY_DATA}}", moderateQualityData.toString())
                          .replace("{{GOOD_QUALITY_DATA}}", goodQualityData.toString())
                          .replace("{{ACTUAL_DATA}}", actualData.toString())
;
        } catch (IOException e) {
            throw new RuntimeException("Failed to load chart template", e);
        }
    }

    public static String generateContinuousLineGraphConfig(String containerId, double[][] data, double minY, double maxY,
                                                         String xLabel, String[] seriesNames, int[] xValues, String title) {
        try {
            String template = loadTemplate("/Templates/Plots/continuous_line_graph.js");
            String seriesTemplate = loadTemplate("/Templates/Plots/continuous_line_series.js");

            // Build legend data
            StringBuilder legendData = new StringBuilder();
            for (int i = 0; i < seriesNames.length; i++) {
                if (i > 0) legendData.append(", ");
                legendData.append("'").append(escapeString(seriesNames[i])).append("'");
            }

            // Build series data
            StringBuilder seriesData = new StringBuilder();
            for (int d = 0; d < data.length; d++) {
                if (d > 0) seriesData.append(",\n");

                // Build data array for this series (x, y pairs for continuous axis)
                StringBuilder dataArray = new StringBuilder();
                for (int i = 0; i < data[d].length; i++) {
                    if (i > 0) dataArray.append(", ");
                    dataArray.append("[").append(xValues[i]).append(", ").append(df.format(data[d][i])).append("]");
                }

                String series = seriesTemplate.replace("{{SERIES_NAME}}", escapeString(seriesNames[d]))
                                              .replace("{{SERIES_DATA}}", dataArray.toString())
                                              .replace("{{COLOR}}", COLORS[d % COLORS.length]);
                seriesData.append(series);
            }

            return template.replace("{{CONTAINER_ID}}", containerId)
                          .replace("{{TITLE}}", escapeString(title))
                          .replace("{{LEGEND_DATA}}", legendData.toString())
                          .replace("{{X_LABEL}}", escapeString(xLabel))
                          .replace("{{MIN_Y}}", String.valueOf(minY))
                          .replace("{{MAX_Y}}", String.valueOf(maxY))
                          .replace("{{SERIES_DATA}}", seriesData.toString())
;
        } catch (IOException e) {
            throw new RuntimeException("Failed to load chart template", e);
        }
    }
}
