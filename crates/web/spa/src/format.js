// format.js — small pure display helpers shared by the tabs.

import dayjs from 'dayjs';

/// Compact relative heartbeat, e.g. "3s ago" / "2m ago" (dayjs-based diff).
export function relTime(ts) {
  if (ts === undefined || ts === null || ts === '') {
    return '-';
  }
  const t = dayjs(Number(ts));
  if (!t.isValid()) {
    return '-';
  }
  const s = Math.max(0, Math.round(dayjs().diff(t, 'second', true)));
  if (s < 60) {
    return s + 's ago';
  }
  const m = Math.floor(s / 60);
  if (m < 60) {
    return m + 'm ago';
  }
  const h = Math.floor(m / 60);
  if (h < 24) {
    return h + 'h ago';
  }
  return Math.floor(h / 24) + 'd ago';
}

/// Absolute timestamp for Tooltips.
export function absTime(ts) {
  const t = dayjs(Number(ts));
  return t.isValid() ? t.format('YYYY-MM-DD HH:mm:ss') : '-';
}

/// Duration for tool headers: "1.2s" / "850ms" / ''.
export function fmtDuration(ms) {
  if (typeof ms !== 'number' || !Number.isFinite(ms) || ms < 0) {
    return '';
  }
  return ms < 1000 ? Math.round(ms) + 'ms' : (ms / 1000).toFixed(1) + 's';
}

/// Short dialog label: title wins, else id head.
export function dialogLabel(d) {
  const title = (d && d.title) || '';
  const id = (d && (d.session_id || d.id)) || '';
  return title || (id ? id.slice(0, 12) + '…' : '(untitled)');
}
