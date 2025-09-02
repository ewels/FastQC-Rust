//<![CDATA[
// Initialize chart
var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));

// Base chart configuration (theme-neutral)
var baseOption_{{CONTAINER_ID}} = {
  title: { text: '{{TITLE}}', left: 'center', top: 5 },
  tooltip: { trigger: 'axis', formatter: function(params) { return params[0].name + '<br/>Count: ' + params[0].value; } },
  grid: { left: 50, right: 20, top: 35, bottom: 55 },
  xAxis: {
    type: 'category',
    name: 'Mean Sequence Quality (Phred Score)',
    nameLocation: 'middle',
    nameGap: 20,
    data: [{{X_CATEGORIES}}]
  },
  yAxis: {
    type: 'value',
    name: 'Count',
    nameLocation: 'middle',
    nameGap: 50,
    max: {{MAX_Y}}
  },
  series: [
    {
      name: 'Poor Quality',
      type: 'line',
      stack: 'zones',
      areaStyle: {},
      lineStyle: { width: 0 },
      symbol: 'none',
      data: [{{POOR_QUALITY_DATA}}]
    },
    {
      name: 'Moderate Quality',
      type: 'line',
      stack: 'zones',
      areaStyle: {},
      lineStyle: { width: 0 },
      symbol: 'none',
      data: [{{MODERATE_QUALITY_DATA}}]
    },
    {
      name: 'Good Quality',
      type: 'line',
      stack: 'zones',
      areaStyle: {},
      lineStyle: { width: 0 },
      symbol: 'none',
      data: [{{GOOD_QUALITY_DATA}}]
    },
    {
      name: 'Average Quality per read',
      type: 'line',
      lineStyle: { width: 2 },
      data: [{{ACTUAL_DATA}}]
    }
  ]
};

// Theme application function
window.applyThemeToChart_{{CONTAINER_ID}} = function(baseOption, colors) {
  var option = JSON.parse(JSON.stringify(baseOption));

  // Apply theme colors
  option.title.textStyle = { color: colors.text };
  option.backgroundColor = colors.background;

  // Axis styling
  option.xAxis.axisLabel = { color: colors.text };
  option.xAxis.axisLine = { lineStyle: { color: colors.axis } };
  option.xAxis.axisTick = { lineStyle: { color: colors.axis } };
  option.xAxis.nameTextStyle = { color: colors.text };

  option.yAxis.axisLabel = { color: colors.text };
  option.yAxis.axisLine = { lineStyle: { color: colors.axis } };
  option.yAxis.axisTick = { lineStyle: { color: colors.axis } };
  option.yAxis.nameTextStyle = { color: colors.text };
  option.yAxis.splitLine = { lineStyle: { color: colors.grid } };

  // Quality zone area colors with transparency
  option.series[0].areaStyle.color = colors.qualityBad + '33'; // 20% opacity
  option.series[1].areaStyle.color = colors.qualityWarning + '33'; // 20% opacity
  option.series[2].areaStyle.color = colors.qualityGood + '33'; // 20% opacity

  // Average quality line styling
  option.series[3].lineStyle.color = colors.primary;
  option.series[3].itemStyle = { color: colors.primary };

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
