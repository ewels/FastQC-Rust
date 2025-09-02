//<![CDATA[
// Initialize chart
var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));

// Base chart configuration (theme-neutral)
var baseOption_{{CONTAINER_ID}} = {
  title: { text: '{{TITLE}}', left: 'center', top: 5 },
  tooltip: { trigger: 'axis' },
  grid: { left: 50, right: 20, top: 35, bottom: 50 },
  xAxis: {
    type: 'category',
    name: 'Position in read (bp)',
    nameLocation: 'middle',
    nameGap: 20,
    data: [{{X_LABELS}}]
  },
  yAxis: {
    type: 'value',
    name: 'Quality Score',
    nameLocation: 'middle',
    nameGap: 50,
    min: 0,
    max: 40
  },
  series: [
    {
      name: 'Quality Zones',
      type: 'line',
      data: [],
      showInLegend: false,
      markArea: {
        silent: true,
        itemStyle: { opacity: 0.8 },
        label: { show: false },
        data: [
          [{ yAxis: 0 }, { yAxis: 20 }],
          [{ yAxis: 20 }, { yAxis: 28 }],
          [{ yAxis: 28 }, { yAxis: 40 }]
        ]
      }
    },
    {
      name: 'Mean',
      type: 'line',
      lineStyle: { width: 2 },
      symbol: 'none',
      data: [{{MEAN_DATA}}]
    },
    {
      name: 'Quality Scores',
      type: 'boxplot',
      boxWidth: ['7', '99%'],
      itemStyle: {
        borderWidth: 1
      },
      emphasis: {
        itemStyle: {
          borderWidth: 2
        }
      },
      medianStyle: {
        width: 2
      },
      data: [{{BOXPLOT_DATA}}]
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

  // Quality zone colors
  option.series[0].markArea.data[0][0].itemStyle = { color: colors.qualityBad, opacity: 0.8 };
  option.series[0].markArea.data[1][0].itemStyle = { color: colors.qualityWarning, opacity: 0.8 };
  option.series[0].markArea.data[2][0].itemStyle = { color: colors.qualityGood, opacity: 0.8 };

  // Mean line styling
  option.series[1].lineStyle.color = colors.primary;
  option.series[1].itemStyle = { color: colors.primary };

  // Boxplot styling
  option.series[2].itemStyle.color = colors.boxplot;
  option.series[2].itemStyle.borderColor = colors.boxplotBorder;
  option.series[2].emphasis.itemStyle.color = colors.boxplot;
  option.series[2].emphasis.itemStyle.borderColor = colors.boxplotBorder;
  option.series[2].medianStyle.color = colors.accent;

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
