// Independent UI acceptance for the durable TODO initialization window.
const { spawn, spawnSync } = require('child_process');
const { chromium } = require('../../crates/web/spa/node_modules/playwright-core');
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-todo-init-ui-'));
const bin = process.env.PLATFORM_BIN_DIR || path.join(__dirname, '../../target/debug');
const token = crypto.randomBytes(24).toString('hex');
const children = [];
const browserErrors = [];
const expectedOfflineErrors = [];
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let base;
let browser;
let page;
let mock;
let plannedRestart = false;

async function until(check, label, timeout = 40_000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    try {
      last = await check();
      if (last) return last;
    } catch (error) { last = error.message; }
    await pause(100);
  }
  throw new Error(`timeout: ${label}; last=${last}`);
}

function start(binary, args, cwd, label) {
  const logPath = path.join(root, `${label}.log`);
  const log = fs.openSync(logPath, 'w');
  const child = spawn(path.join(bin, binary), args, {
    cwd, stdio: ['ignore', log, log],
    env: { ...process.env, HOME: root, XDG_CONFIG_HOME: path.join(root, 'config'), XDG_DATA_HOME: path.join(root, 'data') },
  });
  fs.closeSync(log);
  child.logPath = logPath;
  children.push(child);
  return child;
}

async function stop(child, signal = 'SIGTERM') {
  if (child.exitCode !== null || child.signalCode) return;
  child.kill(signal);
  await until(() => child.exitCode !== null || child.signalCode, `stop ${child.pid}`, 40_000);
}

async function request(method, route, body) {
  const raw = body === undefined ? '' : JSON.stringify(body);
  const response = await fetch(base + route, {
    method, signal: AbortSignal.timeout(20_000),
    headers: { Authorization: `Bearer ${token}`, ...(raw ? { 'content-type': 'application/json' } : {}) },
    body: raw || undefined,
  });
  const text = await response.text();
  let value;
  try { value = text ? JSON.parse(text) : null; } catch { value = text; }
  return { response, value };
}

async function api(method, route, body) {
  const { response, value } = await request(method, route, body);
  assert(response.ok, `${method} ${route}: ${response.status} ${JSON.stringify(value).slice(0, 500)}`);
  return value;
}

function latestPrompt(raw) {
  try {
    const messages = JSON.parse(raw).messages || [];
    const message = [...messages].reverse().find(({ role }) => role === 'user');
    if (typeof message?.content === 'string') return message.content;
    if (Array.isArray(message?.content)) return message.content.map((part) => part.text || '').join('\n');
  } catch {}
  return raw;
}

function answer(raw) {
  const prompt = latestPrompt(raw);
  if (prompt.includes('Accept or reject one TODO candidate')) {
    return JSON.stringify({ operation: 'accept', reason: 'accepted by fixture', mark_milestone: false });
  }
  if (prompt.includes('Complete exactly one focused TODO')) {
    return JSON.stringify({
      status: 'candidate', summary: 'VISIBLE-NORMAL-CONTENT', result: 'done',
      verification: 'fixture', evidence_refs: [], recovery_context: { summary: 'done', refs: [] },
    });
  }
  if (prompt.includes('Decide the next workflow operation')) {
    return prompt.includes('"candidate_ready"') || prompt.includes('"passed"')
      ? JSON.stringify({ operation: 'complete', reason: 'done' })
      : JSON.stringify({ operation: 'dispatch', todos: [{ todo_id: 'one', context_mode: 'new' }], reason: 'run' });
  }
  return 'fixture answer';
}

async function startMock() {
  mock = http.createServer(async (incoming, outgoing) => {
    const chunks = [];
    for await (const chunk of incoming) chunks.push(chunk);
    const text = answer(Buffer.concat(chunks).toString());
    outgoing.writeHead(200, { 'content-type': 'text/event-stream' });
    outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: { role: 'assistant', content: text }, finish_reason: null }] })}\n\n`);
    outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } })}\n\n`);
    outgoing.end('data: [DONE]\n\n');
  });
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve));
}

function writeConfig(directory) {
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(path.join(directory, 'opencoder.json'), JSON.stringify({
    providers: { fixture: { base_url: `http://127.0.0.1:${mock.address().port}/v1`, api_key: 'fixture' } },
    model: 'fixture/model', cache_salt: false,
  }));
}

async function holdWriteLock(database) {
  const code = [
    'import sqlite3,sys',
    'db=sqlite3.connect(sys.argv[1],isolation_level=None)',
    'db.execute("PRAGMA busy_timeout=0")',
    'db.execute("BEGIN IMMEDIATE")',
    'print("LOCKED",flush=True)',
    'sys.stdin.read(1)',
    'db.rollback()',
  ].join(';');
  const child = spawn('python3', ['-u', '-c', code, database], { stdio: ['pipe', 'pipe', 'pipe'] });
  children.push(child);
  const ready = new Promise((resolve, reject) => {
    child.stdout.once('data', (bytes) => resolve(bytes.toString()));
    child.once('error', reject);
    child.once('exit', (status) => { if (status) reject(new Error(`lock helper exited ${status}`)); });
  });
  assert.match(await ready, /LOCKED/);
  return async () => {
    child.stdin.end('x');
    await until(() => child.exitCode !== null, 'write lock release', 5_000);
    assert.equal(child.exitCode, 0);
  };
}

