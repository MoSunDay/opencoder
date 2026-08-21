// api.js - token plumbing, fetch helpers, and shared UI state. Loaded FIRST:
// every later script is a classic script sharing these globals. All network
// calls go through apiGet/apiSend/apiUrl so bearer auth (page URL ?token=...)
// keeps working for fetches AND EventSource (which cannot set headers).
var $ = function (s) { return document.querySelector(s); };

// Shared mutable state (declared once, in the first script).
var cur = null;        // active session id
var busy = false;      // a drain is running (composer button + polling gates)
var mode = 'act';      // active agent: 'act' | 'plan'
// Fns to re-run when the tab becomes visible again (pollers re-register).
var visibleAgain = [];

function esc(s) {
  var d = document.createElement('div');
  d.textContent = s == null ? '' : String(s);
  return d.innerHTML;
}

// -- token mechanism ---------------------------------------------------------
// The bearer token rides the page URL as ?token=ULID (see auth.rs): the HTML
// route itself is served with that query, so it is propagated to every
// subsequent request as both a query param (EventSource-compatible) and an
// Authorization header (fetch).
function pageToken() {
  return new URLSearchParams(location.search).get('token') || '';
}
function authHeaders(extra) {
  var t = pageToken();
  var h = {};
  if (extra) { for (var k in extra) { if (extra.hasOwnProperty(k)) h[k] = extra[k]; } }
  if (t) { h['authorization'] = 'Bearer ' + t; }
  return h;
}
// Append the token as a query param. Handles URLs that already carry a query
// string (the old impl blindly appended '?token=' after '&search=...' and
// produced an unparseable URL).
function withToken(url) {
  var t = pageToken();
  if (!t) { return url; }
  return url + (url.indexOf('?') >= 0 ? '&' : '?') + 'token=' + encodeURIComponent(t);
}
// URL for EventSource endpoints (token must be a query param there).
function apiUrl(path) { return withToken(path); }

// -- fetch helpers -----------------------------------------------------------
// Non-2xx responses throw { status, error } so callers can `catch (e) { alert(e.error) }`.
function apiFail(status, j) {
  return { status: status, error: (j && j.error) || ('HTTP ' + status) };
}

async function apiGet(path) {
  var r, j;
  try {
    r = await fetch(withToken(path), { headers: authHeaders() });
  } catch (err) {
    throw apiFail(0, { error: 'network: ' + err });
  }
  try { j = await r.json(); } catch (_) { j = null; }
  if (!r.ok) { throw apiFail(r.status, j); }
  return j;
}

// apiSend('POST', path, {..}) — body === undefined sends no body (DELETE/no-op POST).
async function apiSend(method, path, body) {
  var opts = { method: method, headers: authHeaders(body === undefined ? null : { 'content-type': 'application/json' }) };
  if (body !== undefined) { opts.body = JSON.stringify(body); }
  var r, j;
  try {
    r = await fetch(withToken(path), opts);
  } catch (err) {
    throw apiFail(0, { error: 'network: ' + err });
  }
  try { j = await r.json(); } catch (_) { j = null; }
  if (!r.ok) { throw apiFail(r.status, j); }
  return j;
}

// Background pollers must surface non-2xx too, but alerting on every 1.5s
// tick would spam: latch per key - alert the FIRST failure, re-arm on success.
var alertLatch = {};
function alertOnce(key, e) {
  if (alertLatch[key]) { return; }
  alertLatch[key] = true;
  alert(e && e.error ? e.error : String(e || 'error'));
}
function alertOk(key) { alertLatch[key] = false; }


// Pause work while the tab is hidden; pollers re-run on return.
document.addEventListener('visibilitychange', function () {
  if (!document.hidden) {
    for (var i = 0; i < visibleAgain.length; i++) { visibleAgain[i](); }
  }
});
