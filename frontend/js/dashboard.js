/**
 * Dashboard application logic for OS-Pulse.
 * Depends on: utils.js, charts.js
 */

let selectedMinutes = 15;
let selectedContainer = '';

/* ---- Data loaders ---- */

async function loadMe() {
  const res = await fetch('/api/me', { credentials: 'include' });
  if (res.status === 401) { window.location.href = '/login'; return; }
  const me = await res.json();
  document.getElementById('welcome').textContent = `Signed in as ${me.username}`;
}

async function loadMetrics() {
  const res = await fetch('/api/metrics', { credentials: 'include' });
  if (res.status === 401) { window.location.href = '/login'; return; }
  const data = await res.json();
  const s = data.system;

  document.getElementById('cpuV').textContent = `${s.cpu_percent.toFixed(1)}%`;
  document.getElementById('cpuS').textContent = `Load ${s.load_1.toFixed(2)} / ${s.load_5.toFixed(2)} / ${s.load_15.toFixed(2)}`;
  document.getElementById('memV').textContent = `${s.memory_percent.toFixed(1)}%`;
  document.getElementById('memS').textContent = `${fmtBytes(s.memory_used_bytes)} / ${fmtBytes(s.memory_total_bytes)}`;
  document.getElementById('diskV').textContent = `${s.disk_percent.toFixed(1)}%`;
  document.getElementById('diskS').textContent = `${fmtBytes(s.disk_used_bytes)} / ${fmtBytes(s.disk_total_bytes)}`;
  document.getElementById('netV').textContent = fmtMbps(s.network_rx_bytes + s.network_tx_bytes);
  document.getElementById('netS').textContent = `RX ${fmtMbps(s.network_rx_bytes)} · TX ${fmtMbps(s.network_tx_bytes)}`;

  const rows = document.getElementById('containerRows');
  const empty = document.getElementById('containerEmpty');
  rows.innerHTML = '';

  if (!data.containers.length) {
    empty.style.display = 'block';
  } else {
    empty.style.display = 'none';
    for (const c of data.containers) {
      const tr = document.createElement('tr');
      tr.innerHTML = `
        <td>${escapeHtml(c.name)}</td>
        <td>${escapeHtml(c.status)}</td>
        <td>${c.cpu_percent.toFixed(1)}%</td>
        <td>${fmtBytes(c.memory_used_bytes)} / ${fmtBytes(c.memory_limit_bytes)}</td>
        <td>RX ${fmtBytes(c.network_rx_bytes)} · TX ${fmtBytes(c.network_tx_bytes)}</td>
        <td>R ${fmtBytes(c.disk_read_bytes)} · W ${fmtBytes(c.disk_write_bytes)}</td>
        <td>${escapeHtml(c.image)}</td>
        <td>${escapeHtml(c.tag)}</td>
        <td>${c.restart_count}</td>
      `;
      rows.appendChild(tr);
    }
  }
}

async function loadTrends() {
  const res = await fetch(`/api/trends?minutes=${selectedMinutes}`, { credentials: 'include' });
  if (res.status === 401) { window.location.href = '/login'; return; }
  const data = await res.json();
  const points = data.points || [];

  const win = data.requested_minutes || selectedMinutes;
  const cpuSeries  = buildWindowSeries(points, (p) => p.cpu_percent, win);
  const memSeries  = buildWindowSeries(points, (p) => p.memory_percent, win);
  const memUsedSeries = buildWindowSeries(points, (p) => p.memory_used_bytes, win);
  const memTotalSeries = buildWindowSeries(points, (p) => p.memory_total_bytes, win);
  const diskSeries = buildWindowSeries(points, (p) => Number(p.disk_iops || 0), win);
  const netRxSeries = buildWindowSeries(points, (p) => (Number(p.network_rx_bytes || 0) * 8) / 1_000_000, win);
  const netTxSeries = buildWindowSeries(points, (p) => (Number(p.network_tx_bytes || 0) * 8) / 1_000_000, win);

  drawLine('cpuTrend',  cpuSeries.values,  '#54c7ff', cpuSeries.labels,  (v) => `${Number(v).toFixed(1)}%`, 0, 100);
  drawLine('memTrend',  memSeries.values,  '#7b61ff', memSeries.labels,  (v, idx) => {
    const pct = `${Number(v).toFixed(1)}%`;
    const used = memUsedSeries.values[idx];
    const total = memTotalSeries.values[idx];
    if (used && total) return `${pct}  (${fmtBytes(used)} / ${fmtBytes(total)})`;
    return pct;
  }, 0, 100);
  drawLine('diskTrend', diskSeries.values, '#30d188', diskSeries.labels, (v) => `${Number(v).toFixed(0)} IOPS`);
  drawMultiLine(
    'netTrend',
    netRxSeries.labels,
    [
      { label: 'RX', values: netRxSeries.values, color: '#54c7ff' },
      { label: 'TX', values: netTxSeries.values, color: '#f5b740' },
    ],
    (v) => `${Number(v).toFixed(2)} Mbps`,
  );

  const sampled = data.sampled ? ' · sampled to 96' : '';
  document.getElementById('historyMeta').textContent =
    `window ${data.requested_minutes}m · available ${data.available_minutes}m · points ${data.returned_points}${sampled}`;
}

