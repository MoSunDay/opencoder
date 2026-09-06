// timeText.jsx — the shared relative-time cell: dayjs fromNow() up front with
// the absolute timestamp (YYYY-MM-DD HH:mm:ss) one hover away. Replaces the
// per-panel `new Date(v).toLocaleString()` cells and the older English
// relTime/absTime pairs. dayjs relativeTime renders in the active locale
// (main.jsx sets zh-cn → 「3 分钟前」). Pure helpers + one function component.

import { Tooltip } from 'antd';
import dayjs from 'dayjs';
import relativeTime from 'dayjs/plugin/relativeTime';

dayjs.extend(relativeTime);

/// parseTs(ts) → dayjs | null. Accepts epoch-ms numbers, numeric strings and
/// ISO strings; anything unparsable yields null so callers render '-'.
export function parseTs(ts) {
  if (ts === undefined || ts === null || ts === '') {
    return null;
  }
  const direct = dayjs(ts);
  if (direct.isValid()) {
    return direct;
  }
  const numeric = Number(ts);
  const parsed = Number.isFinite(numeric) ? dayjs(numeric) : null;
  return parsed && parsed.isValid() ? parsed : null;
}

/// fromNowText(ts) → localized relative text ('3 分钟前'), '-' when unparsable.
export function fromNowText(ts) {
  const t = parseTs(ts);
  return t ? t.fromNow() : '-';
}

/// absTimeText(ts) → 'YYYY-MM-DD HH:mm:ss' for the Tooltip, '-' otherwise.
export function absTimeText(ts) {
  const t = parseTs(ts);
  return t ? t.format('YYYY-MM-DD HH:mm:ss') : '-';
}

/// TimeText — relative time now, absolute time on hover.
export function TimeText({ ts }) {
  return (
    <Tooltip title={absTimeText(ts)}>
      <span>{fromNowText(ts)}</span>
    </Tooltip>
  );
}
