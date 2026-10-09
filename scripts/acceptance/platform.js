// Real binaries + bundled SPA + two outbound nodes. Uses only temporary state
// and a loopback LLM fixture. Run after cargo build --workspace and SPA build:
// PLATFORM_BIN_DIR=/path/to/target/debug node scripts/acceptance/platform.js
const { spawn } = require('child_process');
const { chromium } = require('../../crates/web/spa/node_modules/playwright-core');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');
const assert = require('assert/strict');
const { prepareNative } = require('./harness/native');
const { resources } = require('./ui/scenarios/resources');
const { schedules } = require('./ui/scenarios/schedules');
const { teamAnswer, teams } = require('./ui/scenarios/teams');
const { exportsStatus } = require('./ui/scenarios/exports');
const { chat } = require('./ui/scenarios/chat');
const { access } = require('./ui/scenarios/access');
require('./harness/namespace').isolateFixture();

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-platform-browser-'));
const bin = process.env.PLATFORM_BIN_DIR || path.join(__dirname, '../../target/debug');
const token = crypto.randomBytes(24).toString('hex');
const children = [];
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let native;
let browser;
let page;
let mock;
let base;
let llmDelay = 0;
let timedOut = false;
const errors = [];
const llmRequests = [];

async function until(check, label, timeout = 30000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (timedOut) throw new Error('acceptance exceeded its time budget');
    if (await check()) return;
    await pause(150);
  }
  throw new Error(`timeout: ${label}`);
}
async function api(method, route, body) {
  const text = body === undefined ? '' : JSON.stringify(body);
  const response = await fetch(base + route, { signal: AbortSignal.timeout(30000), method, headers: {
    authorization: `Bearer ${token}`, ...(text ? { 'content-type': 'application/json' } : {}),
  }, body: text || undefined });
  const raw = await response.text();
  assert(response.headers.get('content-type')?.includes('application/json'), `${route}: non-JSON response ${response.status}: ${raw.slice(0, 160)}`);
  const value = JSON.parse(raw);
  assert(response.ok, `${method} ${route}: ${response.status} ${JSON.stringify(value)}`);
  return value;
}
function start(name, args, workdir) {
  const log = fs.openSync(path.join(root, `${name}-${children.length}.log`), 'w');
  const child = spawn(path.join(bin, name), args, { cwd: workdir, stdio: ['ignore', log, log], env: {
    ...process.env, HOME: root, XDG_CONFIG_HOME: path.join(root, 'config'), XDG_DATA_HOME: path.join(root, 'data'),
  } });
  fs.closeSync(log);
  child.on('error', (error) => { errors.push(`${name}: ${error.message}`); });
  child.logPath = path.join(root, `${name}-${children.length}.log`);
  children.push(child);
  return child;
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode) return;
  child.kill('SIGTERM');
  const end = Date.now() + 30000;
  while (child.exitCode === null && !child.signalCode && Date.now() < end) await pause(100);
  if (child.exitCode === null && !child.signalCode) {
    child.kill('SIGKILL');
    const killed = Date.now() + 5000;
    while (child.exitCode === null && !child.signalCode && Date.now() < killed) await pause(100);
    assert(child.exitCode !== null || child.signalCode, 'fixture child did not stop');
  }
}
async function main() {
  mock = http.createServer(async (req, res) => {
    let raw = ''; for await (const chunk of req) raw += chunk;
    const request = JSON.parse(raw);
    llmRequests.push(request);
    if (req.url.endsWith('/embeddings')) {
      const inputs = Array.isArray(request.input) ? request.input : [request.input];
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ object: 'list', data: inputs.map((_, index) => ({ object: 'embedding', index, embedding: [1, 0, 0] })), model: request.model, usage: { prompt_tokens: 1, total_tokens: 1 } }));
      return;
    }
    if (llmDelay) await pause(llmDelay);
    res.writeHead(200, { 'content-type': 'text/event-stream' });
    for (const chunk of [
      { choices: [{ index: 0, delta: { role: 'assistant', content: teamAnswer(request) }, finish_reason: null }] },
      { choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], usage: { prompt_tokens: 10, completion_tokens: 4, total_tokens: 14 } },
    ]) res.write(`data: ${JSON.stringify(chunk)}\n\n`);
    res.end('data: [DONE]\n\n');
  });
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve));
  const dirs = ['server', 'node-a', 'node-b'].map((name) => {
    const dir = path.join(root, name);
    fs.mkdirSync(dir);
    fs.writeFileSync(path.join(dir, 'opencoder.json'), JSON.stringify({
      providers: { fixture: { base_url: `http://127.0.0.1:${mock.address().port}/v1`, api_key: crypto.randomBytes(12).toString('hex') } },
      model: 'fixture/model', cache_salt: false,
    }));
    return dir;
  });
  native = await prepareNative(root, dirs[0], dirs.slice(1), process.argv[2]);
  const server = start('opencoder-server', ['--workdir', dirs[0], '--port', '0', '--token', token], dirs[0]);
  await until(async () => {
    if (server.exitCode !== null) throw new Error(`server exited: ${fs.readFileSync(server.logPath, 'utf8')}`);
    const match = fs.readFileSync(server.logPath, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);
    if (match) { base = match[1]; return true; }
    return false;
  }, 'server listening');
  console.log('server ready');
  native.mount();
  for (const dir of dirs.slice(1)) start('opencoder-agent', ['--remote', base, '--token', token, '--name', path.basename(dir), '--workdir', dir, '--data-dir', path.join(dir, 'state')], dir);
  await until(async () => (await api('GET', '/api/nodes')).nodes.filter((n) => n.online && n.snapshot.ready).length === 2, 'registered nodes');
  assert.deepEqual((await api('GET', '/api/executions')).executions, [], 'maintenance must not run automatically');
  console.log('two nodes registered');
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  page = await browser.newPage({ viewport: { width: 1600, height: 1000 } });
  page.setDefaultTimeout(45000);
  page.setDefaultNavigationTimeout(20000);
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => { if (message.type() === 'error') console.error('browser:', message.text()); });
  page.on('response', async (response) => { if (response.url().includes('/api/') && response.status() >= 400) console.error(response.status(), response.url(), await response.text()); });
  await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
  await page.goto(base, { waitUntil: 'networkidle' });
  await page.getByText('node-a', { exact: true }).waitFor();
  await page.getByText('node-b', { exact: true }).waitFor();
  await page.getByRole('columnheader', { name: '活跃 agent loops' }).waitFor();
  await page.getByRole('button', { name: '维护节点' }).first().click();
  await page.getByRole('button', { name: '执行指令', exact: true }).click();
  await page.locator('.ant-modal pre').filter({ hasText: 'maintenance_agent_id' }).waitFor();
  await page.locator('.ant-modal-close').click();
  await page.locator('.fleet-nav-category').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.locator('.ant-empty').waitFor();
  const browserNode = (await api('GET', '/api/nodes')).nodes.find((node) => node.name === 'node-a');
  await api('POST', '/api/executions', { id: 'agent-browser', kind: 'agent', target: 'act', node_id: browserNode.id, input: { prompt: 'run browser acceptance' } });
  await page.getByRole('button', { name: /^刷\s*新$/ }).click();
  await page.getByRole('button', { name: 'agent-browser', exact: true }).waitFor();
  await page.getByRole('button', { name: 'agent-browser', exact: true }).click();
  await page.getByText('browser node-owned answer', { exact: true }).waitFor({ timeout: 30000 });
  await page.screenshot({ path: path.join(root, 'agent-detail.png') });
  const indexes = (await api('GET', '/api/executions')).executions;
  const run = indexes.find((i) => i.id.startsWith('agent-'));
  assert(run);
  assert.deepEqual(Object.keys(run).sort(), ['created_at', 'id', 'kind', 'name', 'node_id', 'status']);
  const detail = await api('GET', `/api/executions/${run.id}`);
  const messages = await api('GET', `/api/executions/${run.id}/messages`);
  assert(messages.chunks.map(chunk => Buffer.from(chunk.bytes_b64, 'base64').toString()).join('').includes('browser node-owned answer'));
  await page.locator('.ant-drawer-close').click();
  await api('POST', '/api/executions', { id: 'dag-browser', kind: 'dag', input: { definition: { name: 'browser-artifact', steps: [{ name: 'python', kind: { type: 'binary', resource: 'stdout' } }] } } });
  await until(async () => (await api('GET', '/api/executions/dag-browser')).execution.status === 'done', 'DAG completion', 120000);
  const artifact = await api('POST', '/api/executions/dag-browser/commands', { action: 'artifact', input: { step: 'python', file: 'output.txt' } });
  assert.equal(Buffer.from(artifact.bytes_b64, 'base64').toString(), 'node artifact\n');
  assert.equal(artifact.eof, true);
  console.log('CASE NFS exports');
  await exportsStatus({ page, api, root, until });
  console.log('CASE native binary resources');
  await resources({ page, api, root, until });
  console.log('CASE native schedules');
  await schedules({ page, api, root, until });
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  const todo = await api('POST', '/api/project/todos', { title: 'browser project', draft: 'draft one' });
  const projectId = `project-${todo.id}`;
  const project = await api('POST', `/api/project/todos/${todo.id}/plan`, {});
  await until(async () => (await api('GET', `/api/executions/${projectId}`)).execution.status === 'idle', 'initial project plan');
  await api('PATCH', `/api/project/todos/${todo.id}`, { draft: 'latest browser draft' });
  await page.getByRole('button', { name: /^刷\s*新$/ }).click();
  await page.getByRole('button', { name: projectId, exact: true }).click();
  const planRequestsBefore = llmRequests.length;
  llmDelay = 1500;
  assert.equal(await page.getByRole('button', { name: '生成计划', exact: true }).count(), 0);
  await api('POST', `/api/project/todos/${todo.id}/plan`, {});
  await until(async () => (await api('GET', `/api/executions/${projectId}`)).execution.status === 'running', 'Plan started');
  await until(async () => (await api('GET', `/api/executions/${projectId}`)).execution.status === 'idle', 'latest project plan');
  llmDelay = 0;
  const planned = await api('GET', `/api/executions/${projectId}`);
  assert.equal(planned.todo.draft, 'latest browser draft');
  assert.equal(planned.execution.node_id, project.node_id);
  await page.getByRole('button', { name: '刷新明细', exact: true }).click();
  await page.getByRole('button', { name: 'v2 · plan · done', exact: true }).waitFor();
  await page.locator('.ant-drawer .ant-tag').filter({ hasText: /^等待继续$/ }).waitFor();
  assert(llmRequests.slice(planRequestsBefore).some(request => JSON.stringify(request).includes('latest browser draft')));
  await page.locator('.ant-drawer').getByText('browser node-owned answer', { exact: true }).waitFor();
  await page.screenshot({ path: path.join(root, 'project-latest-plan.png'), animations: 'disabled' });
  await page.locator('.ant-drawer-close').click();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.locator('.fleet-mobile-nav').getByText('后台管理', { exact: true }).click();
  await page.getByRole('combobox', { name: '页面导航' }).click();
  await page.locator('.ant-select-item-option-content').getByText('节点列表', { exact: true }).click();
  await page.getByText('node-a', { exact: true }).waitFor();
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'mobile page must not overflow');
  await page.getByRole('combobox', { name: '页面导航' }).press('Escape');
  await page.screenshot({ path: path.join(root, 'mobile-nodes.png'), animations: 'disabled' });
  await page.setViewportSize({ width: 1600, height: 1000 });
  await page.locator('.fleet-nav-category').getByText('Agent', { exact: true }).click();
  console.log('CASE Team execution');
  await teams({ page, api, root, until });
  await page.getByRole('menuitem', { name: '大脑调度' }).click();
  await page.getByRole('tab', { name: '计划库', exact: true }).click();
  await page.getByRole('button', { name: '新建计划', exact: true }).waitFor();
  await page.screenshot({ path: path.join(root, 'brain.png') });
  console.log('CASE chat continuation');
  await chat({ page, api, root, until });
  console.log('CASE roles and token administration');
  await access({ page, browser, base, api, root });
  // An offline owner must expose an error without creating a replacement run.

  const owner = (await api('GET', '/api/nodes')).nodes.find((node) => node.id === project.node_id);
  const ownerChild = children.find((child) => child.spawnargs.includes(owner.name));
  const ownerDir = path.join(root, owner.name);
  await api('POST', '/api/executions', { id: 'dag-crash', kind: 'dag', node_id: owner.id,
    input: { definition: { name: 'crash-lifecycle', steps: [{ name: 'loop', kind: {
      type: 'binary', resource: 'spin' } }] } } });
  await until(async () => (await api('GET', '/api/executions/dag-crash')).execution.status === 'running', 'native step running');
  ownerChild.kill('SIGKILL');
  await until(async () => ownerChild.signalCode === 'SIGKILL', 'native owner process exits');
  const restarted = start('opencoder-agent', ownerChild.spawnargs.slice(1), ownerDir);
  await until(async () => (await api('GET', '/api/nodes')).nodes.some((node) => node.id === owner.id && node.online && node.snapshot.ready), 'owner restarted');
  assert.equal((await api('GET', '/api/executions/dag-crash')).execution.status, 'interrupted');
  const count = (await api('GET', '/api/executions')).executions.length;
  await stop(restarted);
  await until(async () => !(await api('GET', '/api/nodes')).nodes.find((node) => node.id === owner.id).online, 'owner disconnected');
  await page.locator('.fleet-nav-category').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.getByRole('combobox', { name: '执行类型筛选' }).click();
  await page.locator('.ant-select-item-option-content').getByText('项目任务', { exact: true }).click();
  await page.getByRole('button', { name: projectId, exact: true }).click();
  await page.locator('.ant-drawer .ant-alert-error').waitFor();
  assert.equal((await api('GET', '/api/executions')).executions.length, count);
  await page.screenshot({ path: path.join(root, 'offline-owner-error.png') });
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ result: 'PASS', nodes: 2, execution_id: run.id, owner: run.node_id, artifacts: root }));
}
const deadline = setTimeout(() => {
  timedOut = true;
  process.exitCode = 1;
  console.error(`acceptance exceeded 600s: ${root}`);
  for (const child of children) child.kill('SIGTERM');
  if (browser) browser.close().catch(console.error);
}, 600000);
main().catch(async (error) => { console.error(error); if (page) { await page.screenshot({ path: path.join(root, 'failure.png') }); console.error(await page.locator('body').innerText()); } console.error(`artifacts: ${root}`); process.exitCode = 1; }).finally(async () => {
  if (browser) await browser.close();
  for (const child of children.reverse()) await stop(child);
  if (native) native.close();
  if (mock) mock.close();
  clearTimeout(deadline);
});
