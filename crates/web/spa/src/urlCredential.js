// urlCredential.js — URL-carried credential capture (login without the modal).
//
// Product contract (LAN convenience): a link like
//   http://host:port/?token=SECRET   (query form)
//   http://host:port/#token=SECRET   (fragment form — fragments never reach
//                                     the server, so no access-log leak;
//                                     preferred for sharing)
// logs the visitor in automatically; `base` rides the same two channels for
// SPAs hosted off-origin. Params are adopted once and immediately scrubbed
// from the address bar via history.replaceState — the secret must not linger
// in a copy/paste-able URL. Pure string/URL work only: the unit suite covers
// every case without jsdom.

const CREDENTIAL_KEYS = ['token', 'base'];

/// `'#token=a&x=1'` → { token: 'a', x: '1' }-style pairs; `'#fleet'` (anchor)
/// has no '=' and is left untouched by urlCredential.
function parseHashParams(hash) {
  if (!hash || !hash.startsWith('#')) {
    return null;
  }
  const body = hash.slice(1);
  return body.includes('=') ? new URLSearchParams(body) : null;
}

function takePair(params) {
  if (!params) {
    return { token: '', base: '' };
  }
  const token = String(params.get('token') || '').trim();
  const base = String(params.get('base') || '').trim();
  CREDENTIAL_KEYS.forEach((key) => params.delete(key));
  return { token, base };
}

/// urlCredential(href) → { captured, token, base, clean }.
/// `clean` is a same-origin relative URL (pathname + search + hash) with the
/// credential params removed from whichever channel carried them; unrelated
/// query params and anchor-style hashes survive verbatim. `captured` is false
/// (and `clean === null`) when the URL carries no credential, so callers skip
/// the replaceState no-op.
export function urlCredential(href) {
  const url = new URL(href);
  // One params object per channel: takePair reads then scrubs in place, so
  // the rebuilt URL reuses the same (already cleaned) instance.
  const searchParams = new URLSearchParams(url.searchParams);
  const hashParams = parseHashParams(url.hash);
  const fromQuery = takePair(searchParams);
  const fromHash = takePair(hashParams);
  const token = fromQuery.token || fromHash.token;
  const base = fromQuery.base || fromHash.base;
  const captured = Boolean(token || base);
  if (!captured) {
    return { captured: false, token: '', base: '', clean: null };
  }
  const query = searchParams.toString();
  const rebuiltHash = hashParams ? hashParams.toString() : url.hash.slice(1);
  const hash = rebuiltHash ? '#' + rebuiltHash : '';
  return {
    captured: true,
    token,
    base,
    clean: url.pathname + (query ? '?' + query : '') + hash,
  };
}
