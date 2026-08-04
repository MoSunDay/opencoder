// render.js — shared state, helpers, transcript rendering, live SSE streaming.

const $ = s => document.querySelector(s);
const $$ = s => document.querySelectorAll(s);

// shared mutable state (classic scripts share global lexical scope)
let cur = null, busy = false, es = null, pendingImages = [], mode = 'act';
let curAssistant = null, curTool = null, curThink = null;

// ── helpers ─────────────────────────────────────────────────────────────────
function esc(s) { const d = document.createElement('div'); d.textContent = s || ''; return d.innerHTML; }
function tokenParam() {
  const t = new URLSearchParams(location.search).get('token');
  return t ? '?token=' + encodeURIComponent(t) : '';
}
function authHeaders(extra) {
  const t = new URLSearchParams(location.search).get('token');
  const h = { ...extra };
  if (t) h['authorization'] = 'Bearer ' + t;
  return h;
}
function withToken(url) { return url + tokenParam(); }

// ── transcript snapshot ─────────────────────────────────────────────────────
async function load() {
  if (!cur) return;
  const r = await fetch(withToken('/api/sessions/' + cur + '/messages'));
  const j = await r.json();
  renderTranscript(j.messages || []);
  if (j.meta && j.meta.agent) { mode = j.meta.agent; updateModeDisplay(); }
}

function renderTranscript(messages) {
  const log = $('#log');
  log.innerHTML = '';
  messages.forEach(m => log.appendChild(mkMsgDiv(m)));
  log.scrollTop = log.scrollHeight;
}

function mkMsgDiv(m) {
  const div = document.createElement('div'); div.className = 'm ' + m.role;
  const role = document.createElement('div'); role.className = 'r'; role.textContent = m.role;
  div.appendChild(role);
  if (m.blocks) m.blocks.forEach(b => div.appendChild(mkBlock(b)));
  else { const body = document.createElement('div'); body.className = 'b'; body.textContent = m.text || ''; div.appendChild(body); }
  return div;
}

function fmtObj(v) { return typeof v === 'string' ? v : JSON.stringify(v, null, 2); }

function mkBlock(b) {
  if (b.type === 'text') { const el = document.createElement('div'); el.className = 'b'; el.textContent = b.text || ''; return el; }
  if (b.type === 'tool_use') {
    const el = document.createElement('div'); el.className = 'tool';
    el.innerHTML = '<b>&#x1f527; ' + esc(b.name || 'tool') + '</b>';
    if (b.input) { const inp = document.createElement('div'); inp.className = 'o'; inp.textContent = fmtObj(b.input); el.appendChild(inp); }
    return el;
  }
  if (b.type === 'tool_result') {
    const el = document.createElement('div'); el.className = 'tool' + (b.is_error ? ' err' : '');
    const out = b.output || (b.content ? b.content.map(c => c.text || '').join('\n') : '');
    el.innerHTML = '<b>&#x2190; result</b>';
    const o = document.createElement('div'); o.className = 'o'; o.textContent = fmtObj(out); el.appendChild(o);
    return el;
  }
  if (b.type === 'reasoning') {
    const el = document.createElement('div'); el.className = 'think';
    const lines = (b.text || '').split('\n').length;
    const th = mkThinkToggle(lines);
    const tb = document.createElement('div'); tb.className = 'tb'; tb.textContent = b.text || ''; tb.style.display = 'none';
    th.onclick = () => toggleThink(th, tb, lines);
    el.appendChild(th); el.appendChild(tb); return el;
  }
  if (b.type === 'image' || b.type === 'image_url') {
    const el = document.createElement('div');
    const img = document.createElement('img'); img.className = 'img-att';
    img.src = b.url || (b.image_url && b.image_url.url) || b.data || '';
    el.appendChild(img); return el;
  }
  return document.createElement('div');
}

function mkThinkToggle(lines) {
  const th = document.createElement('div');
  th.textContent = '\u{1F4AD} Thinking (' + lines + ' lines) [\u2193]';
  return th;
}
function toggleThink(th, tb, lines) {
  const open = tb.style.display !== 'none';
  tb.style.display = open ? 'none' : 'block';
  th.textContent = open ? mkThinkToggle(lines).textContent : '\u{1F4AD} [\u2191]';
}

// ── live SSE streaming ──────────────────────────────────────────────────────
function ensureAssistant() {
  if (curAssistant) return;
  const div = document.createElement('div'); div.className = 'm assistant';
  const role = document.createElement('div'); role.className = 'r'; role.textContent = 'assistant';
  const body = document.createElement('div'); body.className = 'b';
  div.appendChild(role); div.appendChild(body);
  $('#log').appendChild(div);
  curAssistant = body;
  scrollEnd();
}
function ensureThink() {
  if (curThink) return;
  ensureAssistant();
  const el = document.createElement('div'); el.className = 'think';
  const th = mkThinkToggle(0);
  const tb = document.createElement('div'); tb.className = 'tb'; tb.style.display = 'none';
  th.onclick = () => toggleThink(th, tb, (tb.textContent || '').split('\n').length);
  el.appendChild(th); el.appendChild(tb);
  curAssistant.parentElement.insertBefore(el, curAssistant);
  curThink = { el, tb };
}
function appendText(el, text) { el.textContent += text; scrollEnd(); }
function scrollEnd() { const log = $('#log'); log.scrollTop = log.scrollHeight; }

function openStream() {
  if (es) es.close();
  if (!cur) return;
  curAssistant = null; curTool = null; curThink = null;
  es = new EventSource(withToken('/api/sessions/' + cur + '/events'));
  bindSSE(es);
}

