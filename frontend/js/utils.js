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

function buildWindowSeries(points, valueMapper, windowMinutes) {
  const labels = new Array(FIXED_TREND_POINTS).fill('');
  const values = new Array(FIXED_TREND_POINTS).fill(null);
  const source = points || [];
  if (!source.length || !windowMinutes) return { labels, values };

  const nowTs = Math.floor(Date.now() / 1000);
  const windowStart = nowTs - windowMinutes * 60;
  const slotSec = (windowMinutes * 60) / FIXED_TREND_POINTS;

  // Pre-fill time labels for each slot
  for (let i = 0; i < FIXED_TREND_POINTS; i++) {
    labels[i] = fmtTime(Math.round(windowStart + (i + 0.5) * slotSec));
  }

  // Map each data point into the correct time-based slot
  const counts = new Array(FIXED_TREND_POINTS).fill(0);
  for (const pt of source) {
    const ts = pt.ts;
    if (ts < windowStart || ts > nowTs) continue;
    let idx = Math.floor((ts - windowStart) / slotSec);
    if (idx >= FIXED_TREND_POINTS) idx = FIXED_TREND_POINTS - 1;

    const val = valueMapper(pt);
    if (val === null || val === undefined) continue;

    if (values[idx] === null) {
      values[idx] = val;
      counts[idx] = 1;
    } else {
      // Multiple points in same slot → running average
      values[idx] = (values[idx] * counts[idx] + val) / (counts[idx] + 1);
      counts[idx]++;
    }
  }

  return { labels, values };
}
