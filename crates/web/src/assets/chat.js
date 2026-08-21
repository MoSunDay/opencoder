// chat.js - transcript rendering: snapshot replay, live streaming, subagent
// cards, usage chips, system chips, plan cards, empty-session welcome hero.
var curAssistant = null, curTool = null, curThink = null;
var subagentCards = {};   // subagent id -> card element
var usageEl = null;       // token chip of the running turn

// -- snapshot replay ---------------------------------------------------------
async function loadTranscript() {
  if (!cur) { return; }
  var j;
  try { j = await apiGet('/api/sessions/' + cur + '/messages'); }
  catch (e) { return; }
  renderMessages((j && j.messages) || []);
  var meta = (j && j.meta) || {};
  if (meta.agent) { mode = meta.agent; updateModeDisplay(); }
  if (typeof setModelDisplay === 'function') { setModelDisplay(meta.model || null); }
  $('#cur-id').textContent = cur.slice(0, 8);
}

function renderMessages(messages) {
  var log = $('#log');
  log.innerHTML = '';
  subagentCards = {};
  resetStreamTurn();
  for (var i = 0; i < messages.length; i++) { log.appendChild(mkMsgDiv(messages[i])); }
  log.scrollTop = log.scrollHeight;
}

function mkMsgDiv(m) {
  var div = document.createElement('div');
  div.className = 'm ' + m.role;
  var role = document.createElement('div');
  role.className = 'r';
  role.textContent = m.role;
  div.appendChild(role);
  if (m.blocks) {
    for (var i = 0; i < m.blocks.length; i++) { div.appendChild(mkBlock(m.blocks[i])); }
  } else {
    var body = document.createElement('div');
    body.className = 'b';
    body.textContent = m.text || '';
    div.appendChild(body);
  }
  return div;
}

function fmtObj(v) { return typeof v === 'string' ? v : JSON.stringify(v, null, 2); }

function mkBlock(b) {
  if (b.type === 'text') {
    var t = document.createElement('div');
    t.className = 'b';
    t.textContent = b.text || '';
    return t;
  }
  if (b.type === 'tool_use') {
    var el = document.createElement('div');
    el.className = 'tool';
    el.innerHTML = '<b>&#x1f527; ' + esc(b.name || 'tool') + '</b>';
    if (b.input) {
      var inp = document.createElement('div');
      inp.className = 'o';
      inp.textContent = fmtObj(b.input);
      el.appendChild(inp);
    }
    return el;
  }
  if (b.type === 'tool_result') {
    var r = document.createElement('div');
    r.className = 'tool' + (b.is_error ? ' err' : '');
    var out = b.output || (b.content ? b.content.map(function (c) { return c.text || ''; }).join('\n') : '');
    r.innerHTML = '<b>&#x2190; result</b>';
    var o = document.createElement('div');
    o.className = 'o';
    o.textContent = fmtObj(out);
    r.appendChild(o);
    return r;
  }
  if (b.type === 'reasoning') {
    var th = document.createElement('div');
    th.className = 'think';
    var lines = (b.text || '').split('\n').length;
    var head = mkThinkToggle(lines);
    var tb = document.createElement('div');
    tb.className = 'tb';
    tb.textContent = b.text || '';
    tb.style.display = 'none';
    head.onclick = function () { toggleThink(head, tb, lines); };
    th.appendChild(head);
    th.appendChild(tb);
    return th;
  }
  if (b.type === 'image' || b.type === 'image_url') {
    var img = document.createElement('img');
    img.className = 'img-att';
    img.src = b.url || (b.image_url && b.image_url.url) || b.data || '';
    return img;
  }
  return document.createElement('div');
}

function mkThinkToggle(lines) {
  var th = document.createElement('div');
  th.textContent = '\ud83d\udcad Thinking (' + lines + ' lines) [\u2193]';
  return th;
}
function toggleThink(th, tb, lines) {
  var open = tb.style.display !== 'none';
  tb.style.display = open ? 'none' : 'block';
  th.textContent = open ? mkThinkToggle(lines).textContent : '\ud83d\udcad [\u2191]';
}

// -- live turn plumbing ------------------------------------------------------
function resetStreamTurn() {
  curAssistant = null; curTool = null; curThink = null; usageEl = null;
}
function scrollEnd() { var log = $('#log'); log.scrollTop = log.scrollHeight; }

function ensureAssistant() {
  if (curAssistant) { return; }
  var div = document.createElement('div');
  div.className = 'm assistant';
  var role = document.createElement('div');
  role.className = 'r';
  role.textContent = 'assistant';
  var body = document.createElement('div');
  body.className = 'b';
  div.appendChild(role);
  div.appendChild(body);
  $('#log').appendChild(div);
  curAssistant = body;
  usageEl = null; // new turn: the next llm_usage gets a fresh chip
  scrollEnd();
}
function ensureThink() {
  if (curThink) { return; }
  ensureAssistant();
  var el = document.createElement('div');
  el.className = 'think';
  var th = mkThinkToggle(0);
  var tb = document.createElement('div');
  tb.className = 'tb';
  tb.style.display = 'none';
  th.onclick = function () { toggleThink(th, tb, (tb.textContent || '').split('\n').length); };
  el.appendChild(th);
  el.appendChild(tb);
  curAssistant.parentElement.insertBefore(el, curAssistant);
  curThink = { el: el, tb: tb };
}
function appendText(el, text) { el.textContent += text; scrollEnd(); }

