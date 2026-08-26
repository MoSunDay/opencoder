// nodes_panel.js - worker-node fleet panel (Phase 4): registry list with a
// 3s poll while open, a per-node dispatch form (prompt / agent / model), the
// node's task history, and a "live" task view fed by its OWN EventSource
// (/api/nodes/tasks/:tid/events). Not wired into sse.js SSE_HANDLERS: this
// stream must not join the main session reconnect logic. Reconnect strategy
// is total repaint: on error close, clear the rendered body and reopen the
// same ?after=0 URL — replayed frames rebuild exactly what we showed.
// State lives in file-scoped vars; index.html calls toggleNodesPanel only.
var npTimer = null;        // 3s list poller while the panel is open
var npSel = null;          // selected node record (dispatch/history target)
var npModels = null;       // cached catalog from GET /api/models
var npTaskId = null;       // task id shown in the live view
var npEs = null;           // panel-local EventSource
var npToolStarts = {};     // tool id -> timeline row (for end-of-tool reuse)
var npLastNodes = [];      // last registry snapshot (for instant repaint)

function nodesPanelVisible() {
  var p = $('#nodes-panel');
  return !!(p && p.style && p.style.display !== 'none');
}

function npTick() {
  if (!document.hidden && nodesPanelVisible()) { refreshNodes(); }
}

function toggleNodesPanel() {
  var p = $('#nodes-panel');
  var opening = p.style.display === 'none';
  p.style.display = opening ? '' : 'none';
  $('#ntoggle').classList.toggle('active', opening);
  if (!opening) {                        // closed: stop poller + live stream
    clearInterval(npTimer);
    npTimer = null;
    npCloseLive();
    return;
  }
  npBuildSkeleton();
  renderNodesList([]);
  renderNodeForm();
  refreshNodes();
  npTimer = setInterval(npTick, 3000);
}

// One-time inner skeleton of <aside id="nodes-panel">: registry list,
// dispatch form slot, task-history area (index.html ships them empty).
function npBuildSkeleton() {
  var p = $('#nodes-panel');
  if (!p || $('#np-nodes')) { return; }
  p.innerHTML = '';
  p.appendChild(npHeading('worker nodes'));
  p.appendChild(npBox('np-nodes'));
  p.appendChild(npHeading('dispatch a task'));
  p.appendChild(npBox('np-form'));
  p.appendChild(npHeading('task history'));
  p.appendChild(npBox('np-history-list'));
}

function npBox(id) {
  var d = document.createElement('div');
  d.id = id;
  return d;
}

function npHeading(text) {
  var h = document.createElement('h3');
  h.textContent = text;
  return h;
}

// -- registry list ---------------------------------------------------------------
async function refreshNodes() {
  var j;
  try { j = await apiGet('/api/nodes'); }
  catch (e) { alertOnce('nodes', e); return; }
  alertOk('nodes');
  npLastNodes = (j && j.nodes) || [];
  renderNodesList(npLastNodes);
  refreshNodeHistory();                    // selected-node chain sits above
}

function npDot(status) {
  return 'np-dot' + (status === 'busy' ? ' busy' : status === 'lost' ? ' lost' : '');
}

function renderNodesList(nodes) {
  var box = $('#np-nodes');
  if (!box) { return; }
  box.innerHTML = '';
  if (!nodes.length) {
    box.appendChild(npDiv('np-empty', 'no worker nodes registered'));
    return;
  }
  for (var i = 0; i < nodes.length; i++) {
    var n = nodes[i];
    var row = document.createElement('div');
    row.className = 'np-row' + (npSel && npSel.id === n.id ? ' active' : '');
    var dot = document.createElement('span');
    dot.className = npDot(n.status);
    row.appendChild(dot);
    var name = npDiv('np-name', n.name || '(unnamed)');
    name.title = (n.version || '') + ' @ ' + (n.workdir || '?');
    row.appendChild(name);
    var meta = npDiv('np-meta', (n.status === 'busy' && n.last_task_id)
      ? String(n.last_task_id).slice(0, 8)
      : (n.status || '?'));
    row.appendChild(meta);
    var del = document.createElement('button');
    del.className = 'np-del';
    del.title = 'remove node';
    del.innerHTML = '&#x2715;';
    del.onclick = function (nid) { return function () { deleteNode(nid); }; }(n.id);
    row.appendChild(del);
    row.onclick = function (rec) { return function () { selectNode(rec); }; }(n);
    box.appendChild(row);
  }
}

function npDiv(cls, text) {
  var el = document.createElement('div');
  el.className = cls;
  el.textContent = text == null ? '' : String(text);
  return el;
}

function selectNode(n) {
  npSel = n;
  npModels = null;                         // catalog may differ per workdir
  renderNodesList(npLastNodes);            // immediate active-row highlight
  renderNodeForm();
  refreshNodeHistory();
}