function trigger(database, create) {
  const sql = create
    ? `CREATE TRIGGER fixture_todo_init_failure BEFORE INSERT ON sessions
       WHEN NEW.id LIKE 'todo-workflow-%'
       BEGIN SELECT RAISE(ABORT, 'fixture todo initialization failed'); END`
    : 'DROP TRIGGER fixture_todo_init_failure';
  const code = 'import sqlite3,sys; db=sqlite3.connect(sys.argv[1]); db.execute(sys.argv[2]); db.commit()';
  const result = spawnSync('python3', ['-c', code, database, sql], { encoding: 'utf8', timeout: 10_000 });
  assert.equal(result.status, 0, result.stderr);
}

function spec(id) {
  return {
    schema_version: 1, id, name: id, objective: 'verify initialization visibility', constraints: [], metadata: {},
    todos: [{
      id: 'one', title: 'visible item', requirement_background: 'fixture background',
      instructions: 'return the fixture result', depends_on: [], agent: 'act', max_attempts: 1,
      acceptance: { criteria: 'fixture accepts', required_tool_calls: [] }, metadata: {},
    }],
  };
}

async function createTodo(id, nodeId) {
  const value = await api('POST', '/api/executions', { id, kind: 'todos', node_id: nodeId, input: { spec: spec(id) } });
  assert.equal(value.id, id);
  return value;
}

async function openExecution(id) {
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.getByRole('button', { name: /^刷\s*新$/ }).click();
  await page.getByRole('button', { name: id, exact: true }).click();
  const drawer = page.locator('.ant-drawer:visible');
  await drawer.waitFor();
  return drawer;
}

async function closeDrawer(drawer) {
  await drawer.locator('.ant-drawer-close').click();
  await drawer.waitFor({ state: 'hidden' });
}

