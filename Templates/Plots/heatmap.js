//<![CDATA[
// Initialize chart
var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));

// Base chart configuration (theme-neutral)
var baseOption_{{CONTAINER_ID}} = {
  title: { text: '{{TITLE}}', left: 'center', top: 10 },
  tooltip: { position: 'top' },
  grid: { left: 80, right: 80, top: 80, bottom: 80 },
  xAxis: {
    type: 'category',
    data: [{{X_LABELS}}],
    splitArea: { show: true }
  },
  yAxis: {
    type: 'category',
    data: [{{Y_LABELS}}],
    splitArea: { show: true }
  },
  visualMap: {
    min: 0, max: 10,
    calculable: true,
    orient: 'horizontal',
    left: 'center',
    bottom: 20,
    inRange: {
      color: ['#0000C8', '#0080C8', '#00C8C8', '#00C800', '#C8C800', '#C88000', '#C80000']
    }
  },
  series: [{
    name: 'Quality',
    type: 'heatmap',
    data: [{{HEATMAP_DATA}}],
    emphasis: {
      itemStyle: {
        shadowBlur: 10,
        shadowColor: 'rgba(0, 0, 0, 0.5)'
      }
    }
  }]
};

// Theme application function
window.applyThemeToChart_{{CONTAINER_ID}} = function(baseOption, colors) {
  var option = JSON.parse(JSON.stringify(baseOption));

  // Apply theme colors
  option.title.textStyle = { color: colors.text };
  option.backgroundColor = colors.background;

  // Axis styling
  option.xAxis.axisLabel = { color: colors.text };
  option.xAxis.splitArea.areaStyle = { color: [colors.background, colors.grid] };

  option.yAxis.axisLabel = { color: colors.text };
  option.yAxis.splitArea.areaStyle = { color: [colors.background, colors.grid] };

  // Visual map styling
  option.visualMap.textStyle = { color: colors.text };

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