async function deleteNode(id) {
  if (!confirm('remove this node?')) { return; }
  try { await apiSend('DELETE', '/api/nodes/' + id); }
  catch (e) { alert(e.error || e); return; }
  if (npSel && npSel.id === id) { npSel = null; renderNodeForm(); }
  refreshNodes();
}

// -- dispatch form -----------------------------------------------------------------
function renderNodeForm() {
  var form = $('#np-form');
  if (!form) { return; }
  form.innerHTML = '';
  if (!npSel) {
    form.appendChild(npDiv('np-empty', 'select a node to dispatch a task'));
    return;
  }
  form.appendChild(npDiv('np-form-head', 'dispatch to ' + (npSel.name || npSel.id.slice(0, 8))));
  var ta = document.createElement('textarea');
  ta.id = 'np-prompt';
  ta.rows = 3;
  ta.placeholder = 'task prompt...';
  form.appendChild(ta);
  var agent = document.createElement('input');
  agent.id = 'np-agent';
  agent.type = 'text';
  agent.placeholder = 'agent/mode (optional)';
  form.appendChild(agent);
  var sel = document.createElement('select');
  sel.id = 'np-modelsel';
  form.appendChild(sel);
  fillModelOptions(sel);
  var go = document.createElement('button');
  go.id = 'np-dispatch';
  go.textContent = 'Dispatch';
  go.onclick = function () { dispatchToSelected(); };
  form.appendChild(go);
}

async function fillModelOptions(sel) {
  sel.innerHTML = '';
  sel.appendChild(npOpt('', '(server default)'));
  if (!npModels) {
    try { npModels = await apiGet('/api/models'); }
    catch (e) { alertOnce('npmodels', e); return; }
  }
  var ids = (npModels && npModels.models) || [];
  for (var i = 0; i < ids.length; i++) { sel.appendChild(npOpt(ids[i], ids[i])); }
}

function npOpt(value, label) {
  var o = document.createElement('option');
  o.value = value;
  o.textContent = label;
  return o;
}

async function dispatchToSelected() {
  if (!npSel) { return; }
  var prompt = (($('#np-prompt') || {}).value || '');
  if (!prompt.trim()) { alert('prompt must not be empty'); return; }
  var agent = (($('#np-agent') || {}).value || '').trim();
  var model = (($('#np-modelsel') || {}).value || '').trim();
  var body = { prompt: prompt };
  if (agent) { body.agent = agent; }
  if (model) { body.model = model; }
  var j;
  try { j = await apiSend('POST', '/api/nodes/' + npSel.id + '/tasks', body); }
  catch (e) { alert(e.error || e); return; }
  openNodeTaskLive((j && j.task_id) || '', npSel.id);
}

// -- history --------------------------------------------------------------------------
async function refreshNodeHistory() {
  var box = $('#np-history-list');
  if (!box || !npSel) { return; }
  var j;
  try { j = await apiGet('/api/nodes/' + npSel.id + '/tasks'); }
  catch (e) { alertOnce('nphist', e); return; }
  box.innerHTML = '';
  var tasks = ((j && j.tasks) || []).slice().reverse();   // newest first
  for (var i = 0; i < tasks.length; i++) {
    var t = tasks[i];
    var row = document.createElement('div');
    row.className = 'np-task-row';
    var bd = npDiv('', t.status);
    bd.className = 'np-badge ' + npBadgeClass(t.status);
    row.appendChild(bd);
    var txt = npDiv('np-task-txt', (t.prompt || '').slice(0, 60));
    txt.title = t.prompt || '';
    row.appendChild(txt);
    row.appendChild(npDiv('np-time', relTime(t.created_at)));
    row.onclick = function (tid) { return function () { openNodeTaskLive(tid); }; }(t.id);
    box.appendChild(row);
  }
}

// status -> badge vocabulary (.ok/.err/.warn): terminal green/red, rest amber.
function npBadgeClass(st) {
  if (st === 'done') { return 'ok'; }
  if (st === 'error') { return 'err'; }
  return 'warn';      // pending / running / cancelling / cancelled
}

// -- live view ------------------------------------------------------------------------
function openNodeTaskLive(taskId, nodeIdHint) {
  if (!taskId) { return; }
  if (nodeIdHint && (!npSel || npSel.id !== nodeIdHint)) {
    npSel = { id: nodeIdHint, name: String(nodeIdHint).slice(0, 8) };
  }
  npTaskId = taskId;
  buildLiveView();
  $('#nodes-live').style.display = '';
  $('#nodes-panel').style.display = 'none';
  npStreamEvents();
}

