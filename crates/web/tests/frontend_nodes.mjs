#!/usr/bin/env node
// Headless runtime acceptance for the Phase-4 nodes panel: loads the REAL
// asset scripts (all of them + nodes_panel.js) into a vm via dom_shim and
// asserts — registry render (busy/lost dots + busy task id), dispatch form
// POST shape + live-view switch with ?after=0&token= URLs, TextDelta /
// tool timeline accumulation, terminal done/error/cancelled badges with an
// explicitly CLOSED EventSource, cancel POST path, and history-driven
// replay reopening a fresh stream at after=0. Exit 0 = all assertions pass.

import { createShim, reporter, sleep } from './dom_shim.mjs';

const state = {
  nodesList: [],
  nodeTasks: {},
  dispatchResp: {},
};
const {
  byId, qsa, document, state: sharedState, calls, sandbox, ES, dispatchSse,
} = await createShim({
  names: ['api', 'sse', 'sessions', 'chat', 'composer', 'questions',
    'queue_panel', 'subagent_view', 'bg_panel', 'settings', 'nodes_panel'],
  locationSearch: '?token=t0k-123',
});

// Extend the SHARED backend only where it has no say yet (fleet endpoints).
sharedState.router = async ({ method, path }) => {
  if (method === 'GET' && path === '/api/nodes') {
    return { ok: true, status: 200, json: async () => ({ nodes: state.nodesList }) };
  }
  let m;
  if (method === 'GET' && (m = /^\/api\/nodes\/([^/]+)\/tasks$/.exec(path))) {
    return { ok: true, status: 200, json: async () => ({ tasks: state.nodeTasks[m[1]] || [] }) };
  }
  if (method === 'POST' && /^\/api\/nodes\/[^/]+\/tasks$/.test(path)) {
    return { ok: true, status: 200, json: async () => state.dispatchResp };
  }
  return undefined; // everything else: default mock behaviour
};

// Panel internals nest below the two top-level containers; byId only sees
// body's direct children, so bind them through querySelector instead.
const q = (sel) => document.querySelector(sel);

const R = reporter('FRONTEND NODES');
const ok = (c, m) => R.ok(c, m);
const lastEs = () => ES[ES.length - 1];

function stageTasks(nodeId, list) {
  state.nodeTasks[nodeId] = list;
}

// S1: registry render — busy + lost rows paint computed status affordances.
console.log('S1 registry render');
state.nodesList = [
  { id: 'n-busy', name: 'spark', version: '9.9.9-dev', workdir: '/repo/a',
    status: 'busy', last_seen_at: Date.now(), last_task_id: 'ab12cd34ef56gh78' },
  { id: 'n-lost', name: 'ghost', version: null, workdir: null,
    status: 'lost', last_seen_at: Date.now() - 900000 },
];
await sandbox.toggleNodesPanel();
await sleep(60);
ok(byId('nodes-panel').style.display === '', 'opening the panel makes it visible');
let rows = qsa(q('#np-nodes'), '.np-row');
ok(rows.length === 2, 'two registered nodes rendered');
const dotOf = (row) => qsa(row, '.np-dot')[0];
ok(dotOf(rows[0])._cls.has('busy'), 'busy node paints the orange dot');
ok(dotOf(rows[1])._cls.has('lost'), 'lost node paints the red-grey dot');
ok(qsa(rows[0], '.np-meta')[0].textContent === 'ab12cd34', 'busy row surfaces first 8 chars of its task id');
ok(qsa(rows[1], '.np-name')[0].textContent === 'ghost', 'node name rendered');
ok(qsa(rows[1], '.np-del').length === 1, 'every row carries a remove affordance');
rows[0].onclick();
await sleep(60);
const reRows = qsa(q('#np-nodes'), '.np-row');
ok(reRows[0]._cls.has('active'), 'clicking a row flags it active');
qsa(rows[1], '.np-del')[0].onclick();        // shim confirm() answers true
await sleep(60);
ok(calls('DELETE', /\/api\/nodes\/n-lost$/).length === 1, 'remove confirms then DELETEs the node');

