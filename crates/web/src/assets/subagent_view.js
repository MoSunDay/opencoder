// subagent_view.js - subagent transcript drill-down without touching chat.js:
// an "expand" control is bolted onto each subagent card (live ones via the
// subagent_start SSE event, historical ones restored from
// GET /api/sessions/:id/subagents after every transcript load), opening the
// child session's full transcript in a right drawer via the existing
// /api/sessions/:id/messages endpoint. Clicks reach us through #log event
// delegation so chat.js rendering stays untouched.
var saChildren = {};   // task id -> child session id (live SSE fast path)
var saMeta = {};       // task id -> {kind, status, prompt} (list endpoint)

// -- live cards: chat.js renders the card first (registration order), we add
// the expand button carrying the child session id from the same event.
onSSE('subagent_start', function (d) {
  if (!d || !d.id) { return; }
  saChildren[d.id] = d.child_session_id || null;
  addExpandBtn(d.id, d.child_session_id);
});

// -- restore after refresh: loadTranscript (chat.js) only renders messages,
// so subagent cards vanish on reload. Wrap it to re-render them from the
// durable subagent task list once the transcript paint settles.
var saLoadTranscript = loadTranscript;
loadTranscript = async function () {
  await saLoadTranscript.apply(null, arguments);
  await restoreSubagentCards(); // awaited: callers see a fully restored view
};

function saStatusDot(status) {
  if (status === 'completed') { return 'sa-dot ok'; }
  if (status === 'failed' || status === 'cancelled') { return 'sa-dot fail'; }
  return 'sa-dot running';
}

function addExpandBtn(taskId, childId) {
  var card = subagentCards[taskId];
  if (!card || card.querySelector('.sa-expand')) { return; }
  var btn = document.createElement('button');
  btn.className = 'sa-expand';
  btn.textContent = 'expand';
  btn.dataset.tid = taskId;
  if (childId) { btn.dataset.child = childId; }
  card.querySelector('.sa-head').appendChild(btn);
}

async function restoreSubagentCards() {
  if (!cur) { return; }
  var j;
  try { j = await apiGet('/api/sessions/' + cur + '/subagents'); }
  catch (e) { alertOnce('subagents', e); return; }
  alertOk('subagents');
  var tasks = (j && j.tasks) || [];
  for (var i = 0; i < tasks.length; i++) {
    var t = tasks[i];
    if (!t || !t.id) { continue; }
    saMeta[t.id] = { kind: t.kind, status: t.status, prompt: t.prompt };
    if (t.child_session_id) { saChildren[t.id] = t.child_session_id; }
    if (taskCarded(t.id)) { continue; }  // live-rendered card already present
    if (typeof renderSubagentCard !== 'function') { return; }
    renderSubagentCard({ id: t.id, kind: t.kind, prompt: t.prompt });
    var card = subagentCards[t.id];
    if (!card) { continue; }
    card.dataset.tid = t.id;
    var dot = card.querySelector('.sa-dot');
    if (dot) { dot.className = saStatusDot(t.status); }
    if (t.status !== 'running') {
      var steer = card.querySelector('.sa-steer');  // historical task: no steer
      if (steer) { steer.remove(); }
      var tail = document.createElement('div');
      tail.className = 'sa-tail ' + (t.status === 'completed' ? 'ok' : 'fail');
      tail.textContent = (t.status === 'completed' ? '\u2713 done' :
        (t.status === 'cancelled' ? '\u2717 cancelled' : '\u2717 failed')) +
        (t.result ? ': ' + String(t.result).slice(0, 200) : '');
      card.appendChild(tail);
    }
    addExpandBtn(t.id, t.child_session_id);
  }
}

function taskCarded(taskId) {
  if (subagentCards[taskId]) { return true; }                 // live map entry
  var btns = ($('#log') && $('#log').querySelectorAll('.sa-expand')) || [];
  for (var i = 0; i < btns.length; i++) {
    if (btns[i].dataset && btns[i].dataset.tid === taskId) { return true; }
  }
  return false;
}

