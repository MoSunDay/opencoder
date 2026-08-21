// composer.js - send path: steer/queue + keyboard, image attach, $skill
// popup flow, pending-skill chip, subagent inline steer, busy transitions.
var pendingImages = [];
var pendingSkill = null;    // skill NAME sent with the NEXT prompt, then cleared
var skillCache = null;      // /api/skills fetched once
var skillPopIdx = 0, skillPopItems = [];

// -- busy transitions (send button + question polling follow this) ----------
function setBusy(v) {
  busy = v;
  updateSendBtn();
  if (v) { startQuestionPoll(); } else { stopQuestionPoll(true); }
}

// -- send / interrupt --------------------------------------------------------
async function send(delivery) {
  if (!cur) {
    await newSession();
    if (!cur) { return; }
  }
  var ta = $('#msg');
  var t = ta.value.trim();
  if (!t) { return; }
  setBusy(true);
  ta.value = '';
  ta.style.height = 'auto';

  // optimistic user echo (images inline)
  var u = document.createElement('div');
  u.className = 'm user';
  u.innerHTML = '<div class="r">user</div><div class="b"></div>';
  u.querySelector('.b').textContent = t;
  for (var i = 0; i < pendingImages.length; i++) {
    var img = document.createElement('img');
    img.className = 'img-att';
    img.src = pendingImages[i];
    img.style.maxHeight = '80px';
    u.appendChild(img);
  }
  $('#log').appendChild(u);
  scrollEnd();

  var body = { prompt: t, delivery: delivery || 'steer' };
  if (pendingImages.length) { body.images = pendingImages.slice(); pendingImages = []; renderImgPreview(); }
  if (pendingSkill) { body.skill = pendingSkill; pendingSkill = null; renderSkillChip(); }

  try {
    var j = await apiSend('POST', '/api/sessions/' + cur + '/prompt', body);
    if (j && j.ok === false) { alert(j.error || 'prompt rejected'); setBusy(false); return; }
    hideSkillPop();
    if (!es) { openStream(); }
  } catch (e) {
    alert(e.error || e);
    setBusy(false);
  }
}

function interrupt() {
  if (!cur) { return; }
  apiSend('POST', '/api/sessions/' + cur + '/interrupt').catch(function (e) { alert(e.error || e); });
}

function updateSendBtn() {
  var btn = $('#send');
  if (busy) {
    btn.textContent = 'Interrupt';
    btn.classList.add('interrupt');
    btn.onclick = interrupt;
  } else {
    btn.textContent = 'Send';
    btn.classList.remove('interrupt');
    btn.onclick = function () { send('steer'); };
  }
}

// -- image attach ------------------------------------------------------------
function renderImgPreview() {
  var c = $('#img-preview');
  c.innerHTML = '';
  pendingImages.forEach(function (src, i) {
    var wrap = document.createElement('div');
    wrap.className = 'pi';
    var img = document.createElement('img');
    img.src = src;
    var rm = document.createElement('button');
    rm.className = 'rm';
    rm.textContent = '\u2715';
    rm.onclick = function () { pendingImages.splice(i, 1); renderImgPreview(); };
    wrap.appendChild(img);
    wrap.appendChild(rm);
    c.appendChild(wrap);
  });
}
function readImageFile(f) {
  var reader = new FileReader();
  reader.onload = function () { pendingImages.push(reader.result); renderImgPreview(); };
  reader.readAsDataURL(f);
}
function handlePaste(e) {
  var items = e.clipboardData ? e.clipboardData.items : [];
  for (var i = 0; i < items.length; i++) {
    if (!items[i].type || items[i].type.indexOf('image/') !== 0) { continue; }
    var f = items[i].getAsFile();
    if (f) { readImageFile(f); }
  }
}
function handleFileSelect(e) {
  var files = e.target.files || [];
  for (var i = 0; i < files.length; i++) {
    if (files[i].type && files[i].type.indexOf('image/') === 0) { readImageFile(files[i]); }
  }
}

