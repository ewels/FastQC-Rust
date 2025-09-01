var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));
var option_{{CONTAINER_ID}} = {
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
      { gte: 28, color: 'rgba(195,230,195,0.3)' },
      { gte: 20, lt: 28, color: 'rgba(230,220,195,0.3)' },
      { lt: 20, color: 'rgba(230,195,195,0.3)' }
    ]
  },
  series: [
    {
      type: 'boxplot',
      data: [{{BOXPLOT_DATA}}],
      itemStyle: { color: 'rgba(240,240,0,0.8)', borderColor: '#000' },
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
      lineStyle: { color: '#0000C8', width: 2 },
      symbol: 'none'
    }
  ]
};
chart_{{CONTAINER_ID}}.setOption(option_{{CONTAINER_ID}});
window.addEventListener('resize', function() {
  chart_{{CONTAINER_ID}}.resize();
});
