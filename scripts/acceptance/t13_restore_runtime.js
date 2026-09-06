// Independent T13 live backup/restore acceptance: one real Server and Node.
const { spawn, spawnSync } = require('child_process');
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-t13-restore-'));
const repo = path.resolve(__dirname, '../..');
const bin = process.env.PLATFORM_BIN_DIR || path.join(repo, 'target/debug');
const token = crypto.randomBytes(24).toString('hex');
const processes = [];
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let serverUrl;
let modelDelay = 0;
let mockServer;

async function until(check, label, timeout = 40_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (await check()) return;
    await pause(100);
  }
  throw new Error(`timeout: ${label}`);
}

function start(binary, args, cwd, label) {
  const logPath = path.join(root, `${label}.log`);
  const log = fs.openSync(logPath, 'w');
  const child = spawn(path.join(bin, binary), args, {
    cwd, stdio: ['ignore', log, log],
    env: { ...process.env, HOME: root, XDG_CONFIG_HOME: path.join(root, 'xdg-config'), XDG_DATA_HOME: path.join(root, 'xdg-data') },
  });
  fs.closeSync(log);
  child.logPath = logPath;
  processes.push(child);
  return child;
}

async function normalStop(child) {
  if (child.exitCode !== null || child.signalCode) return;
  child.kill('SIGTERM');
  await until(() => child.exitCode !== null || child.signalCode, `normal stop ${child.pid}`, 30_000);
  assert.equal(child.signalCode, null, `${path.basename(child.spawnfile)} terminated by ${child.signalCode}`);
  assert.equal(child.exitCode, 0, `${path.basename(child.spawnfile)} exited ${child.exitCode}`);
}

async function emergencyStop(child) {
  if (child.exitCode !== null || child.signalCode) return;
  child.kill('SIGKILL');
  await until(() => child.exitCode !== null || child.signalCode, `emergency stop ${child.pid}`, 5_000);
}

async function request(method, route, body) {
  const raw = body === undefined ? '' : JSON.stringify(body);
  const response = await fetch(serverUrl + route, {
    method, signal: AbortSignal.timeout(20_000),
    headers: { Authorization: `Bearer ${token}`, ...(raw ? { 'content-type': 'application/json' } : {}) },
    body: raw || undefined,
  });
  const text = await response.text();
  let value;
  try { value = text ? JSON.parse(text) : null; } catch { value = text; }
  assert(response.ok, `${method} ${route}: ${response.status} ${text.slice(0, 500)}`);
  return value;
}

function lastPrompt(raw) {
  try {
    const messages = JSON.parse(raw).messages || [];
    return [...messages].reverse().find(({ role }) => role === 'user')?.content || '';
  } catch { return raw; }
}

async function startMock() {
  const mock = http.createServer(async (incoming, outgoing) => {
    const chunks = [];
    for await (const chunk of incoming) chunks.push(chunk);
    const prompt = lastPrompt(Buffer.concat(chunks).toString());
    if (modelDelay && String(prompt).includes('RESTORE-INTERRUPT')) await pause(modelDelay);
    const answer = String(prompt).includes('RESTORE-INTERRUPT') ? 'restored answer' : 'initial answer';
    outgoing.writeHead(200, { 'content-type': 'text/event-stream' });
    outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: { role: 'assistant', content: answer }, finish_reason: null }] })}\n\n`);
    outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } })}\n\n`);
    outgoing.end('data: [DONE]\n\n');
  });
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve));
  return mock;
}

function writeConfig(directory, modelPort) {
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(path.join(directory, 'opencoder.json'), JSON.stringify({
    providers: { fixture: { base_url: `http://127.0.0.1:${modelPort}/v1`, api_key: 'fixture' } },
    model: 'fixture/model', cache_salt: false,
  }));
}

async function startPlatform(serverWork, serverData, nodeWork, nodeData, suffix, requireReady = true) {
  const server = start('opencoder-server', ['--workdir', serverWork, '--data-dir', serverData, '--port', '0', '--token', token], serverWork, `server-${suffix}`);
  await until(() => {
    const match = fs.readFileSync(server.logPath, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);
    if (match) serverUrl = match[1];
    return Boolean(match);
  }, `server ${suffix}`);
  const node = start('opencoder-agent', ['--remote', serverUrl, '--token', token, '--name', 'restore-node', '--workdir', nodeWork, '--data-dir', nodeData], nodeWork, `node-${suffix}`);
  await until(async () => (await request('GET', '/api/nodes')).nodes.some((item) => (
    item.online && (!requireReady || item.snapshot?.ready)
  )), `node ${suffix}`);
  return { server, node };
}

function runScript(script, args) {
  const result = spawnSync(path.join(repo, 'scripts/platform', script), args, { cwd: repo, encoding: 'utf-8', timeout: 120_000 });
  assert.equal(result.status, 0, `${script}: ${result.status}\n${result.stdout}\n${result.stderr}`);
}

