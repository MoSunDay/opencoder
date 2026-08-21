// queue_panel.js - collapsible right drawer over the pending-input APIs:
// list queue+steer inputs, delete one, swap adjacent order. Live-refreshed on
// queue_consumed/steer_consumed SSE and polled every 2s while busy or open.
var qPanelOpen = false;
var qTimer = null;

function toggleQueuePanel() {
  qPanelOpen = !qPanelOpen;
  $('#qpanel').style.display = qPanelOpen ? '' : 'none';
  if (qPanelOpen) { refreshQueuePanel(true); }
}

function updateQBadge(n) {
  var el = $('#qcount');
  if (el) { el.textContent = String(n); }
}

async function refreshQueuePanel(force) {
  if (!cur) { updateQBadge(0); renderQueueList([]); return; }
  // Poll only while the panel is open or a run is busy (and the tab visible).
  if (!force && !qPanelOpen && !busy) { return; }
  if (document.hidden) { return; }
  var queue = [], steer = [];
  try {
    var q = await apiGet('/api/sessions/' + cur + '/inputs?delivery=queue');
    var s = await apiGet('/api/sessions/' + cur + '/inputs?delivery=steer');
    queue = (q && q.inputs) || [];
    steer = (s && s.inputs) || [];
  } catch (e) {
    alertOnce('inputs', e);
    return;
  }
  alertOk('inputs');
  updateQBadge(queue.length);
  renderQueueList(steer.concat(queue)); // steers first: they fire at turn boundary
}

function renderQueueList(items) {
  var box = $('#qp-list');
  if (!box) { return; }
  box.innerHTML = '';
  if (!items.length) {
    var none = document.createElement('div');
    none.className = 'qp-empty';
    none.textContent = 'no pending inputs';
    box.appendChild(none);
    return;
  }
  for (var i = 0; i < items.length; i++) { box.appendChild(mkQueueRow(items[i], items, i)); }
}

function mkQueueRow(it, items, idx) {
  var row = document.createElement('div');
  row.className = 'qp-item d-' + (it.delivery || 'steer');
  // up-down swap with the adjacent row OF THE SAME delivery (cross-delivery swaps
  // would change drain semantics, not just ordering).
  var prevSame = idx > 0 && items[idx - 1].delivery === it.delivery ? items[idx - 1] : null;
  var nextSame = idx < items.length - 1 && items[idx + 1].delivery === it.delivery ? items[idx + 1] : null;

  var badge = document.createElement('span');
  badge.className = 'qp-badge';
  badge.textContent = it.delivery || 'steer';
  row.appendChild(badge);

  var txt = document.createElement('span');
  txt.className = 'qp-text';
  var p = it.prompt || '';
  txt.textContent = p.length > 80 ? p.slice(0, 80) + '...' : p;
  txt.title = p;
  row.appendChild(txt);

  var up = document.createElement('button');
  up.className = 'qp-move';
  up.title = 'move earlier';
  up.innerHTML = '&#x25b2;';
  up.disabled = !prevSame;
  up.onclick = function () { if (prevSame) { reorderInputs(it.seq, prevSame.seq); } };
  row.appendChild(up);

  var down = document.createElement('button');
  down.className = 'qp-move';
  down.title = 'move later';
  down.innerHTML = '&#x25bc;';
  down.disabled = !nextSame;
  down.onclick = function () { if (nextSame) { reorderInputs(it.seq, nextSame.seq); } };
  row.appendChild(down);

  var del = document.createElement('button');
  del.className = 'qp-del';
  del.title = 'delete input';
  del.innerHTML = '&#x2715;';
  del.onclick = function () { deleteInput(it.seq); };
  row.appendChild(del);
  return row;
}

async function reorderInputs(a, b) {
  if (a == null || b == null) { return; }
  try {
    await apiSend('POST', '/api/sessions/' + cur + '/inputs/reorder', { a: a, b: b });
    refreshQueuePanel(true);
  } catch (e) { alert(e.error || e); }
}

async function deleteInput(seq) {
  try {
    await apiSend('DELETE', '/api/sessions/' + cur + '/inputs/' + seq);
    refreshQueuePanel(true);
  } catch (e) { alert(e.error || e); }
}

// SSE: a consumed input left the pending set - refresh immediately.
onSSE('queue_consumed', function () { refreshQueuePanel(true); });
onSSE('steer_consumed', function () { refreshQueuePanel(true); });

qTimer = setInterval(function () { refreshQueuePanel(false); }, 2000);
visibleAgain.push(function () { refreshQueuePanel(false); });
refreshQueuePanel(true);
