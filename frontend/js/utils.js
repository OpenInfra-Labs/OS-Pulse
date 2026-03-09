/**
 * Shared utility functions for the OS-Pulse frontend.
 */

const FIXED_TREND_POINTS = 96;

function escapeHtml(s) {
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function fmtBytes(n) {
  if (n === null || n === undefined) return '--';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  let i = 0;
  let v = Number(n);
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i++; }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

function fmtMbps(bytesPerSec) {
  const mbps = (Number(bytesPerSec || 0) * 8) / 1_000_000;
  return `${mbps.toFixed(mbps >= 100 ? 0 : 2)} Mbps`;
}

function fmtTime(ts) {
  if (!ts) return '--:--:--';
  return new Date(ts * 1000).toLocaleTimeString('en-GB', { hour12: false });
}

function buildWindowSeries(points, valueMapper) {
  const labels = new Array(FIXED_TREND_POINTS).fill('');
  const values = new Array(FIXED_TREND_POINTS).fill(null);
  const source = (points || []).slice(0, FIXED_TREND_POINTS);

  for (let idx = 0; idx < source.length; idx++) {
    labels[idx] = fmtTime(source[idx].ts);
    values[idx] = valueMapper(source[idx]);
  }

  return { labels, values };
}