function treeDigest(directory) {
  const digest = crypto.createHash('sha256');
  const visit = (current) => {
    for (const entry of fs.readdirSync(current, { withFileTypes: true }).sort((a, b) => a.name.localeCompare(b.name))) {
      const file = path.join(current, entry.name);
      digest.update(path.relative(directory, file));
      if (entry.isDirectory()) visit(file);
      else if (entry.isFile()) digest.update(fs.readFileSync(file));
      else if (entry.isSymbolicLink()) digest.update(fs.readlinkSync(file));
    }
  };
  visit(directory);
  return digest.digest('hex');
}

function decodedMessageBytes(page) {
  assert(Array.isArray(page.chunks), 'message page has no chunks');
  return Buffer.concat(page.chunks.map((chunk) => {
    assert.equal(chunk.encoding, 'base64');
    return Buffer.from(chunk.bytes_b64, 'base64');
  }));
}

async function main() {
  mockServer = await startMock();
  const sourceServerWork = path.join(root, 'source-server-work');
  const sourceNodeWork = path.join(root, 'source-node-work');
  const sourceServerData = path.join(root, 'source-server-data');
  const sourceNodeData = path.join(root, 'source-node-data');
  writeConfig(sourceServerWork, mockServer.address().port);
  writeConfig(sourceNodeWork, mockServer.address().port);
  const source = await startPlatform(sourceServerWork, sourceServerData, sourceNodeWork, sourceNodeData, 'source');
  const nodeId = (await request('GET', '/api/nodes')).nodes[0].id;
  const id = 'agent-restore-proof';
  await request('POST', '/api/executions', { id, kind: 'agent', target: 'act', node_id: nodeId, input: { prompt: 'RESTORE-INITIAL' } });
  await until(async () => (await request('GET', `/api/executions/${id}`)).execution.status === 'idle', 'initial idle');
  modelDelay = 10_000;
  await request('POST', `/api/executions/${id}/commands`, { action: 'prompt', input: { prompt: 'RESTORE-INTERRUPT' } });
  await until(async () => (await request('GET', `/api/executions/${id}`)).execution.status === 'running', 'running before interrupt');
  await request('POST', `/api/executions/${id}/commands`, { action: 'interrupt', input: {} });
  await until(async () => (await request('GET', `/api/executions/${id}`)).execution.status === 'interrupted', 'interrupted');
  const frozen = await request('POST', '/api/admin/drain');
  assert.equal(frozen.server.mode, 'frozen');
  await until(async () => (await request('GET', '/api/admin/drain')).drained === true, 'cluster drained');
  await normalStop(source.node);
  await normalStop(source.server);

  const backup = path.join(root, 'backup');
  runScript('backup.sh', ['--server-data', sourceServerData, '--node', `restore-node=${sourceNodeData}`, '--output', backup]);
  const sourceDigest = treeDigest(sourceServerData) + treeDigest(sourceNodeData);
  const restored = path.join(root, 'restored');
  runScript('restore.sh', ['--backup', backup, '--output', restored]);

  const restoredServerWork = path.join(root, 'restored-server-work');
  const restoredNodeWork = path.join(root, 'restored-node-work');
  writeConfig(restoredServerWork, mockServer.address().port);
  writeConfig(restoredNodeWork, mockServer.address().port);
  modelDelay = 0;
  const recovered = await startPlatform(
    restoredServerWork,
    path.join(restored, 'server'),
    restoredNodeWork,
    path.join(restored, 'nodes/restore-node'),
    'restored',
    false,
  );
  const detail = await request('GET', `/api/executions/${id}`);
  assert.equal(detail.execution.status, 'interrupted');
  assert.equal(detail.execution.node_id, nodeId);
  await request('DELETE', '/api/admin/drain');
  await until(async () => (await request('GET', '/api/nodes')).nodes.some((item) => (
    item.online && item.snapshot?.ready
  )), 'restored node ready after reopen');
  await request('POST', `/api/executions/${id}/commands`, { action: 'resume', input: {} });
  await until(async () => (await request('GET', `/api/executions/${id}`)).execution.status === 'idle', 'restored resume idle');
  const messages = await request('GET', `/api/executions/${id}/messages`);
  assert(decodedMessageBytes(messages).includes(Buffer.from('restored answer')));
  await request('POST', `/api/executions/${id}/commands`, { action: 'interrupt', input: {} });
  await until(async () => (
    await request('GET', `/api/executions/${id}`)
  ).execution.status === 'interrupted', 'restored execution interrupted');
  await request('POST', '/api/admin/drain');
  await until(async () => (await request('GET', '/api/admin/drain')).drained === true, 'restored cluster drained');
  await normalStop(recovered.node);
  await normalStop(recovered.server);
  assert.equal(treeDigest(sourceServerData) + treeDigest(sourceNodeData), sourceDigest);
  console.log(JSON.stringify({ result: 'PASS', root, execution: id, node_id: nodeId, source_digest: sourceDigest }));
}

const deadline = setTimeout(() => { for (const child of processes) child.kill('SIGKILL'); process.exit(1); }, 300_000);
main().catch((error) => { console.error(error); console.error(`artifacts: ${root}`); process.exitCode = 1; }).finally(async () => {
  for (const child of processes.reverse()) await emergencyStop(child);
  if (mockServer?.listening) await new Promise((resolve) => mockServer.close(resolve));
  clearTimeout(deadline);
});