// -- $skill popup ------------------------------------------------------------
// Typing '$' at start-of-word opens the popup; arrows/enter/click pick; the
// pick inserts '$name ' and pins it as body.skill for the NEXT prompt only.
async function ensureSkills() {
  if (skillCache) { return skillCache; }
  try {
    var j = await apiGet('/api/skills');
    skillCache = (j && j.skills) || [];
  } catch (e) { skillCache = []; }
  return skillCache;
}
function skillPrefixBeforeCaret() {
  var ta = $('#msg');
  var pos = (ta.selectionStart == null) ? ta.value.length : ta.selectionStart;
  var m = /(^|\s)\$([A-Za-z0-9_-]*)$/.exec(ta.value.slice(0, pos));
  return m ? m[2] : null;
}
function skillPopOpen() { return $('#skill-pop').style.display !== 'none'; }
function hideSkillPop() { $('#skill-pop').style.display = 'none'; }
async function showSkillPop(prefix) {
  var skills = await ensureSkills();
  var p = (prefix || '').toLowerCase();
  skillPopItems = skills.filter(function (s) {
    return !p || String(s.name || '').toLowerCase().indexOf(p) === 0;
  });
  if (!skillPopItems.length) { hideSkillPop(); return; }
  skillPopIdx = 0;
  renderSkillPop();
}
function renderSkillPop() {
  var pop = $('#skill-pop');
  pop.innerHTML = '';
  skillPopItems.forEach(function (s, i) {
    var it = document.createElement('div');
    it.className = 'sp-item' + (i === skillPopIdx ? ' active' : '');
    var n = document.createElement('b');
    n.textContent = '$' + (s.name || '');
    var d = document.createElement('span');
    d.className = 'sp-desc';
    d.textContent = ' ' + (s.description || '');
    it.appendChild(n);
    it.appendChild(d);
    it.onmousedown = function (e) { e.preventDefault(); pickSkill(s.name); };
    pop.appendChild(it);
  });
  pop.style.display = '';
}
function moveSkillPop(dir) {
  if (!skillPopItems.length) { return; }
  skillPopIdx = (skillPopIdx + dir + skillPopItems.length) % skillPopItems.length;
  renderSkillPop();
}
function pickSkill(name) {
  var ta = $('#msg');
  var pos = (ta.selectionStart == null) ? ta.value.length : ta.selectionStart;
  var m = /(^|\s)\$([A-Za-z0-9_-]*)$/.exec(ta.value.slice(0, pos));
  if (m) {
    var start = pos - m[2].length - 1; // covers the '$' + typed prefix
    ta.value = ta.value.slice(0, start) + name + ' ' + ta.value.slice(pos);
    var np = start + name.length + 1;
    ta.setSelectionRange(np, np);
  }
  pendingSkill = name;
  renderSkillChip();
  hideSkillPop();
  ta.focus();
}
function renderSkillChip() {
  var c = $('#skill-chip');
  if (!pendingSkill) { c.style.display = 'none'; c.innerHTML = ''; return; }
  c.style.display = '';
  c.innerHTML = '';
  var s = document.createElement('span');
  s.textContent = 'skill: ' + pendingSkill;
  var x = document.createElement('button');
  x.className = 'chip-x';
  x.title = 'remove skill';
  x.textContent = '\u2715';
  x.onclick = function () { pendingSkill = null; renderSkillChip(); };
  c.appendChild(s);
  c.appendChild(x);
}

// -- subagent inline steer ---------------------------------------------------
function mountSteer(taskId, cardEl) {
  var old = cardEl.querySelector('.sa-steer-row');
  if (old) { old.querySelector('input').focus(); return; }
  var row = document.createElement('div');
  row.className = 'sa-steer-row';
  var inp = document.createElement('input');
  inp.type = 'text';
  inp.placeholder = 'steer this subagent...';
  var ok = document.createElement('button');
  ok.textContent = 'steer';
  ok.onclick = function () { steerSubagent(taskId, inp.value, cardEl); };
  inp.addEventListener('keydown', function (e) {
    if (e.key === 'Enter') { e.preventDefault(); steerSubagent(taskId, inp.value, cardEl); }
    else if (e.key === 'Escape') { row.remove(); }
  });
  row.appendChild(inp);
  row.appendChild(ok);
  cardEl.appendChild(row);
  inp.focus();
}
async function steerSubagent(taskId, prompt, cardEl) {
  prompt = (prompt || '').trim();
  if (!prompt || !cur) { return; }
  try {
    await apiSend('POST', '/api/sessions/' + cur + '/subagents/' + taskId + '/steer', { prompt: prompt });
    if (cardEl) {
      var row = cardEl.querySelector('.sa-steer-row');
      if (row) { row.remove(); }
      var note = document.createElement('div');
      note.className = 'sa-note';
      note.textContent = '\u21af steer admitted';
      cardEl.appendChild(note);
    }
  } catch (e) { alert(e.error || e); }
}

// -- welcome-hero example buttons --------------------------------------------
function prefillPrompt(text) {
  var ta = $('#msg');
  ta.value = text;
  ta.focus();
  autosizeMsg();
}

function autosizeMsg() {
  var ta = $('#msg');
  ta.style.height = 'auto';
  ta.style.height = Math.min(ta.scrollHeight, 120) + 'px';
}

// -- init --------------------------------------------------------------------
document.addEventListener('paste', handlePaste);
$('#msg').addEventListener('input', function () {
  autosizeMsg();
  var p = skillPrefixBeforeCaret();
  if (p === null) { hideSkillPop(); return; }
  showSkillPop(p);
});
$('#msg').addEventListener('keydown', function (e) {
  if (skillPopOpen()) {
    if (e.key === 'ArrowDown') { e.preventDefault(); moveSkillPop(1); return; }
    if (e.key === 'ArrowUp') { e.preventDefault(); moveSkillPop(-1); return; }
    if (e.key === 'Enter' || e.key === 'Tab') {
      e.preventDefault();
      if (skillPopItems[skillPopIdx]) { pickSkill(skillPopItems[skillPopIdx].name); }
      return;
    }
    if (e.key === 'Escape') { e.preventDefault(); hideSkillPop(); return; }
  }
  if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); if (!busy) { send('steer'); } }
  else if (e.key === 'Enter' && e.shiftKey) { e.preventDefault(); if (!busy) { send('queue'); } }
  else if (e.key === 'Escape' && busy) { interrupt(); }
});
(function () {
  var btns = document.querySelectorAll('#hero .hero-ex button');
  for (var i = 0; i < btns.length; i++) {
    (function (b) { b.onclick = function () { prefillPrompt(b.getAttribute('data-prompt') || ''); }; })(btns[i]);
  }
})();
updateSendBtn();