// S2: select + dispatch form — POST shape is exactly the protocol body.
console.log('S2 dispatch form');
qsa(q('#np-nodes'), '.np-row')[0].onclick();   // select "spark"
await sleep(60);
ok(qsa(q('#np-form'), '#np-prompt').length === 1, 'prompt textarea appears for the selected node');
ok(qsa(q('#np-form'), '#np-agent').length === 1, 'optional agent input appears');
const modelSel = q('#np-modelsel');
ok(modelSel.children.length >= 3, 'model dropdown pre-seeded (default + catalog)');
q('#np-prompt').value = 'build the widget';
modelSel.value = 'x/y';
state.dispatchResp = { task_id: 'task-777', session_id: 'sess-777' };
q('#np-dispatch').onclick();
await sleep(80);
const post = calls('POST', /\/api\/nodes\/n-busy\/tasks$/)[0];
ok(!!post, 'dispatch POSTed to /api/nodes/:id/tasks');
ok(post.body.prompt === 'build the widget' && post.body.model === 'x/y' && !('agent' in post.body),
  'POST body carries prompt+model and omits empty agent');
ok(byId('nodes-live').style.display === '' && byId('nodes-panel').style.display === 'none',
  'successful dispatch switches to the live view');
const esUrl = lastEs().url;
ok(esUrl.indexOf('/api/nodes/tasks/task-777/events?after=0&token=t0k-123') === 0,
  `live EventSource opens at after=0 with query token (${esUrl})`);

// S3: streaming frames — text merges, tools build a timeline.
console.log('S3 live frames');
const es3 = lastEs();
dispatchSse(es3, 'text_delta', { text: 'Hello ' });
dispatchSse(es3, 'text_delta', { text: 'world' });
dispatchSse(es3, 'tool_start', { id: 't1', name: 'bash' });
await sleep(20);
const liveBody = () => q('#np-live-body');
const texts = qsa(liveBody(), '.np-text');
ok(texts.length === 1 && texts[0].textContent === 'Hello world',
  'consecutive text deltas merge into one block');
ok(qsa(liveBody(), '.np-tool').length === 1, 'tool start appends one timeline entry');
ok(qsa(liveBody(), '.np-tool')[0].textContent.indexOf('bash') > 0, 'timeline entry shows the tool name');
dispatchSse(es3, 'tool_end', { id: 't1', name: 'bash' });
await sleep(20);
ok(qsa(liveBody(), '.np-tool').length === 1, 'matching tool end reuses the entry (no duplicate strip)');
ok(qsa(liveBody(), '.np-dur').length === 1, 'closed tool carries a duration chip');
dispatchSse(es3, 'text_delta', { text: '\nafter tool\n' });
await sleep(20);
ok(texts.length === 1 && texts[0].textContent.startsWith('Hello world'),
  'post-tool text continues the same transcript block');

// S4: terminal closure frame — ok badge, stream explicitly closed.
console.log('S4 done badge');
ok(lastEs().closed === false, 'stream still open before the closure frame');
dispatchSse(es3, 'done', { ok: true, error: null, task_id: 'task-777' });
await sleep(20);
let badge = q('#np-live-badge');
ok(badge._cls.has('ok') && badge.textContent === 'completed', 'closure done(ok:true) paints the completed badge');
ok(es3.closed === true && es3.readyState === 2, 'panel closed its EventSource at the terminal frame');
ok(q('#np-cancel').style.display === 'none', 'cancel button hidden after terminal frame');

// S5: mid-run error keeps reading; error closure paints .err and closes.
console.log('S5 error badges');
sandbox.openNodeTaskLive('task-e1', 'n-lost');
await sleep(30);
const es5 = lastEs();
ok(es5 !== es3 && es5.url.indexOf('/api/nodes/tasks/task-e1/events?after=0&token=t0k-123') === 0,
  'a fresh task reopens a fresh EventSource at after=0');