async function main() {
  await startMock();
  const serverWork = path.join(root, 'server-work');
  const nodeWork = path.join(root, 'node-work');
  const serverData = path.join(root, 'server-data');
  const nodeData = path.join(root, 'node-data');
  writeConfig(serverWork); writeConfig(nodeWork);
  const server = start('opencoder-server', ['--workdir', serverWork, '--data-dir', serverData, '--port', '0', '--token', token], serverWork, 'server');
  await until(() => {
    const match = fs.readFileSync(server.logPath, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);
    if (match) base = match[1];
    return base;
  }, 'server listening');
  const agentArgs = ['--remote', base, '--token', token, '--name', 'todo-init-node', '--workdir', nodeWork, '--data-dir', nodeData, '--no-dag'];
  let agent = start('opencoder-agent', agentArgs, nodeWork, 'node-first');
  const nodeId = await until(async () => (await api('GET', '/api/nodes')).nodes.find((node) => node.online && node.snapshot?.ready)?.id, 'node ready');

  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  page.setDefaultTimeout(15_000);
  page.on('pageerror', (error) => browserErrors.push(error.message));
  page.on('console', (message) => {
    if (message.type() !== 'error') return;
    const text = message.text();
    if (plannedRestart && /^Failed to load resource: the server responded with a status of 503/.test(text)) {
      expectedOfflineErrors.push(text);
    } else {
      browserErrors.push(text);
    }
  });
  await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
  await page.goto(base, { waitUntil: 'networkidle' });
  await page.getByText('todo-init-node', { exact: true }).waitFor();

  const database = path.join(nodeData, 'runtime.db');
  let release = await holdWriteLock(database);
  await createTodo('todos-ui-initializing', nodeId);
  let detail = await api('GET', '/api/executions/todos-ui-initializing');
  assert.equal(detail.workflow_initialization, 'initializing');
  assert.equal(detail.workflow_initializing, true);
  let drawer = await openExecution('todos-ui-initializing');
  await drawer.getByText('TODO 工作流正在初始化', { exact: true }).waitFor();
  assert.equal(await drawer.getByText('0 / 0 完成', { exact: true }).count(), 0);
  await page.screenshot({ path: path.join(root, 'todo-initializing.png'), animations: 'disabled' });
  await release();
  await drawer.getByText(/one · /).waitFor({ timeout: 20_000 });
  detail = await until(async () => {
    const current = await api('GET', '/api/executions/todos-ui-initializing');
    return current.workflow_initialization === 'ready' && current;
  }, 'workflow detail ready');
  assert.equal(detail.workflow.workflow.id, 'todos-ui-initializing');
  await page.screenshot({ path: path.join(root, 'todo-ready.png'), animations: 'disabled' });
  await closeDrawer(drawer);

  await until(async () => (await api('GET', '/api/executions/todos-ui-initializing')).execution.status === 'done', 'normal workflow done');
  trigger(database, true);
  await createTodo('todos-ui-init-failed', nodeId);
  detail = await until(async () => {
    const current = await api('GET', '/api/executions/todos-ui-init-failed');
    return current.workflow_initialization === 'failed' && current;
  }, 'initialization failure');
  assert.match(detail.error, /fixture todo initialization failed/);
  drawer = await openExecution('todos-ui-init-failed');
  await drawer.getByText(/TODO 工作流初始化失败/).waitFor();
  const failureDescription = drawer.locator('.ant-alert-description').filter({ hasText: /fixture todo initialization failed/ });
  assert.equal(await failureDescription.count(), 1);
  await failureDescription.waitFor();
  await page.screenshot({ path: path.join(root, 'todo-init-failed.png'), animations: 'disabled' });
  await closeDrawer(drawer);
  trigger(database, false);

  release = await holdWriteLock(database);
  await createTodo('todos-ui-preinit-interrupt', nodeId);
  drawer = await openExecution('todos-ui-preinit-interrupt');
  await drawer.getByText('TODO 工作流正在初始化', { exact: true }).waitFor();
  const interrupted = request('POST', '/api/executions/todos-ui-preinit-interrupt/commands', { action: 'interrupt', input: {} });
  detail = await until(async () => {
    const current = await api('GET', '/api/executions/todos-ui-preinit-interrupt');
    return ['stopping', 'stopped'].includes(current.workflow_initialization) && current;
  }, 'preinit interrupt is durable');
  const interruptReply = await interrupted;
  assert(interruptReply.response.ok, `interrupt failed: ${interruptReply.response.status} ${JSON.stringify(interruptReply.value)}`);
  await drawer.getByText('TODO 工作流正在停止', { exact: true }).waitFor({ timeout: 10_000 });
  await page.screenshot({ path: path.join(root, 'todo-init-stopping.png'), animations: 'disabled' });
  await release();
  detail = await until(async () => {
    const current = await api('GET', '/api/executions/todos-ui-preinit-interrupt');
    return current.execution.status === 'interrupted'
      && current.workflow_initialization === 'ready'
      && current;
  }, 'preinit interrupt completed its in-flight initialization');
  assert.equal(detail.execution.status, 'interrupted');
  await drawer.getByText(/one · /).waitFor({ timeout: 10_000 });
  await pause(3_500);
  detail = await api('GET', '/api/executions/todos-ui-preinit-interrupt');
  assert.equal(detail.execution.status, 'interrupted');
  assert.equal(detail.workflow_initialization, 'ready');
  await drawer.getByText('已中断', { exact: true }).waitFor({ timeout: 10_000 });
  assert.equal(await drawer.getByText(/TODO 工作流正在(初始化|停止)/).count(), 0);
  await page.screenshot({ path: path.join(root, 'todo-init-interrupted.png'), animations: 'disabled' });
  await closeDrawer(drawer);

  release = await holdWriteLock(database);
  await createTodo('todos-ui-preinit-restart', nodeId);
  drawer = await openExecution('todos-ui-preinit-restart');
  await drawer.getByText('TODO 工作流正在初始化', { exact: true }).waitFor();
  plannedRestart = true;
  await stop(agent, 'SIGKILL');
  await release();
  agent = start('opencoder-agent', agentArgs, nodeWork, 'node-restarted');
  await until(async () => (await api('GET', '/api/nodes')).nodes.find((node) => node.id === nodeId)?.online, 'node restarted');
  detail = await until(async () => {
    const current = await api('GET', '/api/executions/todos-ui-preinit-restart');
    return current.workflow_initialization === 'stopped' && current;
  }, 'preinit restart stop visible');
  assert.equal(detail.execution.status, 'interrupted');
  await drawer.getByText('TODO 工作流未启动', { exact: true }).waitFor({ timeout: 10_000 });
  await pause(3_500);
  assert.equal(await drawer.getByText('TODO 工作流正在初始化', { exact: true }).count(), 0);
  await page.screenshot({ path: path.join(root, 'todo-init-stopped.png'), animations: 'disabled' });

  assert.deepEqual(browserErrors, []);
  console.log(JSON.stringify({ result: 'PASS', root, node_id: nodeId, states: ['initializing', 'ready', 'failed', 'interrupted', 'restart-stopped'], expected_offline_503: expectedOfflineErrors.length }));
}

const deadline = setTimeout(() => { for (const child of children) child.kill('SIGKILL'); process.exit(1); }, 240_000);
main().catch(async (error) => {
  console.error(error); console.error(`artifacts: ${root}`);
  if (page) await page.screenshot({ path: path.join(root, 'failure.png'), animations: 'disabled' });
  process.exitCode = 1;
}).finally(async () => {
  if (browser) await browser.close();
  for (const child of children.reverse()) await stop(child, 'SIGKILL');
  if (mock?.listening) await new Promise((resolve) => mock.close(resolve));
  clearTimeout(deadline);
});
