// Real Server + Agent crash containment. All state and processes are fixtures.
// PLATFORM_BIN_DIR=target/debug DAG_TEST_ROOTFS=/path/rootfs node this-file
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');
const { spawn, spawnSync } = require('child_process');

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-node-sigkill-'));
const bin = process.env.PLATFORM_BIN_DIR || path.join(__dirname, '../../target/debug');
const sourceRootfs = process.env.DAG_TEST_ROOTFS;
const token = crypto.randomBytes(24).toString('hex');
const children = [];
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let base;
let mock;
let pausedSupervisor;
let supervisorFreezer;

function start(program, args, cwd, name, options = {}) {
  const logPath = path.join(root, `${name}.log`);
  const log = fs.openSync(logPath, 'a');
  const child = spawn(program, args, {
    cwd, detached: options.detached || false, stdio: ['ignore', log, log],
    env: { ...process.env, HOME: root, XDG_CONFIG_HOME: path.join(root, 'config'), XDG_DATA_HOME: path.join(root, 'data') },
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
    await pause(100);
  }
  throw new Error(`timeout: ${label}${last ? `: ${last.message}` : ''}`);
}

async function api(method, route, body) {
  const text = body === undefined ? undefined : JSON.stringify(body);
  const response = await fetch(base + route, {
    method, signal: AbortSignal.timeout(15000), body: text,
    headers: { authorization: `Bearer ${token}`, ...(text ? { 'content-type': 'application/json' } : {}) },
  });
  const raw = await response.text();
  const value = JSON.parse(raw);
  assert(response.ok, `${method} ${route}: ${response.status} ${raw}`);
  return value;
}

async function stop(child, signal = 'SIGTERM') {
  if (!child || child.exitCode !== null || child.signalCode) return;
  child.kill(signal);
  await until(() => child.exitCode !== null || child.signalCode, `stop ${child.pid}`, 10000);
}

function procExists(pid) { return Boolean(pid) && fs.existsSync(`/proc/${pid}`); }
function processIdentity(pid) {
  try {
    const stat = fs.readFileSync(`/proc/${pid}/stat`, 'utf8');
    const fields = stat.slice(stat.lastIndexOf(') ') + 2).trim().split(/\s+/);
    return { pid, state: fields[0], ppid: Number(fields[1]), starttime: fields[19], stat };
  } catch (_) { return null; }
}
function sameProcess(identity) {
  const current = identity && processIdentity(identity.pid);
  return Boolean(current && current.starttime === identity.starttime);
}
function resumeIfAlive(identity) {
  if (!sameProcess(identity)) return;
  try { process.kill(identity.pid, 'SIGCONT'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
}
function fileSize(file) { return fs.statSync(file).size; }
function readPid(file) { return Number(fs.readFileSync(file, 'utf8').trim()); }
function parentPid(pid) {
  const match = fs.readFileSync(`/proc/${pid}/status`, 'utf8').match(/^PPid:\s+(\d+)$/m);
  return match ? Number(match[1]) : 0;
}
function childrenOf(pid) {
  const file = `/proc/${pid}/task/${pid}/children`;
  return fs.existsSync(file) ? fs.readFileSync(file, 'utf8').trim().split(/\s+/).filter(Boolean).map(Number) : [];
}
function runcState(stateRoot, id) {
  const result = spawnSync('runc', ['--root', stateRoot, 'state', id], { encoding: 'utf8' });
  if (result.status !== 0) return null;
  return JSON.parse(result.stdout);
}

function pythonTreeCode(prefix) {
  return `import posix, time\n` +
    `open(STEP_DIR + '/starts', 'a').write('s')\n` +
    `open(STEP_DIR + '/leader.pid', 'w').write(str(posix.getpid()))\n` +
    `p = posix.fork()\n` +
    `if p == 0:\n` +
    `    posix.setsid()\n` +
    `    if posix.fork() > 0: posix._exit(0)\n` +
    `    open(STEP_DIR + '/double.pid', 'w').write(str(posix.getpid()))\n` +
    `    while True:\n` +
    `        open(STEP_DIR + '/ticks', 'a').write('${prefix}d')\n` +
    `        time.sleep(0.02)\n` +
    `while True:\n` +
    `    open(STEP_DIR + '/ticks', 'a').write('${prefix}l')\n` +
    `    time.sleep(0.02)`;
}

async function main() {
  assert(sourceRootfs && fs.statSync(sourceRootfs).isDirectory(), 'set DAG_TEST_ROOTFS to a prepared rootfs');
  assert.equal(spawnSync('runc', ['--version']).status, 0, 'runc must be installed');
  const serverDir = path.join(root, 'server');
  const nodeDir = path.join(root, 'node');
  const stateDir = path.join(nodeDir, 'state');
  fs.mkdirSync(serverDir, { recursive: true });
  fs.mkdirSync(path.join(stateDir, 'dag'), { recursive: true });
  fs.cpSync(sourceRootfs, path.join(stateDir, 'dag/rootfs'), { recursive: true });

  const bashArtifacts = path.join(root, 'bash');
  fs.mkdirSync(bashArtifacts);
  const bashScript = path.join(root, 'bash-tree.py');
  fs.writeFileSync(bashScript, `import os,sys,time\nr=sys.argv[1]\nopen(r+'/leader.pid','w').write(str(os.getpid()))\np=os.fork()\nif p==0:\n os.setsid()\n if os.fork()>0: os._exit(0)\n open(r+'/double.pid','w').write(str(os.getpid()))\n while True:\n  open(r+'/ticks','a').write('d')\n  time.sleep(.02)\nwhile True:\n open(r+'/ticks','a').write('l')\n time.sleep(.02)\n`);
  const bashCommand = `python3 ${bashScript} ${bashArtifacts}`;

  mock = http.createServer(async (request, response) => {
    for await (const _ of request) { /* consume */ }
    const arguments = JSON.stringify({ command: bashCommand });
    const chunks = [
      { choices: [{ index: 0, delta: { role: 'assistant', tool_calls: [{ index: 0, id: 'call-crash', type: 'function', function: { name: 'bash', arguments } }] }, finish_reason: null }] },
      { choices: [{ index: 0, delta: {}, finish_reason: 'tool_calls' }], usage: { prompt_tokens: 10, completion_tokens: 4, total_tokens: 14 } },
    ];
    response.writeHead(200, { 'content-type': 'text/event-stream' });
    for (const chunk of chunks) response.write(`data: ${JSON.stringify(chunk)}\n\n`);
    response.end('data: [DONE]\n\n');
  });
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve));
  fs.mkdirSync(nodeDir, { recursive: true });
  fs.writeFileSync(path.join(nodeDir, 'opencoder.json'), JSON.stringify({
    providers: { fixture: { base_url: `http://127.0.0.1:${mock.address().port}/v1`, api_key: 'fixture-key' } },
    model: 'fixture/model', cache_salt: false,
  }));

  const server = start(path.join(bin, 'opencoder-server'), ['--workdir', serverDir, '--port', '0', '--token', token], serverDir, 'server');
  await until(() => {
    assert.equal(server.exitCode, null, fs.readFileSync(server.logPath, 'utf8'));
    const match = fs.readFileSync(server.logPath, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);
    if (match) { base = match[1]; return true; }
    return false;
  }, 'server listening');
  const agentArgs = ['--remote', base, '--token', token, '--name', 'crash-node', '--workdir', nodeDir, '--data-dir', stateDir, '--max-runs', '4'];
  let agent = start(path.join(bin, 'opencoder-agent'), agentArgs, nodeDir, 'agent-first');
  await until(async () => (await api('GET', '/api/nodes')).nodes.some((node) => node.online && node.snapshot?.ready), 'agent ready');
  const node = (await api('GET', '/api/nodes')).nodes.find((item) => item.online);

  const unrelatedDir = path.join(root, 'unrelated');
  fs.mkdirSync(unrelatedDir);
  const unrelated = start('/bin/sh', ['-c', `while :; do echo u >> ${unrelatedDir}/ticks; sleep .02; done`], root, 'unrelated', { detached: true });
  await until(() => fs.existsSync(path.join(unrelatedDir, 'ticks')), 'unrelated control started');
  const unrelatedIdentity = processIdentity(unrelated.pid);

  const vmId = 'dag-crash-vm';
  const cancelId = 'dag-cancel-crash';
  const runcId = 'dag-crash-runc';
  const agentId = 'agent-crash-bash';
  await api('POST', '/api/executions', { id: agentId, kind: 'agent', node_id: node.id, input: { prompt: 'run the fixture command now' } });
  await api('POST', '/api/executions', { id: vmId, kind: 'dag', node_id: node.id, input: { definition: { name: 'vm-tree', steps: [{ name: 'vm', kind: { type: 'python', code: pythonTreeCode('v') } }] } } });
  await api('POST', '/api/executions', { id: cancelId, kind: 'dag', node_id: node.id, input: { definition: { name: 'cancel-tree', steps: [{ name: 'cancel', kind: { type: 'python', code: pythonTreeCode('c') } }] } } });
  const runcCode = `import os,time\nd='/workspace/context/runc/'\nopen(d+'started','w').write(str(os.getpid()))\np=os.fork()\nif p==0:\n open(d+'child.pid','w').write(str(os.getpid()))\nwhile True:\n open(d+'ticks','a').write('r')\n time.sleep(.02)`;
  await api('POST', '/api/executions', { id: runcId, kind: 'dag', node_id: node.id, input: { definition: { name: 'runc-tree', steps: [{ name: 'runc', kind: { type: 'python', code: runcCode, sandbox: 'runc' } }] } } });

  const vmDir = path.join(stateDir, 'dag', vmId, 'vm');
  const cancelDir = path.join(stateDir, 'dag', cancelId, 'cancel');
  const runcDir = path.join(stateDir, 'dag', runcId, 'runc');
  const runcRoot = path.join(stateDir, 'dag/bundles', runcId, 'runc/runc-state');
  const containerId = `${runcId}-runc`;
  await until(() => fs.existsSync(path.join(bashArtifacts, 'double.pid')) && fileSize(path.join(bashArtifacts, 'ticks')) > 4, 'bash double-fork running');
  await until(() => fs.existsSync(path.join(vmDir, 'double.pid')) && fileSize(path.join(vmDir, 'ticks')) > 4, 'VM double-fork running');
  await until(() => fs.existsSync(path.join(cancelDir, 'double.pid')) && fileSize(path.join(cancelDir, 'ticks')) > 4, 'cancel-window double-fork running');
  await until(() => fs.existsSync(path.join(runcDir, 'started')) && fileSize(path.join(runcDir, 'ticks')) > 4 && runcState(runcRoot, containerId), 'runc tree running', 90000);

  const cancelLeader = readPid(path.join(cancelDir, 'leader.pid'));
  const cancelSupervisor = parentPid(cancelLeader);
  const cancelSupervisorIdentity = processIdentity(cancelSupervisor);
  assert(cancelSupervisorIdentity, 'cancel-window supervisor must be alive');
  process.kill(cancelSupervisor, 'SIGSTOP');
  pausedSupervisor = cancelSupervisorIdentity;
  // Keep this exact owner frozen across the durable command acknowledgement.
  // OwnedSupervisor::terminate normally wakes a stopped owner so cleanup can
  // finish; this timer creates the narrower crash window where the Node dies
  // after persisting cancel intent but before cleanup completes.
  supervisorFreezer = setInterval(() => {
    if (!sameProcess(cancelSupervisorIdentity)) return;
    try { process.kill(cancelSupervisor, 'SIGSTOP'); } catch (error) { if (error.code !== 'ESRCH') throw error; }
  }, 1);
  const cancelReply = await api('POST', `/api/executions/${cancelId}/commands`, { action: 'cancel', input: {} });
  assert.equal(cancelReply.status, 'cancelling');
  const cancelRecordPath = path.join(stateDir, 'dag', cancelId, 'execution.json');
  const cancelRecord = JSON.parse(fs.readFileSync(cancelRecordPath, 'utf8'));
  assert.equal(cancelRecord.lifecycle.stop_intent, 'cancel', 'cancel intent must be durable before command acknowledgement');
  assert.equal(cancelRecord.assignment.index.status, 'cancelling');
  await until(() => processIdentity(cancelSupervisor)?.state === 'T', 'supervisor remains frozen after durable cancel acknowledgement');
  assert(sameProcess(cancelSupervisorIdentity), 'cancel-window supervisor identity must remain owned before Node crash');

  const ownedPids = [readPid(path.join(bashArtifacts, 'leader.pid')), readPid(path.join(bashArtifacts, 'double.pid')), readPid(path.join(vmDir, 'leader.pid')), readPid(path.join(vmDir, 'double.pid')), cancelSupervisor, cancelLeader, readPid(path.join(cancelDir, 'double.pid'))];
  const state = runcState(runcRoot, containerId);
  assert(state && procExists(state.pid), 'runc init must be alive');
  await until(() => childrenOf(state.pid).length > 0, 'runc child alive');
  ownedPids.push(state.pid, ...childrenOf(state.pid));
  const owned = ownedPids.map(processIdentity);
  assert(owned.every(Boolean), 'every owned process must have a captured starttime');
  fs.writeFileSync(path.join(root, 'owned-before-crash.json'), JSON.stringify({
    agent: processIdentity(agent.pid), cancelSupervisor: cancelSupervisorIdentity, owned,
  }, null, 2));
  const tickFiles = [path.join(bashArtifacts, 'ticks'), path.join(vmDir, 'ticks'), path.join(cancelDir, 'ticks'), path.join(runcDir, 'ticks')];
  const beforeCrash = tickFiles.map(fileSize);
  assert(beforeCrash.every((size) => size > 4));
  const unrelatedBefore = fileSize(path.join(unrelatedDir, 'ticks'));

  agent.kill('SIGKILL');
  await until(() => agent.signalCode === 'SIGKILL', 'agent SIGKILL observed');
  clearInterval(supervisorFreezer);
  supervisorFreezer = undefined;
  resumeIfAlive(cancelSupervisorIdentity);
  pausedSupervisor = undefined;
  try {
    await until(() => owned.every((identity) => !sameProcess(identity)), 'all owned descendants gone', 30000);
  } catch (error) {
    const remaining = owned.filter(sameProcess).map((identity) => processIdentity(identity.pid));
    throw new Error(`${error.message}; remaining=${JSON.stringify(remaining)}`);
  }
  await until(() => !runcState(runcRoot, containerId) && !fs.existsSync(path.join(runcRoot, containerId)), 'runc state deleted', 30000);
  const stable = tickFiles.map(fileSize);
  await pause(400);
  assert.deepEqual(tickFiles.map(fileSize), stable, 'owned tick files must stop changing');
  assert(sameProcess(unrelatedIdentity), 'unrelated process must survive Agent crash');
  assert(fileSize(path.join(unrelatedDir, 'ticks')) > unrelatedBefore, 'unrelated process must keep running');

  agent = start(path.join(bin, 'opencoder-agent'), agentArgs, nodeDir, 'agent-restarted');
  await until(async () => (await api('GET', '/api/nodes')).nodes.some((item) => item.id === node.id && item.online && item.snapshot?.ready), 'same node ready after cleanup');
  for (const [id, kind] of [[agentId, 'agent'], [vmId, 'dag'], [runcId, 'dag']]) {
    await until(async () => (await api('GET', `/api/executions/${id}`)).execution.status === 'interrupted', `${id} interrupted`);
    const index = (await api('GET', '/api/executions')).executions.find((item) => item.id === id);
    assert.equal(index.node_id, node.id);
    assert.equal(index.kind, kind);
  }
  await until(async () => (await api('GET', `/api/executions/${cancelId}`)).execution.status === 'cancelled', `${cancelId} recovers cancelled`);
  assert.equal((await api('POST', `/api/executions/${cancelId}/commands`, { action: 'cancel', input: {} })).status, 'cancelled');
  const rejectedResume = await fetch(base + `/api/executions/${cancelId}/commands`, {
    method: 'POST', body: JSON.stringify({ action: 'resume', input: {} }),
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' },
  });
  assert.equal(rejectedResume.status, 409, 'cancelled execution cannot resume');
  await pause(400);
  assert.deepEqual(tickFiles.map(fileSize), stable, 'restart must not auto-run interrupted executions');
  const starts = path.join(vmDir, 'starts');
  assert.equal(fileSize(starts), 1, 'initial VM attempt must be recorded exactly once');
  assert.equal((await api('GET', '/api/nodes')).nodes.filter((item) => item.id === node.id).length, 1, 'restart must reuse node identity');
  assert(sameProcess(unrelatedIdentity), 'restart cleanup must not kill unrelated process');

  await api('POST', `/api/executions/${vmId}/commands`, { action: 'resume', input: {} });
  await until(() => fileSize(starts) === 2 && fileSize(path.join(vmDir, 'ticks')) > stable[1], 'explicit VM resume starts once');
  await pause(400);
  assert.equal(fileSize(starts), 2, 'one resume command must launch one attempt');
  const resumedPids = [readPid(path.join(vmDir, 'leader.pid')), readPid(path.join(vmDir, 'double.pid'))];
  assert(resumedPids.every(procExists), 'resumed VM tree must be alive');
  const resumedSupervisor = processIdentity(parentPid(resumedPids[0]));
  assert(resumedSupervisor, 'resumed VM supervisor must be alive');
  process.kill(resumedSupervisor.pid, 'SIGSTOP');
  pausedSupervisor = resumedSupervisor;
  await api('POST', `/api/executions/${vmId}/commands`, { action: 'cancel', input: {} });
  await until(async () => (await api('GET', `/api/executions/${vmId}`)).execution.status === 'cancelled', 'resumed VM cancelled');
  await until(() => !sameProcess(resumedSupervisor), 'stopped supervisor cleaned while Agent remains alive');
  pausedSupervisor = undefined;
  await until(() => resumedPids.every((pid) => !procExists(pid)), 'resumed VM descendants gone');
  const resumedStable = fileSize(path.join(vmDir, 'ticks'));
  await pause(300);
  assert.equal(fileSize(path.join(vmDir, 'ticks')), resumedStable, 'cancelled resumed VM must stop');
  assert.equal(fileSize(starts), 2, 'cancel must not launch another attempt');

  await stop(agent);
  await stop(unrelated);
  await stop(server);
  console.log(JSON.stringify({ result: 'PASS', root, node_id: node.id, owned_pids: ownedPids, resumed_pids: resumedPids, stable_ticks: stable }));
}

const deadline = setTimeout(() => {
  for (const child of children) if (child.exitCode === null && !child.signalCode) child.kill('SIGKILL');
  console.error(`acceptance exceeded 240s: ${root}`);
  process.exit(1);
}, 240000);
main().catch((error) => { console.error(error); console.error(`artifacts: ${root}`); process.exitCode = 1; }).finally(async () => {
  if (supervisorFreezer) clearInterval(supervisorFreezer);
  if (pausedSupervisor && sameProcess(pausedSupervisor)) {
    resumeIfAlive(pausedSupervisor);
  }
  for (const child of children.reverse()) await stop(child, 'SIGKILL').catch(() => {});
  if (mock) mock.close();
  clearTimeout(deadline);
});
