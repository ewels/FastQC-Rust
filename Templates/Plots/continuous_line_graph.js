//<![CDATA[
// Initialize chart
var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));

// Base chart configuration (theme-neutral)
var baseOption_{{CONTAINER_ID}} = {
  title: { text: '{{TITLE}}', left: 'center', top: 10 },
  tooltip: { trigger: 'axis' },
  legend: { data: [{{LEGEND_DATA}}], bottom: 10 },
  grid: { left: 60, right: 30, top: 60, bottom: 90 },
  xAxis: {
    type: 'value',
    name: '{{X_LABEL}}',
    nameLocation: 'middle',
    nameGap: 30,
    min: 0,
    max: 100
  },
  yAxis: {
    type: 'value',
    min: {{MIN_Y}},
    max: {{MAX_Y}}
  },
  series: [
{{SERIES_DATA}}
  ]
};

// Theme application function
window.applyThemeToChart_{{CONTAINER_ID}} = function(baseOption, colors) {
  var option = JSON.parse(JSON.stringify(baseOption));

  // Apply theme colors
  option.title.textStyle = { color: colors.text };
  option.backgroundColor = colors.background;

  // Legend styling
  option.legend.textStyle = { color: colors.text };

  // Axis styling
  option.xAxis.axisLabel = { color: colors.text };
  option.xAxis.axisLine = { lineStyle: { color: colors.axis } };
  option.xAxis.axisTick = { lineStyle: { color: colors.axis } };
  option.xAxis.nameTextStyle = { color: colors.text };

  option.yAxis.axisLabel = { color: colors.text };
  option.yAxis.axisLine = { lineStyle: { color: colors.axis } };
  option.yAxis.axisTick = { lineStyle: { color: colors.axis } };
  option.yAxis.splitLine = { lineStyle: { color: colors.grid } };

  return option;
};

// Store chart references
window.fastqc_charts = window.fastqc_charts || {};
window.fastqc_chart_options = window.fastqc_chart_options || {};
window.fastqc_charts['{{CONTAINER_ID}}'] = chart_{{CONTAINER_ID}};
window.fastqc_chart_options['{{CONTAINER_ID}}'] = baseOption_{{CONTAINER_ID}};

// Apply initial theme and set options
var currentTheme = document.documentElement.getAttribute('data-theme') || 'light';
var colors = window.getThemeColors ? window.getThemeColors(currentTheme) : {};
var themedOption = window.applyThemeToChart_{{CONTAINER_ID}}(baseOption_{{CONTAINER_ID}}, colors);
chart_{{CONTAINER_ID}}.setOption(themedOption);

// Resize handler
window.addEventListener('resize', function() {
  chart_{{CONTAINER_ID}}.resize();
});
//]]>
