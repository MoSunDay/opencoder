// format.js — small pure display helpers shared by the tabs.

import dayjs from 'dayjs';

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
