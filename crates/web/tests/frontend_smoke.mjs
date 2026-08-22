#!/usr/bin/env node
// Headless runtime acceptance for the embedded web frontend: loads the REAL
// asset scripts (api/sse/sessions/chat/composer/questions/queue_panel/
// settings) into a vm with a minimal DOM shim + mock fetch, then asserts the
// runtime behaviors the static html.rs tests cannot see: question closed
// loop, queue panel list/reorder/delete, model dropdown, SSE reconnect
// badge, and the composer send path. Exit 0 = all assertions passed.

import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
const ASSETS = join(HERE, '..', 'src', 'assets');

// ── tiny DOM shim ─────────────────────────────────────────────────────────
class El {
  constructor(tag, id) {
    this.tag = (tag || 'div').toLowerCase();
    this.id = id || '';
    this.children = [];
    this._text = '';
    this._cls = new Set();
    this.style = { display: id === 'reconnect' || id === 'reconnect-fail' ? 'none' : '' };
    this.attrs = {};
    this.dataset = {};
    this.disabled = false;
    this.value = '';
    this.title = '';
    this.type = '';
    this.src = '';
    this.scrollTop = 0;
    this.scrollHeight = 0;
    this.onclick = null;
    this._parent = null;
    const self = this;
    this.classList = {
      add: (c) => self._cls.add(c),
      remove: (c) => self._cls.delete(c),
      contains: (c) => self._cls.has(c),
    };
  }
  get className() { return [...this._cls].join(' '); }
  set className(v) { this._cls = new Set(String(v).split(/\s+/).filter(Boolean)); }
  get textContent() {
    return this._text + this.children.map((c) => c.textContent).join('');
  }
  set textContent(v) { this._text = String(v); this.children = []; }
  get childNodes() { return this.children; }
  get innerHTML() { return this._text; }
  set innerHTML(v) {
    this.children = [];
    this._text = '';
    if (!v) { return; }
    // Minimal parse of flat "<tag attrs>text</tag>" sequences (the only
    // shapes the assets build via innerHTML: r/b wrappers, <b>tool</b>).
    const re = /<(\w+)([^>]*)>([^<]*)<\/\1>/g;
    let m;
    while ((m = re.exec(v)) !== null) {
      const el = new El(m[1]);
      const cls = /class="([^"]*)"/.exec(m[2]);
      if (cls) { el.className = cls[1]; }
      this.appendChild(el);
      const holder = new El('#text');
      holder.textContent = m[3];
      el.appendChild(holder);
    }
    if (this.children.length === 0) { this._text = v; }
  }
  appendChild(c) { c._parent = this; this.children.push(c); return c; }
  remove() {
    const p = this._parent;
    if (p) { const i = p.children.indexOf(this); if (i >= 0) p.children.splice(i, 1); }
  }
  setAttribute(k, v) { this.attrs[k] = String(v); }
  getAttribute(k) { return k in this.attrs ? this.attrs[k] : null; }
  addEventListener() {}
  focus() {}
  querySelector(sel) { return qsa(this, sel)[0] || null; }
  querySelectorAll(sel) { return qsa(this, sel); }
  contains(x) { let n = x; while (n) { if (n === this) return true; n = n._parent; } return false; }
}

function matches(el, tok) {
  if (tok[0] === '#') return el.id === tok.slice(1);
  if (tok[0] === '.') return el._cls.has(tok.slice(1));
  const attr = /^\[\s*([\w-]+)\s*=\s*"([^"]*)"\s*\]$/.exec(tok);
  if (attr) return el.attrs[attr[1]] === attr[2];
  return el.tag === tok.toLowerCase();
}
function matchesChain(el, toks) {
  if (!matches(el, toks[toks.length - 1])) return false;
  let n = el._parent;
  for (let i = toks.length - 2; i >= 0; i--) {
    while (n && !matches(n, toks[i])) n = n._parent;
    if (!n) return false;
    n = n._parent;
  }
  return true;
}
function qsa(root, sel) {
  const out = [];
  for (const part of sel.split(',').map((s) => s.trim()).filter(Boolean)) {
    const toks = part.split(/\s+/);
    (function dfs(n) {
      for (const c of n.children) {
        if (toks.length === 1 ? matches(c, toks[0]) : matchesChain(c, toks)) out.push(c);
        dfs(c);
      }
    })(root);
  }
  return out;
}

