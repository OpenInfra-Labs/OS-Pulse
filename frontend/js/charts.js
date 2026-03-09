/**
 * Chart.js helpers for OS-Pulse trend charts.
 */

const chartStore = {};

const verticalGuidePlugin = {
  id: 'verticalGuide',
  afterDatasetsDraw(chart) {
    const active = chart.tooltip?.getActiveElements?.() || [];
    if (!active.length) return;
    const { ctx, chartArea } = chart;
    const x = active[0].element.x;
    ctx.save();
    ctx.beginPath();
    ctx.moveTo(x, chartArea.top);
    ctx.lineTo(x, chartArea.bottom);
    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgba(120,147,198,0.45)';
    ctx.stroke();
    ctx.restore();
  },
};

const CHART_SCALE_DEFAULTS = {
  x: {
    grid: { color: 'rgba(120,147,198,0.14)' },
    ticks: { display: false },
    border: { color: 'rgba(120,147,198,0.25)' },
  },
  y: {
    grid: { color: 'rgba(120,147,198,0.14)' },
    ticks: { color: '#93a4c4', maxTicksLimit: 4 },
    border: { color: 'rgba(120,147,198,0.25)' },
  },
};

function ensureLineChart(canvasId, color) {
  if (chartStore[canvasId]) return chartStore[canvasId];
  const canvas = document.getElementById(canvasId);
  const chart = new Chart(canvas, {
    type: 'line',
    plugins: [verticalGuidePlugin],
    data: {
      labels: [],
      datasets: [{
        data: [],
        borderColor: color,
        backgroundColor: color,
        pointRadius: 0,
        pointHoverRadius: 3,
        borderWidth: 2,
        tension: 0.25,
        spanGaps: false,
      }],
    },
    options: {
      responsive: true,
      maintainAspectRatio: false,
      animation: false,
      normalized: true,
      interaction: { mode: 'index', axis: 'x', intersect: false },
      plugins: {
        legend: { display: false },
        tooltip: { enabled: true, mode: 'index', intersect: false },
      },
      scales: structuredClone(CHART_SCALE_DEFAULTS),
    },
  });
  chartStore[canvasId] = chart;
  return chart;
}

function drawLine(canvasId, values, color, labels, valueFormatter, yMin = null, yMax = null) {
  const chart = ensureLineChart(canvasId, color);
  chart.data.labels = labels;
  chart.data.datasets[0].data = values;
  chart.options.scales.y.min = yMin;
  chart.options.scales.y.max = yMax;
  chart.options.plugins.tooltip.callbacks = {
    title: (items) => labels[items?.[0]?.dataIndex ?? 0] || '--:--:--',
    label: (ctx) => valueFormatter ? valueFormatter(ctx.raw) : `${ctx.raw}`,
  };
  chart.update('none');
}

function drawMultiLine(canvasId, labels, datasets, valueFormatter, yMin = null, yMax = null) {
  let chart = chartStore[canvasId];
  if (!chart) {
    const canvas = document.getElementById(canvasId);
    chart = new Chart(canvas, {
      type: 'line',
      plugins: [verticalGuidePlugin],
      data: { labels: [], datasets: [] },
      options: {
        responsive: true,
        maintainAspectRatio: false,
        animation: false,
        normalized: true,
        interaction: { mode: 'index', axis: 'x', intersect: false },
        plugins: {
          legend: { display: true, labels: { color: '#93a4c4', boxWidth: 10 } },
          tooltip: { enabled: true, mode: 'index', intersect: false },
        },
        scales: structuredClone(CHART_SCALE_DEFAULTS),
      },
    });
    chartStore[canvasId] = chart;
  }

  chart.data.labels = labels;
  chart.data.datasets = datasets.map((d) => ({
    label: d.label,
    data: d.values,
    borderColor: d.color,
    backgroundColor: d.color,
    pointRadius: 0,
    pointHoverRadius: 3,
    borderWidth: 2,
    tension: 0.25,
    spanGaps: false,
  }));
  chart.options.scales.y.min = yMin;
  chart.options.scales.y.max = yMax;
  chart.options.plugins.tooltip.callbacks = {
    title: (items) => labels[items?.[0]?.dataIndex ?? 0] || '--:--:--',
    label: (ctx) => `${ctx.dataset.label}: ${valueFormatter ? valueFormatter(ctx.raw) : ctx.raw}`,
  };
  chart.update('none');
}
