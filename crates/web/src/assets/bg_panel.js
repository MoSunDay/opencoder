// bg_panel.js - background-process panel inside the settings popover: lists
// the server's live background processes (GET /api/bg) with a stop-all
// button (POST /api/bg/stop). Refreshed on a 5s timer while the popover is
// open (and the tab visible) plus on tab focus; no settings.js changes.
function bgPanelVisible() {
  var p = $('#settings-pop');
  return !!(p && p.style && p.style.display !== 'none');
}

function bgTick() {
  if (!document.hidden && bgPanelVisible()) { refreshBgPanel(); }
}

async function refreshBgPanel() {
  var j;
  try { j = await apiGet('/api/bg'); }
  catch (e) { alertOnce('bg', e); return; }
  alertOk('bg');
  renderBgList((j && j.processes) || []);
}

function renderBgList(procs) {
  var box = $('#bg-list');
  if (!box) { return; }
  box.innerHTML = '';
  if (!procs.length) {
    var none = document.createElement('div');
    none.className = 'bg-empty';
    none.textContent = 'no background processes';
    box.appendChild(none);
    return;
  }
  for (var i = 0; i < procs.length; i++) {
    var row = document.createElement('div');
    row.className = 'bg-row';
    var pid = document.createElement('span');
    pid.className = 'bg-pid';
    pid.textContent = 'pid ' + (procs[i].pid || '?');
    var path = document.createElement('span');
    path.className = 'bg-path';
    path.textContent = procs[i].output_path || '';
    path.title = procs[i].output_path || '';
    row.appendChild(pid);
    row.appendChild(path);
    box.appendChild(row);
  }
  var stop = document.createElement('button');
  stop.textContent = 'stop all';
  stop.onclick = stopBgAll;
  box.appendChild(stop);
}

async function stopBgAll() {
  var j;
  try { j = await apiSend('POST', '/api/bg/stop'); }
  catch (e) { alert(e.error || e); return; }
  if (j && typeof j.killed === 'number') {
    sysChip('bg: stopped ' + j.killed + ' process(es)');
  }
  refreshBgPanel();
}

visibleAgain.push(function () { if (bgPanelVisible()) { refreshBgPanel(); } });
setInterval(bgTick, 5000);
refreshBgPanel();
