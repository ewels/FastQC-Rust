var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));
var option_{{CONTAINER_ID}} = {
  title: { text: '{{TITLE}}', left: 'center', top: 10 },
  tooltip: { trigger: 'axis', formatter: function(params) { return params[0].name + '<br/>Count: ' + params[0].value; } },
  grid: { left: 80, right: 80, top: 80, bottom: 80 },
  xAxis: {
    type: 'category',
    name: 'Mean Sequence Quality (Phred Score)',
    nameLocation: 'middle',
    nameGap: 30,
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
      areaStyle: { color: 'rgba(255, 0, 0, 0.2)' },
      lineStyle: { width: 0 },
      symbol: 'none',
      data: [{{POOR_QUALITY_DATA}}]
    },
    {
      name: 'Moderate Quality',
      type: 'line',
      stack: 'zones',
      areaStyle: { color: 'rgba(255, 255, 0, 0.2)' },
      lineStyle: { width: 0 },
      symbol: 'none',
      data: [{{MODERATE_QUALITY_DATA}}]
    },
    {
      name: 'Good Quality',
      type: 'line',
      stack: 'zones',
      areaStyle: { color: 'rgba(0, 255, 0, 0.2)' },
      lineStyle: { width: 0 },
      symbol: 'none',
      data: [{{GOOD_QUALITY_DATA}}]
    },
    {
      name: 'Average Quality per read',
      type: 'line',
      lineStyle: { color: '#0000FF', width: 2 },
      itemStyle: { color: '#0000FF' },
      data: [{{ACTUAL_DATA}}]
    }
  ]
};
chart_{{CONTAINER_ID}}.setOption(option_{{CONTAINER_ID}});
window.addEventListener('resize', function() {
  chart_{{CONTAINER_ID}}.resize();
});