function buildLiveView() {
  var lv = $('#nodes-live');
  lv.innerHTML = '';
  var hdr = document.createElement('div');
  hdr.className = 'np-live-hdr';
  var back = document.createElement('button');
  back.textContent = '\u2190 back';
  back.onclick = function () { npCloseLive(); $('#nodes-panel').style.display = ''; };
  hdr.appendChild(back);
  hdr.appendChild(npDiv('np-live-title', 'task ' + String(npTaskId).slice(0, 8)));
  var cancel = document.createElement('button');
  cancel.id = 'np-cancel';
  cancel.textContent = 'Cancel';
  cancel.onclick = function () { requestLiveCancel(); };
  hdr.appendChild(cancel);
  var badge = npDiv('', 'running');
  badge.id = 'np-live-badge';
  badge.className = 'np-badge warn';
  hdr.appendChild(badge);
  lv.appendChild(hdr);
  var body = document.createElement('div');
  body.className = 'np-live-body';
  body.id = 'np-live-body';          // all frame handlers bind through this id
  lv.appendChild(body);
}

// Terminal frames are the server's CLOSURE frames: their payload carries
// task_id (+ ok/error/cancel). A drain's own empty done {} keeps us reading.
function npIsTerminal(kind, d) {
  return (kind === 'done' || kind === 'error') &&
    !!(d && typeof d.task_id === 'string');
}

function npStreamEvents() {
  if (npEs) { npEs.close(); }
  npEs = new EventSource(apiUrl('/api/nodes/tasks/' + npTaskId + '/events?after=0'));
  var kinds = ['text_delta', 'tool_start', 'tool_end', 'done', 'error'];
  for (var i = 0; i < kinds.length; i++) {
    (function (kind) {
      npEs.addEventListener(kind, function (e) {
        var d = {};
        try { if (e.data) { d = JSON.parse(e.data); } } catch (_) { d = {}; }
        if (kind === 'text_delta') { npAppendText(d.text || ''); }
        else if (npIsTerminal(kind, d)) { npFinishTerminal(kind, d); }
        else if (kind === 'tool_start') { npAddTool(d, false); }
        else if (kind === 'tool_end') { npAddTool(d, true); }
        else if (kind === 'error') { npFrameError(d); }   // mid-run, keep reading
        npScrollEnd();
      });
    })(kinds[i]);
  }
  npEs.onerror = function () {         // total-repaint reconnect after a drop
    if (!npTaskId) { return; }
    npEs.close();
    var b = $('#np-live-body');
    if (b) { b.innerHTML = ''; }
    setTimeout(function () { if (npTaskId) { npStreamEvents(); } }, 1000);
  };
}

function npAppendText(text) {
  var b = $('#np-live-body');
  if (!b) { return; }
  var last = b.lastChild;
  if (last && last.classList && last.classList.contains('np-text')) {
    last.textContent += text;
    return;
  }
  b.appendChild(npDiv('np-text', text));
}

function npAddTool(d, end) {
  var b = $('#np-live-body');
  if (!b) { return; }
  var row = (end && d.id && npToolStarts[d.id]) || null;
  if (!row) {
    row = npDiv('np-tool', '');
    var nm = document.createElement('b');
    nm.textContent = (end ? '\u2190 ' : '\u2699 ') + (d.name || (end ? 'result' : 'tool'));
    row.appendChild(nm);
    if (d.id) { npToolStarts[d.id] = row; row._t0 = Date.now(); }
    b.appendChild(row);
  }
  if (end) {
    if (d.is_error) { row.classList.add('err'); }
    if (row._t0) {
      var dur = npDiv('np-dur', Math.max(0, (Date.now() - row._t0) / 1000).toFixed(1) + 's');
      row.appendChild(dur);
    }
    delete npToolStarts[d.id];
  }
}

function npFrameError(d) {
  var b = $('#np-live-body');
  if (b) { b.appendChild(npDiv('np-error-line', d.error || 'error')); }
}

function npFinishTerminal(kind, d) {
  var badge = $('#np-live-badge');
  if (badge) {
    badge.className = 'np-badge ' +
      (kind === 'error' ? 'err' : (d.cancel ? 'warn' : 'ok'));
    badge.textContent = kind === 'error' ? 'failed' : (d.cancel ? 'cancelled' : 'completed');
  }
  var c = $('#np-cancel');
  if (c) { c.style.display = 'none'; }
  if (npEs) { npEs.close(); npEs = null; }   // explicit close at the terminal
}

async function requestLiveCancel() {
  var url = '/api/nodes/' + ((npSel && npSel.id) || '') +
    '/tasks/' + npTaskId + '/cancel';
  var btn = $('#np-cancel');
  try { await apiSend('POST', url); }
  catch (e) { alert(e.error || e); }
  if (btn) { btn.disabled = true; btn.textContent = 'cancelling...'; }
  var badge = $('#np-live-badge');
  if (badge) { badge.textContent = 'cancelling'; }
}

function npScrollEnd() {
  var b = $('#np-live-body');
  if (b) { b.scrollTop = b.scrollHeight; }
}

function npCloseLive() {
  if (npEs) { npEs.close(); npEs = null; }   // teardown also closes the stream
  npTaskId = null;
  var lv = $('#nodes-live');
  if (lv) { lv.style.display = 'none'; lv.innerHTML = ''; }
}

visibleAgain.push(function () { if (nodesPanelVisible()) { refreshNodes(); } });