const body = new El('body');
const byId = (id) => body.children.find((c) => c.id === id) || null;
const SKELETON_IDS = ['side', 'search', 'sess-list', 'cur-id', 'mode', 'model',
  'model-select', 'gear', 'settings-pop', 'annotation', 'autopilot', 'handoff', 'reconnect',
  'reconnect-fail', 'log-wrap', 'log', 'hero', 'questions', 'composer',
  'skill-chip', 'msg', 'img-preview', 'skill-pop', 'send', 'qpanel', 'qp-list',
  'qcount', 'qtoggle', 'top', 'main'];
for (const id of SKELETON_IDS) body.appendChild(new El('div', id));

const document = {
  hidden: false,
  body,
  querySelector: (sel) => qsa(body, sel)[0] || null,
  querySelectorAll: (sel) => qsa(body, sel),
  createElement: (tag) => new El(tag),
  addEventListener: () => {},
};

// ── mock backend ──────────────────────────────────────────────────────────
const state = {
  questions: [], inputs: { steer: [], queue: [] }, seq: 5,
  models: { default: 'a/b', models: ['a/b', 'x/y'] },
  sessions: [{ id: 's1', title: 'smoke session', agent: 'act', updated_at: Date.now() }],
  agentStatus: 200,
};
const CALLS = [];
const ALERTS = [];
function resp(status, j) {
  return { ok: status >= 200 && status < 300, status, json: async () => j };
}
async function fetch(url, opts = {}) {
  const path = url.split('?')[0];
  const method = (opts.method || 'GET').toUpperCase();
  let body = null;
  if (opts.body) { body = JSON.parse(opts.body); }
  CALLS.push({ method, path, body });
  let m;
  if (method === 'GET' && (m = /^\/api\/sessions\/([^/]+)\/questions$/.exec(path))) {
    return resp(200, { questions: state.questions });
  }
  if (method === 'POST' && (m = /^\/api\/sessions\/([^/]+)\/questions\/([^/]+)\/(answer|skip)$/.exec(path))) {
    state.questions = state.questions.filter((q) => q.id !== m[2]);
    return resp(200, { ok: true });
  }
  if (method === 'GET' && (m = /^\/api\/sessions\/([^/]+)\/inputs$/.exec(path))) {
    const delivery = (url.split('delivery=')[1] || '').split('&')[0];
    return resp(200, { inputs: state.inputs[delivery] || [] });
  }
  if (method === 'POST' && /\/inputs\/reorder$/.test(path)) {
    const swap = (list) => {
      const ia = list.findIndex((i) => i.seq === body.a);
      const ib = list.findIndex((i) => i.seq === body.b);
      if (ia >= 0 && ib >= 0) { [list[ia], list[ib]] = [list[ib], list[ia]]; }
    };
    swap(state.inputs.queue);
    swap(state.inputs.steer);
    return resp(200, { ok: true });
  }
  if (method === 'DELETE' && (m = /\/inputs\/(\d+)$/.exec(path))) {
    const seq = Number(m[1]);
    state.inputs.queue = state.inputs.queue.filter((i) => i.seq !== seq);
    state.inputs.steer = state.inputs.steer.filter((i) => i.seq !== seq);
    return resp(200, { ok: true });
  }
  if (method === 'GET' && path === '/api/models') return resp(200, state.models);
  if (method === 'GET' && /\/seq$/.test(path)) return resp(200, { seq: state.seq });
  if (method === 'GET' && /\/messages$/.test(path)) return resp(200, { messages: [], meta: {} });
  if (method === 'GET' && path === '/api/sessions') return resp(200, { sessions: state.sessions });
  if (method === 'POST' && /\/agent$/.test(path)) {
    return state.agentStatus === 200
      ? resp(200, { ok: true, agent: body.value })
      : resp(409, { ok: false, error: 'agent switch refused while drain running' });
  }
  if (method === 'POST' && /\/prompt$/.test(path)) return resp(200, { ok: true, seq: 1 });
  return resp(200, { ok: true });
}

// ── EventSource stub ──────────────────────────────────────────────────────
const ES = [];
class EventSource {
  constructor(url) { this.url = url; this.listeners = {}; this.onerror = null; ES.push(this); }
  addEventListener(kind, fn) { (this.listeners[kind] = this.listeners[kind] || []).push(fn); }
  close() {}
}
const dispatch = (es, kind, data) =>
  (es.listeners[kind] || []).forEach((fn) => fn({ data: JSON.stringify(data) }));

