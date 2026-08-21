// settings.js - top-bar controls + settings popover. Loaded LAST (controller):
// model dropdown (/api/models) with custom fallback, agent select, autopilot,
// annotation, handoff, compact, fork.
var modelList = [];    // catalog from GET /api/models
var curModel = null;   // model of the active session (or config default)

// -- model dropdown ----------------------------------------------------------
async function loadModels() {
  var j = null;
  try { j = await apiGet('/api/models'); }
  catch (e) { alertOnce('models', e); return; } // keep the custom free-text fallback
  alertOk('models');
  modelList = (j && j.models) || [];
  var sel = $('#model-select');
  sel.innerHTML = '';
  for (var i = 0; i < modelList.length; i++) {
    var o = document.createElement('option');
    o.value = modelList[i];
    o.textContent = modelList[i];
    sel.appendChild(o);
  }
  var c = document.createElement('option');
  c.value = '__custom__';
  c.textContent = 'custom...';
  sel.appendChild(c);
  if (!curModel && j && j.default) { curModel = j.default; }
  syncModelSelect();
}

// Called by chat.js on transcript load and on model_switched SSE.
function setModelDisplay(m) {
  curModel = m || null;
  syncModelSelect();
}
function syncModelSelect() {
  var sel = $('#model-select');
  var inp = $('#model');
  if (!sel || !inp) { return; }
  var known = curModel && modelList.indexOf(curModel) >= 0;
  if (known) {
    sel.value = curModel;
    inp.style.display = 'none';
  } else if (curModel) {
    // session model not in the catalog: fall back to the free-text input
    sel.value = '__custom__';
    inp.style.display = '';
    inp.value = curModel;
  } else if (modelList.length) {
    sel.value = modelList[0]; // config default
    inp.style.display = 'none';
  } else {
    sel.value = '__custom__';
    inp.style.display = '';
  }
}
function onModelSelect() {
  var sel = $('#model-select');
  if (sel.value !== '__custom__') { switchModelTo(sel.value); }
  syncModelSelect(); // reveals/hides the custom input
}
async function switchModelTo(m) {
  if (!m || !cur) { return; }
  try {
    await apiSend('POST', '/api/sessions/' + cur + '/model', { value: m });
    setModelDisplay(m);
  } catch (e) { alert(e.error || e); }
}
function switchModel() { // custom free-text input
  var m = $('#model').value.trim();
  if (m) { switchModelTo(m); }
}

// -- agent mode --------------------------------------------------------------
function updateModeDisplay() { var el = $('#mode'); if (el) { el.value = mode; } }
async function switchAgent() {
  mode = $('#mode').value;
  if (!cur) { return; }
  try { await apiSend('POST', '/api/sessions/' + cur + '/agent', { value: mode }); }
  catch (e) { alert(e.error || e); }
}

// -- settings popover --------------------------------------------------------
function toggleSettings() {
  var pop = $('#settings-pop');
  var open = pop.style.display === 'none';
  pop.style.display = open ? '' : 'none';
  if (open) { loadSettingsPop(); }
}
// Read back live state when opening: annotation from session meta.requirement,
// autopilot from meta.autopilot_mode.
async function loadSettingsPop() {
  if (!cur) { return; }
  var j;
  try { j = await apiGet('/api/sessions/' + cur); }
  catch (e) { alertOnce('settings', e); return; }
  alertOk('settings');
  var meta = (j && j.meta) || {};
  $('#annotation').value = meta.requirement || '';
  $('#autopilot').value = meta.autopilot_mode || '';
}
async function saveAnnotation() {
  if (!cur) { return; }
  var txt = $('#annotation').value.trim();
  try { await apiSend('POST', '/api/sessions/' + cur + '/annotation', { text: txt || null }); }
  catch (e) { alert(e.error || e); }
}
async function clearAnnotation() {
  if (!cur) { return; }
  $('#annotation').value = '';
  try { await apiSend('POST', '/api/sessions/' + cur + '/annotation', { text: null }); }
  catch (e) { alert(e.error || e); }
}
async function switchAutopilot() {
  if (!cur) { return; }
  var v = $('#autopilot').value; // '' = follow the global config (send null)
  try { await apiSend('POST', '/api/sessions/' + cur + '/autopilot', { mode: v || null }); }
  catch (e) { alert(e.error || e); }
}
async function handoffSession() {
  if (!cur) { return; }
  var extra = prompt('extra prompt for the handoff (optional)', '');
  if (extra === null) { return; } // cancelled
  try { await apiSend('POST', '/api/sessions/' + cur + '/handoff', { extra: extra }); }
  catch (e) { alert(e.error || e); return; }
  if (!es) { openStream(); }
}

// -- fork / compact ----------------------------------------------------------
async function forkSession() {
  if (!cur) { return; }
  var j;
  try { j = await apiSend('POST', '/api/sessions/' + cur + '/fork'); }
  catch (e) { alert(e.error || e); return; }
  if (j && j.id) { await selectSession(j.id); }
}
async function compactSession() {
  if (!cur) { return; }
  try { await apiSend('POST', '/api/sessions/' + cur + '/compact'); }
  catch (e) { alert(e.error || e); return; }
  if (!es) { openStream(); }
}

// Click-outside closes the popover.
document.addEventListener('click', function (e) {
  var pop = $('#settings-pop');
  if (!pop || pop.style.display === 'none') { return; }
  if (pop.contains(e.target)) { return; }
  var gear = $('#gear');
  if (gear && gear.contains(e.target)) { return; }
  pop.style.display = 'none';
});

loadModels();
