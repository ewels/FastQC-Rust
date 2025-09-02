//<![CDATA[
// Initialize chart
var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));

// Base chart configuration (theme-neutral)
var baseOption_{{CONTAINER_ID}} = {
  title: { text: '{{TITLE}}', left: 'center', top: 10 },
  tooltip: { trigger: 'item' },
  grid: { left: 60, right: 30, top: 60, bottom: 80 },
  xAxis: {
    type: 'category',
    data: [{{X_LABELS}}],
    name: 'Position in read (bp)',
    nameLocation: 'middle',
    nameGap: 30
  },
  yAxis: {
    type: 'value',
    min: {{MIN_Y}},
    max: {{MAX_Y}},
    axisLine: { show: true },
    axisTick: { show: true }
  },
  visualMap: {
    show: false,
    pieces: [
      { gte: 28 },
      { gte: 20, lt: 28 },
      { lt: 20 }
    ]
  },
  series: [
    {
      type: 'boxplot',
      data: [{{BOXPLOT_DATA}}],
      tooltip: {
        formatter: function(params) {
          return params.name + '<br/>' +
            'Upper: ' + params.data[4] + '<br/>' +
            'Q3: ' + params.data[3] + '<br/>' +
            'Median: ' + params.data[2] + '<br/>' +
            'Q1: ' + params.data[1] + '<br/>' +
            'Lower: ' + params.data[0];
        }
      }
    },
    {
      type: 'line',
      name: 'Mean',
      data: [{{MEAN_DATA}}],
      lineStyle: { width: 2 },
      symbol: 'none'
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
  option.yAxis.splitLine = { lineStyle: { color: colors.grid } };

  // Visual map colors
  option.visualMap.pieces[0].color = colors.qualityGood;
  option.visualMap.pieces[1].color = colors.qualityWarning;
  option.visualMap.pieces[2].color = colors.qualityBad;

  // Series styling
  option.series[0].itemStyle = { color: colors.boxplot, borderColor: colors.boxplotBorder };
  option.series[1].lineStyle.color = colors.primary;

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
