var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));
var option_{{CONTAINER_ID}} = {
  title: { text: '{{TITLE}}', left: 'center', top: 10 },
  tooltip: { trigger: 'axis' },
  legend: { data: [{{LEGEND_DATA}}], bottom: 10 },
  grid: { left: 60, right: 30, top: 60, bottom: 90 },
  xAxis: {
    type: 'category',
    data: [{{X_CATEGORIES}}],
    name: '{{X_LABEL}}',
    nameLocation: 'middle',
    nameGap: 30
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
chart_{{CONTAINER_ID}}.setOption(option_{{CONTAINER_ID}});
window.addEventListener('resize', function() {
  chart_{{CONTAINER_ID}}.resize();
});
