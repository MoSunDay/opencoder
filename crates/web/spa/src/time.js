// time.js — clock-skew compensation against the unsigned /api/time endpoint.
// The server rejects signatures whose ts deviates more than REPLAY_WINDOW_MS
// (300s) from its clock, so the offset must be refreshed at login and after
// any 401 (see api.js).

import { urlFor } from './store.js';

let offsetMs = 0;
let lastSyncAt = 0;

export function clockOffsetMs() {
  return offsetMs;
}

export function lastTimeSyncAt() {
  return lastSyncAt;
}

/// GET /api/time is signature-exempt (auth_sig_mw.rs `exempt`), so a plain
/// fetch is enough. offset = server clock - browser clock.
export async function syncTime() {
  const resp = await fetch(urlFor('/api/time'), { method: 'GET' });
  if (!resp.ok) {
    throw new Error('time sync failed: HTTP ' + resp.status);
  }
  const j = await resp.json();
  if (typeof j.server_time_ms === 'number') {
    offsetMs = j.server_time_ms - Date.now();
    lastSyncAt = Date.now();
  }
  return offsetMs;
}

/// Refresh only when the cached offset could plausibly have gone stale
/// (half the replay window). Cheap guard for long-lived tabs.
export async function ensureTimeSynced(maxAgeMs = 150000) {
  if (!lastSyncAt || Date.now() - lastSyncAt > maxAgeMs) {
    await syncTime();
  }
}
