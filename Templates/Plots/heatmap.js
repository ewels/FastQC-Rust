var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));
var option_{{CONTAINER_ID}} = {
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
chart_{{CONTAINER_ID}}.setOption(option_{{CONTAINER_ID}});
window.addEventListener('resize', function() {
  chart_{{CONTAINER_ID}}.resize();
});
