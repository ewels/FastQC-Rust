var chart_{{CONTAINER_ID}} = echarts.init(document.getElementById('{{CONTAINER_ID}}'));
var option_{{CONTAINER_ID}} = {
  title: { text: '{{TITLE}}', left: 'center', top: 10 },
  tooltip: { trigger: 'axis' },
  grid: { left: 80, right: 80, top: 80, bottom: 80 },
  xAxis: {
    type: 'category',
    name: 'Position in read (bp)',
    nameLocation: 'middle',
    nameGap: 30,
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
          [{ yAxis: 0, itemStyle: { color: '#f0c6bc' } }, { yAxis: 20 }],
          [{ yAxis: 20, itemStyle: { color: '#e8d28f' } }, { yAxis: 28 }],
          [{ yAxis: 28, itemStyle: { color: '#afe0b7' } }, { yAxis: 40 }]
        ]
      }
    },
    {
      name: 'Mean',
      type: 'line',
      lineStyle: { color: '#0066CC', width: 2 },
      itemStyle: { color: '#0066CC' },
      symbol: 'none',
      data: [{{MEAN_DATA}}]
    },
    {
      name: 'Quality Scores',
      type: 'boxplot',
      boxWidth: ['7', '99%'],
      itemStyle: {
        color: '#FFFF00',
        borderColor: '#000000',
        borderWidth: 1
      },
      emphasis: {
        itemStyle: {
          color: '#FFFF00',
          borderColor: '#000000',
          borderWidth: 2
        }
      },
      medianStyle: {
        color: '#FF0000',
        width: 2
      },
      data: [{{BOXPLOT_DATA}}]
    }
  ]
};
chart_{{CONTAINER_ID}}.setOption(option_{{CONTAINER_ID}});
window.addEventListener('resize', function() {
  chart_{{CONTAINER_ID}}.resize();
});
