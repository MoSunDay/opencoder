// Real Server + Agent graceful drain. Every process and file is a fixture.
// PLATFORM_BIN_DIR=target/debug node scripts/acceptance/node_drain.js
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');
const { spawn } = require('child_process');

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-node-drain-'));
const bin = process.env.PLATFORM_BIN_DIR || path.join(__dirname, '../../target/debug');
const token = crypto.randomBytes(24).toString('hex');
const children = [];
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let base;
let mock;

function start(program, args, cwd, name) {
  const logPath = path.join(root, `${name}.log`);
  const log = fs.openSync(logPath, 'a');
  const child = spawn(program, args, {
    cwd, stdio: ['ignore', log, log],
    env: {
      ...process.env,
      HOME: root,
      XDG_CONFIG_HOME: path.join(root, 'config'),
      XDG_DATA_HOME: path.join(root, 'data'),
    },
  });
  fs.closeSync(log);
  child.logPath = logPath;
  children.push(child);
  return child;
}

async function until(check, label, timeout = 60000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    try { if (await check()) return; } catch (error) { last = error; }
    await pause(50);
  }
  throw new Error(`timeout: ${label}${last ? `: ${last.message}` : ''}`);
}

async function request(method, route, body) {
  const text = body === undefined ? undefined : JSON.stringify(body);
  const response = await fetch(base + route, {
    method,
    signal: AbortSignal.timeout(15000),
    body: text,
    headers: {
      authorization: `Bearer ${token}`,
      ...(text ? { 'content-type': 'application/json' } : {}),
    },
  });
  const raw = await response.text();
  return { status: response.status, body: JSON.parse(raw) };
}

async function api(method, route, body) {
  const response = await request(method, route, body);
  assert(response.status >= 200 && response.status < 300,
    `${method} ${route}: ${response.status} ${JSON.stringify(response.body)}`);
  return response.body;
}

async function stop(child, signal = 'SIGTERM') {
  if (!child || child.exitCode !== null || child.signalCode) return;
  child.kill(signal);
  await until(() => child.exitCode !== null || child.signalCode, `stop ${child.pid}`, 15000);
}

function agentArgs(nodeDir, stateDir) {
  return [
    '--remote', base,
    '--token', token,
    '--name', 'drain-node',
    '--workdir', nodeDir,
    '--data-dir', stateDir,
    '--max-runs', '2',
    '--no-dag',
  ];
}

async function main() {
  const serverDir = path.join(root, 'server');
  const nodeDir = path.join(root, 'node');
  const stateDir = path.join(nodeDir, 'state');
  fs.mkdirSync(serverDir, { recursive: true });
  fs.mkdirSync(nodeDir, { recursive: true });

  mock = http.createServer(async (incoming, response) => {
    for await (const _ of incoming) { /* consume */ }
    const chunks = [
      { choices: [{ index: 0, delta: { role: 'assistant', content: 'done' }, finish_reason: null }] },
      { choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], usage: { prompt_tokens: 2, completion_tokens: 1, total_tokens: 3 } },
    ];
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    for (const chunk of chunks) response.write(`data: ${JSON.stringify(chunk)}\n\n`);
    response.end('data: [DONE]\n\n');
  });
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve));
  fs.writeFileSync(path.join(nodeDir, 'opencoder.json'), JSON.stringify({
    providers: {
      fixture: {
        base_url: `http://127.0.0.1:${mock.address().port}/v1`,
        api_key: 'fixture-key',
      },
    },
    model: 'fixture/model',
    cache_salt: false,
  }));

  const server = start(
    path.join(bin, 'opencoder-server'),
    ['--workdir', serverDir, '--port', '0', '--token', token],
    serverDir,
    'server',
  );
  await until(() => {
    assert.equal(server.exitCode, null, fs.readFileSync(server.logPath, 'utf8'));
    const match = fs.readFileSync(server.logPath, 'utf8')
      .match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);
    if (!match) return false;
    base = match[1];
    return true;
  }, 'server listening');

  let agent = start(
    path.join(bin, 'opencoder-agent'), agentArgs(nodeDir, stateDir), nodeDir, 'agent-first');
  await until(async () => (await api('GET', '/api/nodes')).nodes
    .some((node) => node.online && node.snapshot?.ready), 'first agent ready');
  const node = (await api('GET', '/api/nodes')).nodes.find((item) => item.online);
  await api('POST', '/api/executions', {
    id: 'agent-drain-idle', kind: 'agent', node_id: node.id, input: { prompt: 'finish once' },
  });
  await until(async () => (await api('GET', '/api/executions/agent-drain-idle'))
    .execution.status === 'idle', 'initial turn idle');

  agent.kill('SIGTERM');
  await until(() => agent.exitCode !== null || agent.signalCode, 'graceful agent exit');
  assert.equal(agent.exitCode, 0, fs.readFileSync(agent.logPath, 'utf8'));
  assert.deepEqual(
    JSON.parse(fs.readFileSync(path.join(stateDir, 'admission.json'), 'utf8')),
    { version: 1, mode: 'frozen' },
  );

  agent = start(
    path.join(bin, 'opencoder-agent'), agentArgs(nodeDir, stateDir), nodeDir, 'agent-restarted');
  await until(async () => (await api('GET', '/api/nodes')).nodes.some((item) =>
    item.id === node.id && item.online && item.snapshot && !item.snapshot.ready
      && item.snapshot.resource_error?.includes('frozen')), 'restart remains frozen');
  await until(async () => (await api('GET', '/api/executions/agent-drain-idle'))
    .execution.status === 'interrupted', 'idle execution durably interrupted');

  const rejected = await request('POST', '/api/executions', {
    id: 'agent-frozen-rejected', kind: 'agent', node_id: node.id, input: { prompt: 'reject' },
  });
  assert.equal(rejected.status, 503);
  await api('POST', '/api/admin/drain', {});
  const reopened = await api('DELETE', '/api/admin/drain');
  assert.equal(reopened.server.mode, 'open');
  assert(reopened.nodes.some((item) => item.node_id === node.id && item.body.mode === 'open'));
  await until(async () => (await api('GET', '/api/nodes')).nodes.some((item) =>
    item.id === node.id && item.online && item.snapshot?.ready), 'explicit reopen ready');

  await stop(agent);
  await stop(server);
}

main().then(() => {
  console.log(`node drain acceptance passed: ${root}`);
}).catch((error) => {
  console.error(error.stack || error);
  process.exitCode = 1;
}).finally(async () => {
  for (const child of children.reverse()) {
    try { await stop(child, 'SIGKILL'); } catch (_) { /* best effort fixture cleanup */ }
  }
  if (mock) await new Promise((resolve) => mock.close(resolve));
});
