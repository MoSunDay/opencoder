const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');
const cp = require('node:child_process');
const crypto = require('node:crypto');
const assert = require('node:assert/strict');
const harness = require('../todo_workbench/harness');
const resources = process.argv[2] || '/mnt/opencoder-agents';
assert(path.isAbsolute(resources) && fs.statSync(resources).isDirectory(), 'NFS agent pool must exist');
let runtime;
async function main() {
  const h = await harness.open(() => 'snapshot accepted', { withBrowser: false });
  console.log(JSON.stringify({stage: 'ready', evidence: h.root}));
  const work = path.join(h.root, 'runtime-work');
  const data = path.join(h.root, 'runtime-data');
  fs.mkdirSync(work);
  const config = JSON.parse(fs.readFileSync(path.join(h.root, 'node-work/opencoder.json')));
  config.agent = { ...config.agent, agents_dir: resources };
  fs.writeFileSync(path.join(work, 'opencoder.json'), JSON.stringify(config), { mode: 0o600 });
  const token = crypto.randomBytes(24).toString('hex');
  const tokenFile = path.join(h.root, 'runtime-token');
  fs.writeFileSync(tokenFile, token, { mode: 0o600 });
  const socket = net.createServer();
  await new Promise(resolve => socket.listen(0, '127.0.0.1', resolve));
  const port = socket.address().port;
  await new Promise(resolve => socket.close(resolve));
  const startRuntime = () => {
  const log = fs.openSync(path.join(h.root, 'runtime.log'), 'a');
  runtime = cp.spawn(path.join(process.env.PLATFORM_BIN_DIR, 'opencoder-agent'), ['--workdir', work, '--data-dir', data, '--token-file', tokenFile, 'runtime', '--port', String(port)], {
    stdio: ['ignore', log, log], env: { ...process.env, HOME: h.root, XDG_CONFIG_HOME: path.join(h.root, 'config'), XDG_DATA_HOME: path.join(h.root, 'data') },
  });
  fs.closeSync(log);
  };
  startRuntime();
  const api = async (route, body) => {
    const response = await fetch(`http://127.0.0.1:${port}${route}`, { method: body ? 'POST' : 'GET', signal: AbortSignal.timeout(240000), headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' }, body: body ? JSON.stringify(body) : undefined });
    assert(response.ok, `${route}: ${response.status}`);
    return response.json();
  };
  const inventory = await h.until(() => api('/inventory'), 'runtime ready');
  const assignment = (id) => ({ index: { id, kind: 'agent', created_at: Date.now(), node_id: inventory.registration.id, status: 'pending' }, request: { id, kind: 'agent', target: 'act', input: { prompt: 'Return snapshot accepted without tools.' } } });
  const first = { operation: 'create', assignment: assignment('agent-nfs-replay-accepted') };
  const accepting = api('/rpc', first);
  const firstDirectory = path.join(data, 'agent', first.assignment.index.id);
  await h.until(() => fs.existsSync(firstDirectory) && fs.readdirSync(firstDirectory).some(name => name.startsWith('resources.staging-')), 'same-ID resource snapshot in progress', 30000);
  const duplicate = api('/rpc', first);
  const replies = await Promise.all([accepting, duplicate]);
  for (const accepted of replies) assert.equal(accepted.status, 200, JSON.stringify(accepted));
  assert.equal((await api('/inventory')).indexes.filter(i => i.id === first.assignment.index.id).length, 1);
  console.log(JSON.stringify({stage: 'cold_duplicate_accepted_once', id: first.assignment.index.id}));
  const second = { operation: 'create', assignment: assignment('agent-nfs-replay-cold') };
  const cold = api('/rpc', second);
  const directory = path.join(data, 'agent', second.assignment.index.id);
  await h.until(() => fs.existsSync(directory) && fs.readdirSync(directory).some(name => name.startsWith('resources.staging-')), 'unrelated resource snapshot in progress', 30000);
  const began = performance.now();
  const replay = await api('/rpc', first);
  const seconds = (performance.now() - began) / 1000;
  console.log(JSON.stringify({stage: 'replayed', seconds, evidence: h.root}));
  assert.equal(replay.status, 200);
  assert.equal(replay.body.id, first.assignment.index.id);
  const modules = path.join(data, 'dag/_modules');
  fs.mkdirSync(modules, { recursive: true });
  fs.writeFileSync(path.join(modules, 'quick.wasm'), Buffer.from('0061736d0100000001040160000003020100070a01065f737461727400000a040102000b', 'hex'));
  const fastId = 'dag-nfs-independent-wasi';
  const fast = { operation: 'create', assignment: {
    index: { id: fastId, kind: 'dag', created_at: Date.now(), node_id: inventory.registration.id, status: 'pending' },
    request: { id: fastId, kind: 'dag', input: {} },
    definition: { name: 'independent', steps: [{ name: 'run', kind: { type: 'wasm', command: 'quick.wasm' } }] },
  }};
  const fastStart = performance.now();
  assert.equal((await api('/rpc', fast)).status, 200);
  const newSeconds = (performance.now() - fastStart) / 1000;
  console.log(JSON.stringify({stage: 'new_wasi', seconds: newSeconds}));
  const freezeStart = performance.now();
  assert.equal((await api('/rpc', { operation: 'admission', command: 'freeze' })).status, 200);
  const freezeSeconds = (performance.now() - freezeStart) / 1000;
  console.log(JSON.stringify({stage: 'freeze', seconds: freezeSeconds}));
  const secondReply = await cold;
  assert.equal(secondReply.status, 503, JSON.stringify(secondReply));
  const marker = path.join(directory, 'pending-create.json');
  assert(fs.existsSync(marker));
  const saved = fs.readFileSync(marker, 'utf8');
  assert(!fs.existsSync(path.join(directory, 'execution.json')));
  await h.until(async () => {
    const indexes = (await api('/inventory')).indexes;
    return [first.assignment.index.id, fastId].every(id => indexes.some(i => i.id === id && (i.status === 'done' || (i.kind === 'agent' && i.status === 'idle'))));
  }, 'admitted work terminal before private runtime restart', 120000);
  const stopped = new Promise(resolve => runtime.once('exit', resolve));
  runtime.kill('SIGTERM');
  await stopped;
  startRuntime();
  await h.until(() => api('/inventory'), 'private runtime restarted');
  const admission = await api('/rpc', { operation: 'admission', command: 'status' });
  assert.equal(admission.body.mode, 'frozen');
  assert.equal(fs.readFileSync(marker, 'utf8'), saved);
  assert.equal((await api('/rpc', { operation: 'admission', command: 'reopen' })).status, 200);
  assert.equal((await api('/rpc', second)).status, 200);
  const persisted = JSON.parse(fs.readFileSync(path.join(directory, 'execution.json')));
  assert.deepEqual(persisted.assignment.request.input, second.assignment.request.input);
  assert.equal(persisted.assignment.index.created_at, second.assignment.index.created_at);
  assert(!fs.existsSync(marker));
  assert.equal((await api('/inventory')).indexes.filter(i => i.id === second.assignment.index.id).length, 1);
  await h.until(async () => (await api('/inventory')).indexes.some(i => i.id === second.assignment.index.id && ['done', 'idle'].includes(i.status)), 'recovered request finished', 120000);
  assert(newSeconds < 1, `new WASI blocked by NFS: ${newSeconds}s`);
  assert(freezeSeconds < 1, `admission freeze blocked by NFS: ${freezeSeconds}s`);
  const result = { result: seconds < 1 ? 'PASS' : 'FAIL', replay_seconds: seconds, new_wasi_seconds: newSeconds, freeze_seconds: freezeSeconds, restart_recovery: true, cold_duplicate: true, source: resources, evidence: h.root, ids: [first.assignment.index.id, second.assignment.index.id] };
  fs.writeFileSync(path.join(h.root, 'nfs-replay.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
  assert(seconds < 1, `durable replay stalled behind cold NFS preparation: ${seconds}s`);
}
main().catch(error => { console.error(error.message); process.exitCode = 1; }).finally(async () => { if (runtime) runtime.kill('SIGKILL'); await harness.close(); });
