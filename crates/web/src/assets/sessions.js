// sessions.js - sidebar: server-side search (300ms debounce), session rows
// (title, agent/model/skill-NAME badges, relative time), select/new/delete.
function currentId() { return cur; }

function relTime(ms) {
  if (!ms) { return ''; }
  var d = Date.now() - ms;
  if (d < 0) { d = 0; }
  var m = Math.floor(d / 60000);
  if (m < 1) { return 'now'; }
  if (m < 60) { return m + 'm ago'; }
  var h = Math.floor(m / 60);
  if (h < 24) { return h + 'h ago'; }
  return Math.floor(h / 24) + 'd ago';
}

async function refreshSessions() {
  var search = ($('#search').value || '').trim();
  var path = '/api/sessions?limit=50' + (search ? '&search=' + encodeURIComponent(search) : '');
  var j;
  try { j = await apiGet(path); } catch (e) { alertOnce('sessions', e); return; }
  alertOk('sessions');
  var list = (j && j.sessions) || [];
  var box = $('#sess-list');
  box.innerHTML = '';
  if (!list.length) {
    var none = document.createElement('div');
    none.style.cssText = 'color:var(--mut);padding:8px;font-size:12px';
    none.textContent = 'no sessions';
    box.appendChild(none);
    return;
  }
  for (var i = 0; i < list.length; i++) { box.appendChild(mkSessionRow(list[i])); }
}

function mkBadge(cls, text, title) {
  var b = document.createElement('span');
  b.className = 'bdg ' + cls;
  b.textContent = text;
  if (title) { b.title = title; }
  return b;
}

function mkSessionRow(s) {
  var row = document.createElement('div');
  row.className = 'sess' + (s.id === cur ? ' active' : '');
  row.title = s.title || s.id;

  var main = document.createElement('div');
  main.className = 'sess-main';
  var t = document.createElement('div');
  t.className = 'sess-title';
  t.textContent = s.title || s.id.slice(0, 8);
  main.appendChild(t);

  // meta badges: agent, model (short form, full id on hover), skill NAME only
  var meta = document.createElement('div');
  meta.className = 'sess-meta';
  if (s.agent) { meta.appendChild(mkBadge('bdg-agent', s.agent)); }
  if (s.model) {
    meta.appendChild(mkBadge('bdg-model', String(s.model).split('/').pop(), s.model));
  }
  // `skill` in list items is the skill NAME (SessionMeta.skill), never the body.
  if (s.skill) { meta.appendChild(mkBadge('bdg-skill', '$' + s.skill)); }
  var tm = document.createElement('span');
  tm.className = 'sess-time';
  tm.textContent = relTime(s.updated_at);
  meta.appendChild(tm);
  main.appendChild(meta);
  row.appendChild(main);

  var del = document.createElement('button');
  del.className = 'del';
  del.title = 'delete';
  del.innerHTML = '&#x2715;';
  del.onclick = function (ev) { ev.stopPropagation(); delSession(s.id); };
  row.appendChild(del);

  row.onclick = function () { selectSession(s.id); };
  return row;
}

async function selectSession(id) {
  cur = id;
  closeStream();
  setBusy(false);
  $('#log').innerHTML = '';
  resetStreamTurn();
  clearQuestionCards();
  await loadTranscript();
  await refreshSessions();
  refreshQueuePanel(true);
  openStream();
}

async function newSession() {
  var j;
  try { j = await apiSend('POST', '/api/sessions', {}); }
  catch (e) { alert(e.error || e); return; }
  if (j && j.id) { await selectSession(j.id); }
}

async function delSession(id) {
  if (!id || !confirm('Delete this session?')) { return; }
  try { await apiSend('DELETE', '/api/sessions/' + id); }
  catch (e) { alert(e.error || e); return; }
  if (cur === id) {
    cur = null;
    closeStream();
    $('#log').innerHTML = '';
    $('#cur-id').textContent = '';
  }
  refreshSessions();
  refreshQueuePanel(true);
}

// server-side search with 300ms debounce
$('#search').addEventListener('input', function () {
  clearTimeout(window._searchTimer);
  window._searchTimer = setTimeout(refreshSessions, 300);
});

refreshSessions();