function bindSSE(stream) {
  stream.addEventListener('text_delta', e => { const d = JSON.parse(e.data); ensureAssistant(); appendText(curAssistant, d.text || ''); });
  stream.addEventListener('reasoning_delta', e => { const d = JSON.parse(e.data); ensureThink(); curThink.tb.textContent += (d.text || ''); });
  stream.addEventListener('tool_start', e => {
    const d = JSON.parse(e.data); const el = document.createElement('div'); el.className = 'tool'; el.dataset.tid = d.id;
    el.innerHTML = '<b>&#x1f527; ' + esc(d.name || 'tool') + '</b>';
    if (d.input) { const inp = document.createElement('div'); inp.className = 'o'; inp.textContent = fmtObj(d.input); el.appendChild(inp); }
    $('#log').appendChild(el); curTool = el; scrollEnd();
  });
  stream.addEventListener('tool_end', e => {
    const d = JSON.parse(e.data);
    if (curTool && curTool.dataset.tid === d.id) {
      if (d.is_error) curTool.classList.add('err');
      const o = document.createElement('div'); o.className = 'o'; o.textContent = fmtObj(d.output); curTool.appendChild(o);
    } else {
      const el = document.createElement('div'); el.className = 'tool' + (d.is_error ? ' err' : '');
      el.innerHTML = '<b>&#x2190; ' + esc(d.name || 'result') + '</b>';
      const o = document.createElement('div'); o.className = 'o'; o.textContent = fmtObj(d.output); el.appendChild(o);
      $('#log').appendChild(el);
    }
    curTool = null; scrollEnd();
  });
  stream.addEventListener('compaction_delta', e => {
    const d = JSON.parse(e.data);
    let box = $('#log').querySelector('.compaction-delta');
    if (!box) { box = document.createElement('div'); box.className = 'compaction compaction-delta'; const sum = document.createElement('div'); sum.className = 'sum'; box.appendChild(sum); $('#log').appendChild(box); }
    box.querySelector('.sum').textContent += (d.text || ''); scrollEnd();
  });
  stream.addEventListener('compaction', e => {
    const d = JSON.parse(e.data);
    const box = $('#log').querySelector('.compaction-delta'); if (box) box.remove();
    const el = document.createElement('div'); el.className = 'compaction';
    const sum = document.createElement('div'); sum.className = 'sum'; sum.textContent = d.summary || 'compacted';
    el.appendChild(sum); $('#log').appendChild(el); scrollEnd();
  });
  stream.addEventListener('status', e => {
    const d = JSON.parse(e.data); const st = d.status || '';
    const el = document.createElement('div'); el.className = 'status';
    if (st === 'interrupted') { el.textContent = '\u26a0\ufe0f interrupted'; busy = false; updateSendBtn(); }
    else { el.textContent = st; }
    $('#log').appendChild(el); scrollEnd();
  });
  stream.addEventListener('agent_switched', e => { const d = JSON.parse(e.data); if (d.agent) { mode = d.agent; updateModeDisplay(); } });
  stream.addEventListener('model_switched', e => { const d = JSON.parse(e.data); if (d.model && $('#model')) $('#model').value = d.model; });
  stream.addEventListener('plan_handoff', e => {
    const d = JSON.parse(e.data); const el = document.createElement('div'); el.className = 'plan-card';
    el.innerHTML = '<div class="ph">&#x1f4cb; Plan &#x2192; Act Handoff</div>';
    const body = document.createElement('div'); body.className = 'pb'; body.textContent = d.plan || '';
    el.appendChild(body); $('#log').appendChild(el); mode = 'act'; updateModeDisplay(); scrollEnd();
  });
  stream.addEventListener('transcript_reset', () => { $('#log').innerHTML = ''; curAssistant = curTool = curThink = null; });
  stream.addEventListener('subagent_start', e => {
    const d = JSON.parse(e.data); const el = document.createElement('div'); el.className = 'subagent';
    el.innerHTML = '<span class="sk">[' + esc(d.kind || 'subagent') + ']</span> <span class="sm">' + esc((d.prompt || '').slice(0, 100)) + '</span>';
    el.dataset.sid = d.id; $('#log').appendChild(el); scrollEnd();
  });
  stream.addEventListener('subagent_end', e => {
    const d = JSON.parse(e.data); const el = document.createElement('div'); el.className = 'subagent';
    const status = d.cancelled ? 'cancelled' : (d.ok ? 'done' : 'failed');
    el.innerHTML = '<span class="sm">subagent ' + status + ': ' + esc((d.summary || '').slice(0, 200)) + '</span>';
    $('#log').appendChild(el); scrollEnd();
  });
  stream.addEventListener('autopilot', e => { const d = JSON.parse(e.data); const el = document.createElement('div'); el.className = 'status'; el.textContent = '\u{1F69C} autopilot [' + (d.phase || '') + '] iter ' + (d.iteration || 0); $('#log').appendChild(el); scrollEnd(); });
  stream.addEventListener('error', e => {
    if (e && e.data) { try { const d = JSON.parse(e.data); const el = document.createElement('div'); el.className = 'error'; el.textContent = d.error || 'error'; $('#log').appendChild(el); scrollEnd(); } catch (_) {} }
    if (es && es.readyState === EventSource.CLOSED) { es = null; }
    busy = false; updateSendBtn();
  });
  stream.addEventListener('done', () => { busy = false; updateSendBtn(); curAssistant = curTool = curThink = null; load(); });
}