function echoUserPrompt(text) {
  if (!text) { return; }
  // A consumed prompt starts a fresh visual turn: null out the current
  // assistant/tool/think blocks so the next delta creates a new turn AFTER
  // this echo instead of appending to the previous one.
  resetStreamTurn();
  var div = document.createElement('div');
  div.className = 'm user';
  var role = document.createElement('div');
  role.className = 'r';
  role.textContent = 'user';
  div.appendChild(role);
  var body = document.createElement('div');
  body.className = 'b';
  body.textContent = text;
  div.appendChild(body);
  $('#log').appendChild(div);
  scrollEnd();
}

function sysChip(text) {
  var el = document.createElement('div');
  el.className = 'sys-chip';
  el.textContent = text;
  $('#log').appendChild(el);
  scrollEnd();
  return el;
}

function usageChip(d) {
  if (!usageEl || !usageEl.parentElement) {
    usageEl = document.createElement('div');
    usageEl.className = 'usage';
    $('#log').appendChild(usageEl);
  }
  usageEl.textContent = '\u25b2in ' + (d.input_tokens || 0) +
    '  \u25bc out ' + (d.output_tokens || 0) +
    '  \u03a3 ' + (d.total_tokens || 0);
  scrollEnd();
}

// -- subagent cards ----------------------------------------------------------
function renderSubagentCard(d) {
  var el = document.createElement('div');
  el.className = 'subagent-card';
  var head = document.createElement('div');
  head.className = 'sa-head';
  var dot = document.createElement('span');
  dot.className = 'sa-dot running';
  var kind = document.createElement('b');
  kind.textContent = '[' + (d.kind || 'subagent') + ']';
  var pr = document.createElement('span');
  pr.className = 'sa-prompt';
  pr.textContent = (d.prompt || '').slice(0, 120);
  var steer = document.createElement('button');
  steer.className = 'sa-steer';
  steer.textContent = 'steer';
  steer.onclick = function () { mountSteer(d.id, el); };
  head.appendChild(dot);
  head.appendChild(kind);
  head.appendChild(pr);
  head.appendChild(steer);
  var body = document.createElement('div');
  body.className = 'sa-body';
  el.appendChild(head);
  el.appendChild(body);
  subagentCards[d.id] = el;
  $('#log').appendChild(el);
  scrollEnd();
}

// d.event is a serde externally-tagged SessionEvent, e.g. {"TextDelta": "..."}
function eventTag(ev) {
  if (!ev || typeof ev !== 'object') { return ''; }
  var ks = Object.keys(ev);
  return ks.length ? ks[0] : '';
}
function saAppend(id, cls, text) {
  var card = subagentCards[id];
  if (!card) { return; }
  var body = card.querySelector('.sa-body');
  var last = body.lastChild;
  if (cls === 'sa-text' && last && last.classList && last.classList.contains('sa-text')) {
    last.textContent += text; // merge consecutive child text deltas
  } else {
    var el = document.createElement('div');
    el.className = cls;
    el.textContent = text;
    body.appendChild(el);
  }
  scrollEnd();
}

// -- welcome hero (empty session) --------------------------------------------
function watchHero() {
  var hero = $('#hero');
  var log = $('#log');
  if (!hero || !log) { return; }
  var sync = function () { hero.style.display = log.childNodes.length ? 'none' : ''; };
  if (window.MutationObserver) { new MutationObserver(sync).observe(log, { childList: true }); }
  sync();
}