// ── boot the real frontend ────────────────────────────────────────────────
const timers = [];
const sandbox = {
  document, fetch, EventSource, console, URLSearchParams,
  location: { search: '' }, window: {}, navigator: {},
  alert: (m) => ALERTS.push(String(m)), confirm: () => true, prompt: () => '',
  setTimeout: (fn, ms) => { const t = setTimeout(fn, ms); t.unref && t.unref(); timers.push(t); return t; },
  clearTimeout: (t) => clearTimeout(t),
  setInterval: (fn, ms) => { const t = setInterval(fn, ms); t.unref && t.unref(); timers.push(t); return t; },
  clearInterval: (t) => clearInterval(t),
};
runInNewContext(['api', 'sse', 'sessions', 'chat', 'composer', 'questions',
  'queue_panel', 'settings'].map((n) => readFileSync(join(ASSETS, `${n}.js`), 'utf8')).join('\n;\n'), sandbox);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
await sleep(50); // let load-time fetches settle
sandbox.cur = 's1'; // select the mock session (sidebar click equivalent)

// ── assertions ────────────────────────────────────────────────────────────
let failed = 0;
function ok(cond, msg) {
  if (cond) { console.log(`  ok  ${msg}`); } else { failed++; console.error(`FAIL  ${msg}`); }
}
const calls = (method, re) => CALLS.filter((c) => c.method === method && re.test(c.path));

// S1: question closed loop — card renders, option click answers, card clears.
console.log('S1 question loop');
state.questions = [{ id: 'q1', question: 'which db?', options: ['pg', 'mysql'] }];
await sandbox.pollQuestions();
const Q = byId('questions');
let card = qsa(Q, '[data-qid="q1"]')[0];
ok(card, 'question card rendered with data-qid');
ok(qsa(card, '.q-text')[0].textContent.includes('which db?'), 'card shows the question text');
const optPg = qsa(card, 'button').find((b) => b.textContent === 'pg');
ok(!!optPg, 'option buttons rendered');
optPg.onclick();
await sleep(50);
ok(calls('POST', /questions\/q1\/answer$/).length === 1, 'answer POSTed to /questions/q1/answer');
ok(calls('POST', /questions\/q1\/answer$/)[0].body.answer === 'pg', 'answer body carries the option');
ok(qsa(Q, '[data-qid="q1"]').length === 0, 'card removed after answering');
state.questions = [{ id: 'q2', question: 'free text?' }];
await sandbox.pollQuestions();
card = qsa(Q, '[data-qid="q2"]')[0];
qsa(card, '.q-skip')[0].onclick();
await sleep(50);
ok(calls('POST', /questions\/q2\/skip$/).length === 1, 'skip POSTs /questions/q2/skip');
ok(qsa(Q, '[data-qid]').length === 0, 'skip clears the card');

// S2: queue panel — steers first, badge count, reorder, delete.
console.log('S2 queue panel');
state.inputs.steer = [{ seq: 7, prompt: 'steer the run', delivery: 'steer' }];
state.inputs.queue = [
  { seq: 1, prompt: 'first queued task', delivery: 'queue' },
  { seq: 2, prompt: 'second queued task', delivery: 'queue' },
];
await sandbox.refreshQueuePanel(true);
const rows = () => qsa(byId('qp-list'), '.qp-item');
ok(rows().length === 3, 'queue panel lists steer + queue rows');
ok(rows()[0].textContent.includes('steer the run'), 'steer row leads the list');
ok(qsa(rows()[1], '.qp-badge')[0].textContent === 'queue', 'delivery badge rendered');
ok(byId('qcount').textContent === '2', 'qcount badge counts queue inputs only');
ok(qsa(rows()[0], '.qp-move')[0].disabled === true, 'first row up disabled');
qsa(rows()[2], '.qp-move')[0].onclick(); // up on seq 2 → swap with seq 1
await sleep(50);
ok(calls('POST', /inputs\/reorder$/).length === 1, 'reorder POSTed');
ok(rows()[1].textContent.includes('second queued task'), 'rows re-ordered after swap');
qsa(rows()[0], '.qp-del')[0].onclick(); // delete the steer row (seq 7)
await sleep(50);
ok(calls('DELETE', /inputs\/7$/).length === 1, 'delete DELETEs /inputs/7');
ok(rows().length === 2, 'row disappears after delete');

