// dom_shim.mjs - shared headless DOM shim for the frontend acceptance
// scripts (frontend_smoke.mjs, frontend_nodes.mjs). Extracted verbatim from
// the original in-file shim of frontend_smoke.mjs, plus MINIMAL increments
// needed by later scenes and provably inert for the old ones:
//   * EventSource stub gained readyState tracking (close marks CLOSED) so a
//     scene can assert "the panel closed its stream".
//   * El.classList gained toggle() (no existing asset used it before).
//   * an optional state.router escape hatch lets a scene answer endpoints
//     the shared backend does not know; unset it changes nothing.
// Scene files keep their own assertions + console protocol.

import { readFileSync } from 'node:fs';
import { runInNewContext } from 'node:vm';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const HERE = dirname(fileURLToPath(import.meta.url));
export const ASSETS = join(HERE, '..', 'src', 'assets');

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
      toggle: (c, force) => {
        const on = force === undefined ? !self._cls.has(c) : !!force;
        if (on) { self._cls.add(c); } else { self._cls.delete(c); }
        return on;
      },
    };
  }
  get className() { return [...this._cls].join(' '); }
  set className(v) { this._cls = new Set(String(v).split(/\s+/).filter(Boolean)); }
  get textContent() {
    return this._text + this.children.map((c) => c.textContent).join('');
  }
  set textContent(v) { this._text = String(v); this.children = []; }
  get childNodes() { return this.children; }
  get lastChild() { return this.children.length ? this.children[this.children.length - 1] : null; }
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
  addEventListener(kind, fn) { (this._listeners = this._listeners || {})[kind] = this._listeners[kind] || []; this._listeners[kind].push(fn); }
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

function mkSandboxDom(extraIds = []) {
  const body = new El('body');
  // Hidden-by-default containers mirror their inline style="display:none"
  // attributes in assets/index.html.
  const HIDDEN = ['reconnect', 'reconnect-fail', 'nodes-panel', 'nodes-live'];
  const SKELETON_IDS = ['side', 'search', 'sess-list', 'cur-id', 'mode', 'model',
    'model-select', 'gear', 'settings-pop', 'annotation', 'autopilot', 'handoff', 'reconnect',
    'reconnect-fail', 'log-wrap', 'log', 'hero', 'questions', 'composer',
    'skill-chip', 'msg', 'img-preview', 'skill-pop', 'send', 'qpanel', 'qp-list',
    'qcount', 'qtoggle', 'top', 'main', 'bg-list'];
  for (const id of [...SKELETON_IDS, ...extraIds]) {
    const el = new El('div', id);
    if (HIDDEN.includes(id)) { el.style.display = 'none'; }
    body.appendChild(el);
  }
  const byId = (id) => body.children.find((c) => c.id === id) || null;
  const document = {
    hidden: false,
    body,
    getElementById: (id) => byId(id),
    querySelector: (sel) => qsa(body, sel)[0] || null,
    querySelectorAll: (sel) => qsa(body, sel),
    createElement: (t) => new El(t),
    addEventListener: () => {},
  };
  return { body, byId, document };
}

// ── mock fetch backend ────────────────────────────────────────────────────
// One shared mutable `state` (scene-preconfigurable); every call recorded in
// CALLS BEFORE routing so scenes can assert exact method/path/body shapes.
function mkFetch(state) {
  const CALLS = [];
  async function fetch(url, opts = {}) {
    const path = url.split('?')[0];
    const method = (opts.method || 'GET').toUpperCase();
    let body = null;
    if (opts.body) { body = JSON.parse(opts.body); }
    CALLS.push({ method, path, body });
    if (state.router) {
      const handled = await state.router({ url, method, path, body });
      if (handled !== undefined) { return handled; }
    }
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
    if (method === 'GET' && (m = /^\/api\/sessions\/([^/]+)\/subagents$/.exec(path))) {
      return resp(200, { tasks: state.subagents });
    }
    if (method === 'GET' && (m = /^\/api\/sessions\/([^/]+)\/messages$/.exec(path))) {
      return resp(200, { messages: state.messagesBySession[m[1]] || [], meta: {} });
    }
    if (method === 'GET' && path === '/api/bg') return resp(200, { processes: state.bgProcs });
    if (method === 'POST' && path === '/api/bg/stop') {
      const killed = state.bgProcs.length;
      state.bgProcs = [];
      return resp(200, { ok: true, killed });
    }
    if (method === 'GET' && path === '/api/sessions') return resp(200, { sessions: state.sessions });
    if (method === 'POST' && /\/agent$/.test(path)) {
      return state.agentStatus === 200
        ? resp(200, { ok: true, agent: body.value })
        : resp(409, { ok: false, error: 'agent switch refused while drain running' });
    }
    if (method === 'POST' && /\/prompt$/.test(path)) return resp(200, { ok: true, seq: 1 });
    return resp(200, { ok: true });
  }
  const calls = (method, re) => CALLS.filter((c) => c.method === method && re.test(c.path));
  return { CALLS, fetch, calls };
}