dispatchSse(es5, 'error', { error: 'mid-run hiccup' });
await sleep(20);
ok(qsa(liveBody(), '.np-error-line').length === 1,
  'non-closure error frame renders as a red line without closing');
ok(lastEs().closed === false && q('#np-live-badge').textContent === 'running',
  'mid-run error does not terminate the view');
dispatchSse(es5, 'error', { task_id: 'task-e1', ok: false, error: 'kaboom' });
await sleep(20);
badge = q('#np-live-badge');
ok(badge._cls.has('err') && badge.textContent === 'failed', 'error closure paints the failed (.err) badge');
ok(es5.closed === true, 'error closure also closes the stream');

// S6: cancel path — POST to the right URL, cancelling copy, warn badge.
console.log('S6 cancel flow');
sandbox.openNodeTaskLive('task-c1', 'n-busy');
await sleep(30);
const es6 = lastEs();
const cancelBtn = q('#np-cancel');
ok(cancelBtn.style.display !== 'none' && !cancelBtn.disabled, 'cancel visible while running');
cancelBtn.onclick();
await sleep(50);
const cpost = calls('POST', /\/api\/nodes\/n-busy\/tasks\/task-c1\/cancel$/)[0];
ok(!!cpost, `cancel POSTed to the per-task endpoint`);
ok(cancelBtn.textContent === 'cancelling...' && cancelBtn.disabled,
  'button flips to disabled cancelling copy');
ok(q('#np-live-badge').textContent === 'cancelling', 'badge flips to cancelling while awaiting worker');
dispatchSse(es6, 'done', { task_id: 'task-c1', ok: true, cancel: true });
await sleep(20);
badge = q('#np-live-badge');
ok(badge._cls.has('warn') && badge.textContent === 'cancelled', 'cancelled closure paints the warn badge');
ok(es6.closed === true, 'cancelled closure closes the stream');

// S7: history click — replays through the SAME live path at after=0.
console.log('S7 history replay');
stageTasks('n-busy', [
  { id: 'task-h1', prompt: 'old failed job', status: 'error', created_at: Date.now() - 3600000 },
  { id: 'task-h2', prompt: 'done job', status: 'done', created_at: Date.now() - 60000 },
  { id: 'task-h3', prompt: 'queued job', status: 'pending', created_at: Date.now() },
]);
state.nodesList = [{ id: 'n-busy', name: 'spark', status: 'idle', last_seen_at: Date.now() }];
await sandbox.refreshNodes();
await sleep(40);
const hRows = qsa(q('#np-history-list'), '.np-task-row');
ok(hRows.length === 3, 'history lists every dispatched task (newest first)');
ok(hRows[0].textContent.indexOf('queued job') >= 0, 'newest entry leads');
ok(qsa(hRows[0], '.np-badge')[0]._cls.has('warn'), 'pending badge uses warn tone');
ok(qsa(hRows[1], '.np-badge')[0]._cls.has('ok') &&
   qsa(hRows[2], '.np-badge')[0]._cls.has('err'), 'done→.ok error→.err mapping holds');
const before = ES.length;
hRows[1].onclick();                       // click the finished task
await sleep(40);
ok(ES.length === before + 1, 'history click opened a new EventSource');
ok(lastEs().url.indexOf('/api/nodes/tasks/task-h2/events?after=0&token=t0k-123') === 0,
  `replay stream reopened at after=0 (${lastEs().url})`);

// S8: closing the panel / going back tears everything down.
console.log('S8 teardown');
q('.np-live-hdr').children[0].onclick();   // ← back button out of the live view
await sleep(20);
ok(byId('nodes-live').style.display === 'none' && lastEs().closed === true,
  'back from live stops the task stream');
ok(byId('nodes-panel').style.display === '', 'panel visible again behind');
await sandbox.toggleNodesPanel();          // close
await sleep(20);
ok(byId('nodes-panel').style.display === 'none', 'toggle hides the panel');

R.finish();