// -- click delegation on #log: any .sa-expand click opens the child view.
function closestByClass(el, cls, stopAt) {
  while (el && el !== stopAt) {
    if (el.classList && el.classList.contains(cls)) { return el; }
    el = el._parent || el.parentElement;
  }
  return null;
}

function subagentViewClick(ev) {
  var log = $('#log');
  var btn = closestByClass(ev && ev.target, 'sa-expand', log);
  if (!btn || !log || !log.contains(btn)) { return; }
  openSubagentView(btn.dataset.tid, btn.dataset.child);
}

async function openSubagentView(taskId, childId) {
  if (!taskId) { return; }
  var child = childId || saChildren[taskId];
  var meta = saMeta[taskId] || {};
  if (!child) {
    // Live event missed the child id: resolve it from the durable list.
    try {
      var j = await apiGet('/api/sessions/' + cur + '/subagents');
      var tasks = (j && j.tasks) || [];
      for (var i = 0; i < tasks.length; i++) {
        if (tasks[i].id === taskId) {
          child = tasks[i].child_session_id;
          meta = tasks[i];
          saChildren[taskId] = child;
          break;
        }
      }
    } catch (e) { alert(e.error || e); return; }
  }
  if (!child) { alert('no child session recorded for this subagent'); return; }
  var msgs;
  try {
    var m = await apiGet('/api/sessions/' + child + '/messages');
    msgs = (m && m.messages) || [];
  } catch (e) { alert(e.error || e); return; }
  renderSaDrawer(taskId, meta, child, msgs);
}

// -- drawer: fixed right panel over a backdrop, transcript via chat.js's
// mkMsgDiv so child messages render exactly like the main transcript.
function ensureSaDrawer() {
  var old = document.getElementById('sa-drawer');
  if (old) { old.remove(); }
  var backdrop = document.createElement('div');
  backdrop.id = 'sa-backdrop';
  var drawer = document.createElement('div');
  drawer.id = 'sa-drawer';
  var body = document.body || document;
  body.appendChild(backdrop);
  body.appendChild(drawer);
  backdrop.onclick = closeSaDrawer;
  return drawer;
}

function renderSaDrawer(taskId, meta, child, messages) {
  var drawer = ensureSaDrawer();
  var hdr = document.createElement('div');
  hdr.className = 'sa-drawer-hdr';
  var dot = document.createElement('span');
  dot.className = saStatusDot(meta.status);
  var kind = document.createElement('b');
  kind.textContent = '[' + (meta.kind || 'subagent') + ']';
  var pr = document.createElement('span');
  pr.className = 'sa-prompt';
  pr.textContent = (meta.prompt || taskId).slice(0, 120);
  var close = document.createElement('button');
  close.textContent = '\u2715';
  close.onclick = closeSaDrawer;
  hdr.appendChild(dot); hdr.appendChild(kind); hdr.appendChild(pr); hdr.appendChild(close);
  var metaEl = document.createElement('div');
  metaEl.className = 'sa-drawer-meta';
  metaEl.textContent = 'status: ' + (meta.status || 'running') + ' \u00b7 child: ' + child;
  var bodyEl = document.createElement('div');
  bodyEl.className = 'sa-drawer-body';
  for (var i = 0; i < messages.length; i++) {
    bodyEl.appendChild(typeof mkMsgDiv === 'function'
      ? mkMsgDiv(messages[i])
      : textNode(String((messages[i] && messages[i].text) || '')));
  }
  if (!messages.length) {
    var empty = document.createElement('div');
    empty.className = 'sa-tail';
    empty.textContent = '(child transcript is empty)';
    bodyEl.appendChild(empty);
  }
  drawer.appendChild(hdr); drawer.appendChild(metaEl); drawer.appendChild(bodyEl);
  bodyEl.scrollTop = bodyEl.scrollHeight;
}

function textNode(t) {
  var el = document.createElement('div');
  el.className = 'b';
  el.textContent = t;
  return el;
}

function closeSaDrawer() {
  var d = document.getElementById('sa-drawer');
  if (d) { d.remove(); }
  var b = document.getElementById('sa-backdrop');
  if (b) { b.remove(); }
}

$('#log').addEventListener('click', subagentViewClick);
