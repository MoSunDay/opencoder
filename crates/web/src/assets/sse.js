// sse.js - EventSource lifecycle: open, dispatch, reconnect with backoff.
// Other scripts register handlers via onSSE('kind', fn); sse.js stays
// ignorant of what chat.js / queue_panel.js / questions.js actually do.
// kind -> [fn(data, rawEvent)]; data is the parsed JSON payload.
var SSE_HANDLERS = {};
function onSSE(kind, fn) {
  if (!SSE_HANDLERS[kind]) { SSE_HANDLERS[kind] = []; }
  SSE_HANDLERS[kind].push(fn);
}

var es = null;              // the live EventSource
var sseAttempts = 0;        // consecutive reconnect failures
var sseTimer = null;        // pending reconnect timeout
var SSE_MAX_ATTEMPTS = 5;   // then the persistent "reload" banner shows

function sseEventsUrl(after) {
  var u = '/api/sessions/' + cur + '/events';
  if (after) { u += '?after=' + after; }
  return apiUrl(u);
}

function openStream(after) {
  if (es) { es.close(); es = null; }
  hideReconnectFail();
  if (!cur) { return; }
  es = new EventSource(sseEventsUrl(after));
  bindSSE(es);
}
function closeStream() {
  if (es) { es.close(); es = null; }
  setReconnectBadge(false);
  clearTimeout(sseTimer);
}

function bindSSE(stream) {
  // Register one listener per known kind. All subscriber scripts load before
  // the first openStream() call, so the key set is complete by bind time.
  var kinds = Object.keys(SSE_HANDLERS);
  for (var i = 0; i < kinds.length; i++) {
    (function (kind) {
      stream.addEventListener(kind, function (e) {
        sseAttempts = 0;          // any received event proves the stream is alive
        setReconnectBadge(false); // (covers the spec's "reset on done" and more)
        var d = {};
        try { if (e.data) { d = JSON.parse(e.data); } } catch (_) { d = {}; }
        var fns = SSE_HANDLERS[kind] || [];
        for (var j = 0; j < fns.length; j++) { fns[j](d, e); }
      });
    })(kinds[i]);
  }
  stream.onerror = function () { tryReconnect(); };
}

// -- reconnect ---------------------------------------------------------------
// The browser's built-in EventSource retry would restart from seq 0 and
// replay the whole session, so we take over: close, read the persisted head
// via /seq, reopen with ?after=<seq>. Backoff 1s/2s/4s/8s/16s, max 5 tries.
function tryReconnect() {
  if (!cur) { return; }
  if (es) { es.close(); es = null; }
  if (sseAttempts >= SSE_MAX_ATTEMPTS) {
    setReconnectBadge(false);
    showReconnectFail();
    return;
  }
  var delay = 1000 * Math.pow(2, sseAttempts);
  sseAttempts++;
  setReconnectBadge(true);
  clearTimeout(sseTimer);
  sseTimer = setTimeout(function () {
    if (!cur) { return; }
    apiGet('/api/sessions/' + cur + '/seq').then(function (j) {
      openStream((j && j.seq) || 0);
    }, function () {
      tryReconnect(); // seq fetch itself failed: counts as another attempt
    });
  }, delay);
}

function setReconnectBadge(on) {
  var b = $('#reconnect');
  if (b) { b.style.display = on ? '' : 'none'; }
}
function showReconnectFail() {
  var b = $('#reconnect-fail');
  if (b) { b.style.display = ''; }
}
function hideReconnectFail() {
  var b = $('#reconnect-fail');
  if (b) { b.style.display = 'none'; }
}