// S3: model dropdown — catalog + custom fallback.
console.log('S3 model dropdown');
await sandbox.loadModels();
const sel = byId('model-select');
const vals = sel.children.map((o) => o.value);
ok(JSON.stringify(vals) === JSON.stringify(['a/b', 'x/y', '__custom__']),
  `dropdown options = catalog + custom (${vals.join(',')})`);
sandbox.setModelDisplay('zzz/unknown');
ok(sel.value === '__custom__' && byId('model').style.display === ''
  && byId('model').value === 'zzz/unknown', 'unknown model falls back to free-text input');
sandbox.setModelDisplay('x/y');
ok(sel.value === 'x/y' && byId('model').style.display === 'none', 'known model hides the input');

// S4: composer send path — optimistic echo + prompt POST + busy toggle.
console.log('S4 composer send');
byId('msg').value = 'hello smoke';
await sandbox.send('queue');
ok(calls('POST', /\/prompt$/)[0].body.prompt === 'hello smoke'
  && calls('POST', /\/prompt$/)[0].body.delivery === 'queue', 'prompt POSTed with delivery queue');
ok(qsa(byId('log'), '.m')[0].textContent.includes('hello smoke'), 'optimistic user echo rendered');
ok(byId('send').textContent === 'Interrupt' && sandbox.busy === true, 'busy state flips send button');
dispatch(ES[ES.length - 1], 'done', {});
await sleep(50);
ok(sandbox.busy === false && byId('send').textContent === 'Send', 'done event resets busy');

// S5: mode controls are committed only after server confirmation and stay
// disabled throughout a running drain.
console.log('S5 running mode gate');
sandbox.mode = 'act';
sandbox.updateModeDisplay();
sandbox.setBusy(true);
ok(byId('mode').disabled && byId('handoff').disabled, 'mode and handoff disabled while busy');
byId('msg').value = '/plan later';
const beforeBusyPrompt = calls('POST', /\/prompt$/).length;
await sandbox.send('queue');
ok(calls('POST', /\/prompt$/).length === beforeBusyPrompt, 'busy text mode command sends no request');
ok(byId('msg').value === '/plan later', 'busy text mode command preserves composer input');
byId('msg').value = '';
const beforeBusySwitch = calls('POST', /\/agent$/).length;
const beforeBusyHandoff = calls('POST', /\/handoff$/).length;
byId('mode').value = 'plan';
await sandbox.switchAgent();
await sandbox.handoffSession();
ok(calls('POST', /\/agent$/).length === beforeBusySwitch, 'busy mode switch sends no request');
ok(calls('POST', /\/handoff$/).length === beforeBusyHandoff, 'busy handoff sends no request');
ok(sandbox.mode === 'act' && byId('mode').value === 'act', 'busy select rolls back to committed mode');
sandbox.setBusy(false);
state.agentStatus = 409;
byId('mode').value = 'plan';
await sandbox.switchAgent();
ok(sandbox.mode === 'act' && byId('mode').value === 'act', 'server rejection preserves committed mode');
state.agentStatus = 200;
byId('mode').value = 'plan';
await sandbox.switchAgent();
ok(sandbox.mode === 'plan' && byId('mode').value === 'plan', 'successful switch commits server mode');
ok(!byId('mode').disabled && !byId('handoff').disabled, 'idle controls re-enabled after request');

// S6: SSE reconnect — badge on error, resume from /seq, reset on event,
// persistent banner after max attempts.
console.log('S6 sse reconnect');
const badge = byId('reconnect');
const es1 = ES[ES.length - 1];
es1.onerror();
ok(badge.style.display === '', 'reconnect badge visible on stream error');
await sleep(1300); // backoff 1s → /seq → reopen with ?after=
const es2 = ES[ES.length - 1];
ok(es2 !== es1 && es2.url.includes('after=5'), `reopened from persisted seq (${es2.url})`);
dispatch(es2, 'text_delta', { text: 'x' });
ok(badge.style.display === 'none', 'badge hidden once events flow again');
sandbox.sseAttempts = 5; // white-box: jump to the last allowed attempt
es2.onerror();
ok(byId('reconnect-fail').style.display === '', 'persistent fail banner after max attempts');

console.log(failed === 0 ? 'FRONTEND SMOKE PASSED' : `FRONTEND SMOKE FAILED (${failed})`);
process.exit(failed === 0 ? 0 : 1);
