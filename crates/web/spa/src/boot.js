// boot.js — one-shot startup bootstrap for link login (免弹窗登录).
//
// Runs BEFORE React renders (called at main.jsx module top, ahead of
// createRoot): adopt ?token= / #token= (+base) and scrub the URL in the
// same synchronous tick. Boot-time placement is a correctness requirement,
// not style — a stale stored token must never fire an authenticated request
// before the URL token is adopted, because that request's 401 handler
// (api.js clearToken) would wipe the fresh credential the link just
// delivered (race caught by scripts/acceptance/link_login.js step 3).

import { getState, setCredentials } from './store.js';
import { urlCredential } from './urlCredential.js';

/// Idempotent per URL state: reads window.location, adopts the credentials,
/// scrubs them from the address bar. Returns the adopted token ('' = none).
/// Exported for tests; production callers never need the return value.
export function bootUrlCredential() {
  const { captured, token, base, clean } = urlCredential(window.location.href);
  if (!captured) {
    return '';
  }
  window.history.replaceState(null, '', clean);
  if (token || base) {
    // Captured ⇒ adopted, symmetrically: a token-only link keeps the stored
    // base; a base-only link re-points the console while keeping the session
    // token (fixed-token fleet: same shared token, different host).
    const prev = getState();
    setCredentials(token || prev.token || '', base || prev.base || '');
  }
  return token;
}
