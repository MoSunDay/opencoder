// Real browser -> project -> native execution -> live node result.
// Isolated: PLATFORM_BIN_DIR=... node main.js /absolute/rootfs
// Live: node main.js --live URL --token-file FILE --node-id ID --output DIR
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { prepare } = require('./scenario');
const { answer } = require('./model');
const { run } = require('./checks');
const { openBrowser } = require('../browser');
const option = (name) => {
  const index = process.argv.indexOf('--' + name);
  return index < 0 ? undefined : process.argv[index + 1];
};
let h, harness, browser, state, observation;
const control = { mode: 'normal', waiting: [], requests: [] };
const browserRequests = [];
const save = () => fs.writeFileSync(path.join(h.root, 'state.json'), JSON.stringify(state, null, 2), { mode: 0o600 });

async function live(base) {
  assert(option('token-file') && option('node-id') && option('output'), 'live requires token-file, node-id and output');
  const root = path.resolve(option('output'));
  if (!process.argv.includes('--resume')) assert(!fs.existsSync(root), 'use a new evidence directory or --resume');
  fs.mkdirSync(root, { recursive: true, mode: 0o700 });
  const token = fs.readFileSync(option('token-file'), 'utf8').trim();
  const api = async (method, route, body) => {
    const response = await fetch(base + route, { method, signal: AbortSignal.timeout(120000),
      headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body) });
    const value = await response.json();
    assert(response.ok, method + ' ' + route + ': ' + response.status + ' ' + JSON.stringify(value));
    return value;
  };
  const until = async (check, label, timeout = 60000) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) { const value = await check(); if (value) return value; await new Promise((r) => setTimeout(r, 1000)); }
    throw new Error('timeout: ' + label);
  };
  const node = (await api('GET', '/api/nodes')).nodes.find((row) => row.id === option('node-id'));
  assert(node?.online && node.snapshot?.ready, 'chosen node must be online and ready');
  const errors = [];
  const opened = await openBrowser({ base, token, root }, errors);
  browser = opened.browser;
  return { live: true, page: opened.page, root, api, until, errors, nodeId: node.id, nodeName: node.name };
}

async function main() {
  let modelConfig = option('model-config')
    ? JSON.parse(fs.readFileSync(option('model-config'), 'utf8')) : undefined;
  assert(!(option('live') && modelConfig), 'model-config is for the isolated Server and Node');
  if (process.argv.includes('--observe-model')) {
    assert(modelConfig, 'observe-model requires an isolated real model-config');
    observation = await require('./provenance').observe(modelConfig); modelConfig = observation.config;
  }
  if (option('live')) h = await live(option('live'));
  else {
    harness = require('../../todo_workbench/harness');
    h = await harness.open(async (prompt) => {
      control.requests.push(prompt);
      if (control.mode === 'hold') await new Promise((resolve) => control.waiting.push(resolve));
      if (control.mode === 'fail') throw new Error('injected model failure');
      if (control.mode === 'empty') return '';
      return answer(prompt);
    }, { dag: true, agents: true, modelConfig, contextLimit: modelConfig ? undefined : 256000,
      rootfs: option('rootfs') || (process.argv[2] && !process.argv[2].startsWith('--')
        ? process.argv[2] : process.env.DAG_TEST_ROOTFS) });
    h.realModel = Boolean(modelConfig);
    h.nodeName = (await h.api('GET', '/api/nodes')).nodes.find((node) => node.id === h.nodeId).name;
  }
  console.log(JSON.stringify({ stage: 'ready', evidence: h.root, live: Boolean(h.live) }));
  h.page.on('response', (response) => {
    const route = new URL(response.url()).pathname;
    if (route.startsWith('/api/')) {
      const item = { route, status: response.status(), method: response.request().method() };
      browserRequests.push(item);
      if (/\/project\/todos\/[^/]+\/executions$/.test(route)) response.json().then((body) => { item.body = body; }).catch((error) => { item.error = error.message; });
    }
  });
  const health = await h.api('GET', '/api/health');
  const served = Buffer.from(await fetch(new URL('/static/app.js', h.page.url())).then((response) => {
    assert(response.ok, 'SPA asset must be readable'); return response.arrayBuffer();
  }));
  const digest = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
  if (!h.live) assert.equal(digest(served), digest(fs.readFileSync(
    path.join(__dirname, '../../../../crates/web/spa/dist/static/app.js'))), 'Server must serve the current SPA');
  if (process.argv.includes('--resume')) state = JSON.parse(fs.readFileSync(path.join(h.root, 'state.json')));
  else {
    state = { version: health, spaSha256: digest(served), live: Boolean(h.live),
      modelTransport: h.live || h.realModel ? 'real' : 'fixture',
      ...(modelConfig ? { configuredModel: modelConfig.model } : {}),
      ...(observation ? { modelProvenance: observation.ledger } : {}),
      scenario: await prepare(h.api, 'pcap-' + crypto.randomBytes(6).toString('hex')) };
    save();
  }
  assert.deepEqual(health, state.version, 'server changed since the case was created');
  assert.equal(digest(served), state.spaSha256, 'SPA changed since the case was created');
  await run(h, state, save);
  if (!h.live && !h.realModel) {
    state.errors = await require('./errors').verify(h, state, control);
    state.modelRequests = control.requests.length;
  }
  assert.deepEqual(h.errors, [], 'browser exceptions');
  assert.deepEqual(await h.api('GET', '/api/health'), state.version, 'server version changed during acceptance');
  if (observation) {
    assert(observation.ledger.requests.length > 0);
    assert(observation.ledger.requests.every((row) => row.httpStatus === 200 && row.complete && row.responseModels.length),
      'real model responses must report their actual model and finish successfully');
  }
  state.result = 'PASS'; save();
  fs.writeFileSync(path.join(h.root, 'report.json'), JSON.stringify(state, null, 2), { mode: 0o600 });
  console.log(JSON.stringify({ result: 'PASS', evidence: h.root, executions: state.executions }));
}
main().catch(async (error) => {
  console.error(error);
  if (h) {
    fs.writeFileSync(path.join(h.root, 'failure.json'), JSON.stringify({ error: error.stack, browserErrors: h.errors, requests: browserRequests }, null, 2));
    try { await h.page.screenshot({ path: path.join(h.root, 'failure.png'), animations: 'disabled' }); } catch {}
    try { fs.writeFileSync(path.join(h.root, 'failure.html'), await h.page.content()); } catch {}
  }
  process.exitCode = 1;
}).finally(async () => {
  control.mode = 'normal'; for (const release of control.waiting) release();
  if (browser) await browser.close();
  if (harness) await harness.close();
  if (observation) await observation.close();
});