function resp(status, j) {
  return { ok: status >= 200 && status < 300, status, json: async () => j };
}

// ── EventSource stub ──────────────────────────────────────────────────────
// readyState: 0 CONNECTING 1 OPEN 2 CLOSED. `closed` mirrors close() so a
// scene can assert teardown without poking the numeric constant.
class MockEventSource {
  constructor(url) {
    this.url = url;
    this.listeners = {};
    this.onerror = null;
    this.readyState = 1;
    this.closed = false;
    ES.push(this);
  }
  addEventListener(kind, fn) { (this.listeners[kind] = this.listeners[kind] || []).push(fn); }
  close() { this.closed = true; this.readyState = 2; }
}
const ES = [];
const dispatchSse = (es, kind, data) =>
  (es.listeners[kind] || []).forEach((fn) => fn({ data: JSON.stringify(data) }));

// ── one harness per scene file ────────────────────────────────────────────
// Loads the REAL asset scripts into a vm sandbox wired to the shim. `names`
// is the script load order (classic-script concatenation), `locationSearch`
// seeds ?token= plumbing for apiUrl/apiGet.
export async function createShim({ names, locationSearch = '' } = {}) {
  const { body, byId, document } = mkSandboxDom(['nodes-panel', 'nodes-live', 'ntoggle']);
  const state = {
    questions: [], inputs: { steer: [], queue: [] }, seq: 5,
    models: { default: 'a/b', models: ['a/b', 'x/y'] },
    sessions: [{ id: 's1', title: 'smoke session', agent: 'act', updated_at: Date.now() }],
    agentStatus: 200,
    subagents: [],
    bgProcs: [],
    messagesBySession: {},
    router: null,
  };
  const { CALLS, fetch, calls } = mkFetch(state);
  const ALERTS = [];
  const timers = [];
  const sandbox = {
    document, fetch, EventSource: MockEventSource, console, URLSearchParams,
    location: { search: locationSearch }, window: {}, navigator: {},
    alert: (m) => ALERTS.push(String(m)), confirm: () => true, prompt: () => '',
    setTimeout: (fn, ms) => { const t = setTimeout(fn, ms); t.unref && t.unref(); timers.push(t); return t; },
    clearTimeout: (t) => clearTimeout(t),
    setInterval: (fn, ms) => { const t = setInterval(fn, ms); t.unref && t.unref(); timers.push(t); return t; },
    clearInterval: (t) => clearInterval(t),
  };
  runInNewContext(names.map((n) => readFileSync(join(ASSETS, `${n}.js`), 'utf8')).join('\n;\n'), sandbox);
  await sleep(50); // let load-time fetches settle
  return { body, byId, document, qsa, El, state, CALLS, calls, ALERTS,
    EventSource: MockEventSource, ES, dispatchSse, sandbox };
}

export function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }

// Shared assertion reporter: identical `ok N..` line protocol across scenes.
export function reporter(tag) {
  let failed = 0;
  return {
    ok(cond, msg) {
      if (cond) { console.log(`  ok  ${msg}`); } else { failed++; console.error(`FAIL  ${msg}`); }
    },
    fail(msg) { failed++; console.error(`FAIL  ${msg}`); },
    finish() {
      console.log(failed === 0 ? `${tag} PASSED` : `${tag} FAILED (${failed})`);
      process.exit(failed === 0 ? 0 : 1);
    },
  };
}
