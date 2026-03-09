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
  const source = points || [];
  if (!source.length || !windowMinutes) {
    return {
      labels: new Array(FIXED_TREND_POINTS).fill(''),
      values: new Array(FIXED_TREND_POINTS).fill(null),
    };
  }

  // Expected spacing between consecutive data points (seconds).
  // If actual gap > 2× this, the program was likely not running — insert a
  // null break so Chart.js visually splits the line.
  const expectedSpacingSec = (windowMinutes * 60) / FIXED_TREND_POINTS;
  const gapThreshold = expectedSpacingSec * 2;

  const labels = [];
  const values = [];

  for (let i = 0; i < source.length; i++) {
    const pt = source[i];

    // Detect genuine time gaps between consecutive points
    if (i > 0) {
      const gap = pt.ts - source[i - 1].ts;
      if (gap > gapThreshold) {
        // Insert a single null to break the line at this gap
        labels.push('');
        values.push(null);
      }
    }

    labels.push(fmtTime(pt.ts));
    const val = valueMapper(pt);
    values.push(val === undefined ? null : val);
  }

  return { labels, values };
}