// -- SSE handler registration ------------------------------------------------
onSSE('text_delta', function (d) { ensureAssistant(); appendText(curAssistant, d.text || ''); });
onSSE('reasoning_delta', function (d) { ensureThink(); curThink.tb.textContent += (d.text || ''); });
onSSE('llm_usage', function (d) { usageChip(d || {}); });
onSSE('tool_start', function (d) {
  var el = document.createElement('div');
  el.className = 'tool';
  el.dataset.tid = d.id;
  el.innerHTML = '<b>&#x1f527; ' + esc(d.name || 'tool') + '</b>';
  if (d.input) {
    var inp = document.createElement('div');
    inp.className = 'o';
    inp.textContent = fmtObj(d.input);
    el.appendChild(inp);
  }
  $('#log').appendChild(el);
  curTool = el;
  scrollEnd();
  if (d.name === 'question') { questionsKick(); } // runtime question poll
});
onSSE('tool_end', function (d) {
  if (curTool && curTool.dataset.tid === d.id) {
    if (d.is_error) { curTool.classList.add('err'); }
    var o1 = document.createElement('div');
    o1.className = 'o';
    o1.textContent = fmtObj(d.output);
    curTool.appendChild(o1);
    if (d.images && d.images.length) {
      for (var i = 0; i < d.images.length; i++) {
        var img = document.createElement('img');
        img.className = 'img-att';
        img.src = d.images[i];
        curTool.appendChild(img);
      }
    }
  } else {
    var el = document.createElement('div');
    el.className = 'tool' + (d.is_error ? ' err' : '');
    el.innerHTML = '<b>&#x2190; ' + esc(d.name || 'result') + '</b>';
    var o2 = document.createElement('div');
    o2.className = 'o';
    o2.textContent = fmtObj(d.output);
    el.appendChild(o2);
    $('#log').appendChild(el);
  }
  curTool = null;
  scrollEnd();
});
onSSE('compaction_delta', function (d) {
  var box = $('#log').querySelector('.compaction-delta');
  if (!box) {
    box = document.createElement('div');
    box.className = 'compaction compaction-delta';
    box.appendChild(document.createElement('div')).className = 'sum';
    $('#log').appendChild(box);
  }
  box.querySelector('.sum').textContent += (d.text || '');
  scrollEnd();
});
onSSE('compaction', function (d) {
  var box = $('#log').querySelector('.compaction-delta');
  if (box) { box.remove(); }
  var el = document.createElement('div');
  el.className = 'compaction';
  var sum = document.createElement('div');
  sum.className = 'sum';
  sum.textContent = d.summary || 'compacted';
  el.appendChild(sum);
  $('#log').appendChild(el);
  scrollEnd();
});
onSSE('status', function (d) {
  var el = document.createElement('div');
  el.className = 'status';
  if (d.status === 'interrupted') {
    el.textContent = '\u26a0\ufe0f interrupted';
    setBusy(false);
  } else {
    el.textContent = d.status || '';
  }
  $('#log').appendChild(el);
  scrollEnd();
});
onSSE('agent_switched', function (d) { if (d.agent) { mode = d.agent; updateModeDisplay(); } });
onSSE('model_switched', function (d) { if (d.model && typeof setModelDisplay === 'function') { setModelDisplay(d.model); } });
onSSE('plan_handoff', function (d) {
  var el = document.createElement('div');
  el.className = 'plan-card';
  el.innerHTML = '<div class="ph">&#x1f4cb; Plan &#x2192; Act Handoff</div>';
  var body = document.createElement('div');
  body.className = 'pb';
  body.textContent = d.plan || '';
  el.appendChild(body);
  $('#log').appendChild(el);
  mode = 'act';
  updateModeDisplay();
  scrollEnd();
});
onSSE('transcript_reset', function () {
  $('#log').innerHTML = '';
  subagentCards = {};
  resetStreamTurn();
});
onSSE('subagent_start', function (d) { renderSubagentCard(d); });
onSSE('subagent_child', function (d) {
  var tag = eventTag(d.event);
  var v = d.event ? d.event[tag] : null;
  if (tag === 'TextDelta') { saAppend(d.id, 'sa-text', v || ''); }
  else if (tag === 'ReasoningDelta') { saAppend(d.id, 'sa-think', v || ''); }
  else if (tag === 'ToolStart') { saAppend(d.id, 'sa-tool', '\u2699 ' + ((v && v.name) || 'tool')); }
  else if (tag === 'ToolEnd') { saAppend(d.id, 'sa-tool', '\u2190 ' + ((v && v.name) || 'result')); }
});
onSSE('subagent_end', function (d) {
  var card = subagentCards[d.id];
  if (!card) { sysChip('subagent ended'); return; }
  var dot = card.querySelector('.sa-dot');
  if (dot) { dot.className = 'sa-dot ' + (d.cancelled ? 'fail' : (d.ok ? 'ok' : 'fail')); }
  var steer = card.querySelector('.sa-steer');
  if (steer) { steer.remove(); }
  var row = card.querySelector('.sa-steer-row');
  if (row) { row.remove(); }
  var tail = document.createElement('div');
  tail.className = 'sa-tail ' + (d.cancelled ? 'fail' : (d.ok ? 'ok' : 'fail'));
  tail.textContent = (d.cancelled ? '\u2717 cancelled' : (d.ok ? '\u2713 done' : '\u2717 failed')) +
    (d.summary ? ': ' + String(d.summary).slice(0, 200) : '');
  card.appendChild(tail);
  delete subagentCards[d.id];
  scrollEnd();
});
onSSE('autopilot', function (d) {
  sysChip('\ud83d\ude9c autopilot phase=' + (d.phase || '') + ' iter=' + (d.iteration || 0));
});
onSSE('queue_consumed', function (d) {
  sysChip('\u21af queue consumed');
  echoUserPrompt(d.text || '');
});
onSSE('steer_consumed', function (d) {
  sysChip('\u21af steer consumed');
  echoUserPrompt(d.text || '');
});
onSSE('error', function (d) {
  var el = document.createElement('div');
  el.className = 'error';
  el.textContent = d.error || 'error';
  $('#log').appendChild(el);
  scrollEnd();
  setBusy(false);
});
onSSE('done', function () {
  setBusy(false);
  resetStreamTurn();
  loadTranscript();
});

watchHero();