function syncContainerSelect(available, selected) {
  const select = document.getElementById('containerSelect');
  if (!available.length) {
    select.innerHTML = '<option value="">No container history yet</option>';
    select.disabled = true;
    return;
  }

  select.disabled = false;
  if (!selectedContainer || !available.includes(selectedContainer)) {
    selectedContainer = selected || available[0];
  }

  select.innerHTML = available
    .map((name) => `<option value="${escapeHtml(name)}">${escapeHtml(name)}</option>`)
    .join('');
  select.value = selectedContainer;
}

async function loadContainerTrends() {
  const queryName = selectedContainer ? `&name=${encodeURIComponent(selectedContainer)}` : '';
  const res = await fetch(`/api/trends/containers?minutes=${selectedMinutes}${queryName}`, { credentials: 'include' });
  if (res.status === 401) { window.location.href = '/login'; return; }
  const data = await res.json();
  syncContainerSelect(data.available || [], data.selected || '');

  const points = data.points || [];
  const cpuSeries = buildWindowSeries(points, (p) => p.cpu_percent, selectedMinutes);
  const memSeries = buildWindowSeries(points, (p) => {
    if (!p.memory_limit_bytes) return null;
    return (p.memory_used_bytes / p.memory_limit_bytes) * 100;
  }, selectedMinutes);
  const ioSeries = buildWindowSeries(points, (p) => p.network_total_bytes + p.disk_io_total_bytes, selectedMinutes);

  drawLine('containerCpuTrend', cpuSeries.values, '#54c7ff', cpuSeries.labels, (v) => `${Number(v).toFixed(1)}%`, 0, 100);
  drawLine('containerMemTrend', memSeries.values, '#7b61ff', memSeries.labels, (v) => `${Number(v).toFixed(1)}%`, 0, 100);
  drawLine('containerIoTrend',  ioSeries.values,  '#f5b740', ioSeries.labels,  (v) => `${fmtBytes(Number(v))}`);
}

/* ---- UI bindings ---- */

function bindRangeButtons() {
  const group = document.getElementById('rangeGroup');
  group.addEventListener('click', async (e) => {
    const btn = e.target.closest('.range-btn');
    if (!btn) return;
    selectedMinutes = Number(btn.dataset.min || 60);
    for (const b of group.querySelectorAll('.range-btn')) b.classList.remove('active');
    btn.classList.add('active');
    await Promise.all([loadTrends(), loadContainerTrends()]);
  });
}

function bindContainerSelect() {
  const select = document.getElementById('containerSelect');
  select.addEventListener('change', async () => {
    selectedContainer = select.value;
    await loadContainerTrends();
  });
}

document.getElementById('logoutBtn').addEventListener('click', async () => {
  await fetch('/api/auth/logout', { method: 'POST', credentials: 'include' });
  window.location.href = '/login';
});

/* ---- Main loop ---- */

async function pulse() {
  await Promise.all([loadMetrics(), loadTrends(), loadContainerTrends()]);
}

async function scheduleNextPulse() {
  await pulse();
  setTimeout(scheduleNextPulse, 3000);
}

bindRangeButtons();
bindContainerSelect();
loadMe().then(() => scheduleNextPulse());
