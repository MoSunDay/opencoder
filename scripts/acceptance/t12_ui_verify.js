// Independent T12 browser acceptance: real server, two nodes and loopback LLM.
const { spawn } = require('child_process');
const { chromium } = require('../../crates/web/spa/node_modules/playwright-core');
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-t12-verify-'));
const bin = process.env.PLATFORM_BIN_DIR || path.join(__dirname, '../../target/debug');
const token = crypto.randomBytes(24).toString('hex');
const children = [];
const errors = [];
const httpFailures = [];
const expectedConsoleErrors = [];
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let base;
let browser;
let page;
let mock;
let delay = 0;
let brainCapability = '';

async function until(check, label, timeout = 40_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (await check()) return;
    await pause(150);
  }
  throw new Error(`timeout: ${label}`);
}

function start(name, args, cwd) {
  const logPath = path.join(root, `${name}-${children.length}.log`);
  const log = fs.openSync(logPath, 'w');
  const child = spawn(path.join(bin, name), args, {
    cwd, stdio: ['ignore', log, log],
    env: { ...process.env, HOME: root, XDG_CONFIG_HOME: path.join(root, 'config'), XDG_DATA_HOME: path.join(root, 'data') },
  });
  fs.closeSync(log);
  child.logPath = logPath;
  children.push(child);
  return child;
}

async function stop(child) {
  if (child.exitCode !== null || child.signalCode) return;
  child.kill('SIGKILL');
  await until(() => child.exitCode !== null || child.signalCode, `stop ${child.pid}`, 5_000);
}

async function request(method, route, body, credential = token) {
  const text = body === undefined ? '' : JSON.stringify(body);
  const response = await fetch(base + route, {
    method, signal: AbortSignal.timeout(30_000),
    headers: { ...(credential === null ? {} : { Authorization: `Bearer ${credential}` }), ...(text ? { 'content-type': 'application/json' } : {}) },
    body: text || undefined,
  });
  const raw = await response.text();
  let value = null;
  try { value = raw ? JSON.parse(raw) : null; } catch { value = raw; }
  return { response, value };
}

async function api(method, route, body) {
  const { response, value } = await request(method, route, body);
  assert(response.ok, `${method} ${route}: ${response.status} ${JSON.stringify(value).slice(0, 400)}`);
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

function answerFor(raw) {
  const prompt = latestPrompt(raw);
  if (prompt.includes('输出一棵严格 JSON') || raw.includes('输出一棵严格 JSON')) {
    return JSON.stringify({ threshold: 0.35, root: { id: 'leaf', kind: 'leaf', capability_id: brainCapability, reason: 'only route' } });
  }
  if (prompt.includes('规划下一轮讨论')) return JSON.stringify({ question: 'TEAM-LARGE question', participants: ['captain'], rationale: 'single member' });
  if (prompt.includes('汇总本轮各成员')) return JSON.stringify({ summary: 'team aligned', aligned: true, ambiguities: [] });
  if (prompt.includes('判断话题是否可以收尾')) return JSON.stringify({ complete: true, next_question: null, final_summary: 'team complete' });
  if (prompt.includes('Accept or reject one TODO candidate')) return JSON.stringify({ operation: 'accept', reason: 'verified', mark_milestone: false });
  if (prompt.includes('Complete exactly one focused TODO')) return JSON.stringify({
    status: 'candidate', summary: 'TODO-LARGE-' + 'T'.repeat(70_000), result: 'done',
    verification: 'fixture', evidence_refs: [], recovery_context: { summary: 'done', refs: [] },
  });
  if (prompt.includes('Decide the next workflow operation')) {
    return prompt.includes('"candidate_ready"') || prompt.includes('"passed"')
      ? JSON.stringify({ operation: 'complete', reason: 'done' })
      : JSON.stringify({ operation: 'dispatch', todos: [{ todo_id: 'todo-large', context_mode: 'new' }], reason: 'run' });
  }
  if (prompt.includes('PROJECT-LARGE')) return 'PROJECT-LARGE-' + 'P'.repeat(70_000);
  if (prompt.includes('TEAM-LARGE')) return 'TEAM-LARGE-' + 'M'.repeat(70_000);
  if (prompt.includes('INTERRUPT-ME')) return 'resumed agent answer';
  return 'browser fresh answer';
}

async function startMock() {
  mock = http.createServer(async (incoming, outgoing) => {
    const parts = [];
    for await (const chunk of incoming) parts.push(chunk);
    const raw = Buffer.concat(parts).toString();
    if (incoming.url?.endsWith('/embeddings')) {
      const input = JSON.parse(raw).input;
      const values = Array.isArray(input) ? input : [input];
      const data = values.map((_, index) => ({ index, embedding: [1, 0.25, 0.5, 0.75] }));
      outgoing.writeHead(200, { 'content-type': 'application/json' });
      outgoing.end(JSON.stringify({ data }));
      return;
    }
    if (delay) await pause(delay);
    const answer = answerFor(raw);
    outgoing.writeHead(200, { 'content-type': 'text/event-stream' });
    outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: { role: 'assistant', content: answer }, finish_reason: null }] })}\n\n`);
    outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], usage: { prompt_tokens: 4, completion_tokens: 4, total_tokens: 8 } })}\n\n`);
    outgoing.end('data: [DONE]\n\n');
  });
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve));
}

async function chooseInForm(label, text) {
  const selectIndex = label === '执行类型' ? 0 : 1;
  await page.getByRole('combobox').nth(selectIndex).click();
  await page.locator('.ant-select-item-option:visible').filter({ hasText: text }).last().click();
}

async function openExecution(id) {
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.getByRole('button', { name: /^刷\s*新$/ }).click();
  await page.getByRole('button', { name: id, exact: true }).click();
  await page.locator('.ant-drawer:visible').waitFor();
  return page.locator('.ant-drawer:visible');
}

async function seedIndexes(nodeIds) {
  for (let startAt = 0; startAt < 55; startAt += 5) {
    await Promise.all(Array.from({ length: 5 }, (_, offset) => {
      const n = startAt + offset;
      return api('POST', '/api/executions', {
        id: `agent-seed-${String(n).padStart(2, '0')}`, kind: 'agent', target: 'act',
        node_id: nodeIds[n % 2], input: { prompt: '', title: `seed ${n}` },
      });
    }));
  }
}

async function verifyLogin() {
  const unauth = await request('GET', '/api/nodes', undefined, null);
  assert.equal(unauth.response.status, 401);
  await page.goto(`${base}/?token=retired-secret`, { waitUntil: 'networkidle' });
  assert.equal(new URL(page.url()).searchParams.has('token'), false);
  await page.getByLabel('共享密钥 (Token)').fill('wrong-token');
  await page.getByRole('button', { name: /连\s*接/ }).click();
  await page.getByText(/连接失败/).waitFor();
  assert.equal(await page.evaluate(() => localStorage.getItem('oc_token')), null);
  await page.getByLabel('共享密钥 (Token)').fill(token);
  await page.getByRole('button', { name: /连\s*接/ }).click();
  await page.getByText('node-a', { exact: true }).waitFor();
  assert.equal(await page.getByText('invalid bearer token', { exact: true }).count(), 0);
}

async function verifyPaging(nodeIds) {
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.getByRole('button', { name: '加载更早的执行' }).click();
  const sixthPage = page.locator('.ant-pagination-item-6');
  await sixthPage.waitFor();
  await sixthPage.click();
  await page.locator('tbody').getByText(/agent-seed-/).first().waitFor();
  await page.locator('.ant-pagination-item-1').click();
  const listed = await api('GET', '/api/executions?limit=1');
  assert.deepEqual(Object.keys(listed.executions[0]).sort(), ['created_at', 'id', 'kind', 'node_id', 'status']);
  await api('POST', '/api/executions', { id: 'agent-poll-new', kind: 'agent', target: 'act', node_id: nodeIds[0], input: { prompt: '', title: 'poll' } });
  await page.getByRole('button', { name: 'agent-poll-new', exact: true }).waitFor({ timeout: 8_000 });
}

async function verifyAgent(nodeName) {
  delay = 2_000;
  await page.getByPlaceholder('act / 定义名称 / 任务 ID').fill('act');
  await page.getByLabel('任务要求').fill('AGENT-FRESH');
  await chooseInForm('调度节点', nodeName);
  await page.getByRole('button', { name: '启动执行' }).click();
  let drawer = page.locator('.ant-drawer:visible');
  await drawer.getByText('运行中', { exact: true }).waitFor();
  await drawer.getByText('browser fresh answer', { exact: true }).waitFor({ timeout: 15_000 });
  await drawer.getByText('等待继续', { exact: true }).waitFor();
  delay = 10_000;
  await drawer.getByPlaceholder('继续会话').fill('INTERRUPT-ME');
  await drawer.getByRole('button', { name: /提\s*交/ }).click();
  await drawer.getByText('运行中', { exact: true }).waitFor();
  await drawer.getByRole('button', { name: '中断（可恢复）' }).click();
  await drawer.getByText('已中断', { exact: true }).waitFor();
  delay = 0;
  await drawer.getByRole('button', { name: '在原节点恢复' }).click();
  await drawer.getByText('等待继续', { exact: true }).waitFor({ timeout: 15_000 });
  await drawer.getByRole('button', { name: '取消（终止）' }).click();
  await drawer.getByText('已取消', { exact: true }).waitFor();
  await page.screenshot({ path: path.join(root, 'desktop-agent-states.png'), animations: 'disabled' });
  await drawer.locator('.ant-drawer-close').click();
}

async function verifyDag(nodeId, nodeName) {
  await api('POST', '/api/dag/defs', { spec: { name: 'verify-dag', steps: [{ name: 'first', kind: { type: 'python', code: "print('same-node')" } }] } });
  await chooseInForm('执行类型', 'DAG');
  await page.getByPlaceholder('act / 定义名称 / 任务 ID').fill('verify-dag');
  await page.getByLabel('任务要求').fill('DAG-SAME-NODE');
  await chooseInForm('调度节点', nodeName);
  await page.getByRole('button', { name: '启动执行' }).click();
  const drawer = page.locator('.ant-drawer:visible');
  await drawer.getByText('已完成', { exact: true }).waitFor({ timeout: 20_000 });
  const id = await drawer.locator('.ant-drawer-title').innerText();
  assert.equal((await api('GET', `/api/executions/${id}`)).execution.node_id, nodeId);
  await drawer.getByText('verify-dag', { exact: true }).waitFor();
  await drawer.locator('.ant-drawer-close').click();
}

async function verifyTeamRetry() {
  await api('POST', '/api/teams', { name: 'verify-team', captain: 'captain', members: [{ id: 'captain', agent: 'act', role: 'TEAM-LARGE' }] });
  await page.getByRole('menuitem', { name: '团队组队' }).click();
  await page.getByText('verify-team', { exact: true }).waitFor();
  await page.getByRole('row', { name: /verify-team/ }).getByRole('button', { name: '启动团队' }).click();
  const modal = page.getByRole('dialog', { name: '启动 verify-team' });
  await modal.getByLabel('任务要求').fill('TEAM-LARGE');
  const ids = [];
  let abort = true;
  await page.route('**/api/executions', async (route) => {
    if (route.request().method() !== 'POST') return route.continue();
    const body = route.request().postDataJSON();
    if (body.kind !== 'team') return route.continue();
    ids.push(body.id);
    if (abort) { abort = false; return route.abort('failed'); }
    return route.continue();
  });
  await modal.getByRole('button', { name: /启\s*动/ }).click();
  await page.getByText(/再次启动会继续确认同一执行/).waitFor();
  await modal.getByRole('button', { name: /启\s*动/ }).click();
  assert.equal(ids.length, 2); assert.equal(ids[0], ids[1]);
  await page.unroute('**/api/executions');
  const drawer = page.locator('.ant-drawer:visible');
  await drawer.getByRole('button', { name: 'captain · 第 1 次结果' }).waitFor({ timeout: 30_000 });
  await drawer.getByRole('button', { name: 'captain · 第 1 次结果' }).click();
  await drawer.getByText(/TEAM-LARGE-/).waitFor();
  assert.equal(await page.getByText('网络错误: Failed to fetch', { exact: true }).count(), 0);
  await page.screenshot({ path: path.join(root, 'desktop-team-child.png'), animations: 'disabled' });
  await drawer.locator('.ant-drawer-close').click();
}

async function verifyBrainRetry() {
  const cap = await api('POST', '/api/brain/capabilities', { capability_type: 'tool-usage', summary: 'verify route', input_desc: 'task', output_desc: 'done', eng_inputs: ['verify'] });
  brainCapability = cap.capability.id;
  await api('PUT', `/api/brain/capabilities/${brainCapability}/target`, { kind: 'agent', target: 'act' });
  await page.getByRole('menuitem', { name: '大脑调度' }).click();
  await page.getByLabel('任务目标').fill('BRAIN-STABLE');
  const ids = [];
  let abort = true;
  await page.route('**/api/brain/dispatch', async (route) => {
    const body = route.request().postDataJSON(); ids.push(body.request_id);
    if (abort) { abort = false; return route.abort('failed'); }
    return route.continue();
  });
  await page.getByRole('button', { name: '直接调度执行' }).click();
  await page.getByText(/调度尚未确认/).waitFor();
  await page.getByRole('button', { name: '直接调度执行' }).click();
  assert.equal(ids.length, 2); assert.equal(ids[0], ids[1]);
  await page.unroute('**/api/brain/dispatch');
  await page.locator('.ant-drawer:visible').getByText('browser fresh answer', { exact: true }).waitFor({ timeout: 30_000 });
  assert.equal(await page.getByText('网络错误: Failed to fetch', { exact: true }).count(), 0);
  assert.equal((await api('GET', `/api/executions?limit=100`)).executions.filter((row) => row.id === `agent-${ids[0]}`).length, 1);
  await page.locator('.ant-drawer:visible .ant-drawer-close').click();
}

async function verifyProjectAndTodo() {
  const todo = await api('POST', '/api/project/todos', { title: 'verify project', draft: 'PROJECT-LARGE-' + 'D'.repeat(70_000) });
  await api('POST', `/api/project/todos/${todo.id}/plan`, {});
  const projectId = `project-${todo.id}`;
  await until(async () => (await api('GET', `/api/executions/${projectId}`)).execution.status === 'idle', 'project plan');
  let drawer = await openExecution(projectId);
  const planButtons = drawer.getByRole('button', { name: '计划内容' });
  await planButtons.last().click();
  await drawer.getByText(/PROJECT-LARGE-/).waitFor();
  await drawer.locator('.ant-drawer-close').click();

  const spec = { schema_version: 1, id: 'verify-workflow', name: 'verify workflow', objective: 'TODO-LARGE', constraints: [], metadata: {}, todos: [{ id: 'todo-large', title: 'large result', requirement_background: 'fixture background', instructions: 'TODO-LARGE', depends_on: [], agent: 'act', max_attempts: 1, acceptance: { criteria: 'fixture', required_tool_calls: [] }, metadata: {} }] };
  await api('POST', '/api/executions', { id: 'todos-large-ui', kind: 'todos', input: { spec } });
  await until(async () => (await api('GET', '/api/executions/todos-large-ui')).execution.status === 'done', 'todo workflow', 40_000);
  drawer = await openExecution('todos-large-ui');
  await drawer.getByRole('button', { name: '执行结果' }).click();
  await drawer.getByText(/TODO-LARGE-/).waitFor();
  await drawer.locator('.ant-drawer-close').click();
}

async function verifyMobileAndOffline(nodeB, nodeBId) {
  await page.setViewportSize({ width: 390, height: 844 });
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await page.screenshot({ path: path.join(root, 'mobile-executions.png'), animations: 'disabled' });
  await api('POST', '/api/executions', {
    id: 'agent-offline-owner', kind: 'agent', target: 'act', node_id: nodeBId,
    input: { prompt: '', title: 'offline owner' },
  });
  await until(async () => (await api('GET', '/api/executions/agent-offline-owner')).execution.status === 'idle', 'offline owner idle');
  await stop(nodeB);
  await until(async () => !(await api('GET', '/api/nodes')).nodes.find((node) => node.id === nodeBId)?.online, 'node-b offline');
  await page.getByRole('button', { name: /^刷\s*新$/ }).click();
  const offline = page.getByRole('row', { name: /agent-offline-owner.*离线/ });
  await offline.getByRole('button').click();
  await page.locator('.ant-drawer:visible').getByText(/所属节点当前离线/).waitFor();
  await page.screenshot({ path: path.join(root, 'mobile-offline-owner.png'), animations: 'disabled' });
}

async function main() {
  await startMock();
  const dirs = ['server', 'node-a', 'node-b'].map((name) => { const dir = path.join(root, name); fs.mkdirSync(dir); fs.writeFileSync(path.join(dir, 'opencoder.json'), JSON.stringify({ providers: { fixture: { base_url: `http://127.0.0.1:${mock.address().port}/v1`, api_key: 'fixture' } }, model: 'fixture/model', cache_salt: false })); return dir; });
  const server = start('opencoder-server', ['--workdir', dirs[0], '--port', '0', '--token', token], dirs[0]);
  await until(() => { const found = fs.readFileSync(server.logPath, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/); if (found) base = found[1]; return !!base; }, 'server');
  const nodes = dirs.slice(1).map((dir) => start('opencoder-agent', ['--remote', base, '--token', token, '--name', path.basename(dir), '--workdir', dir, '--data-dir', path.join(dir, 'state')], dir));
  await until(async () => (await api('GET', '/api/nodes')).nodes.filter((node) => node.online && node.snapshot?.ready).length === 2, 'nodes');
  const views = (await api('GET', '/api/nodes')).nodes;
  await seedIndexes(views.map((node) => node.id));
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  page = await browser.newPage({ viewport: { width: 1600, height: 1000 } }); page.setDefaultTimeout(15_000);
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('console', (message) => {
    if (message.type() !== 'error' || message.text().includes('401 (Unauthorized)')) return;
    if (message.text().includes('net::ERR_FAILED') || message.text().includes('503 (Service Unavailable)')) expectedConsoleErrors.push(message.text());
    else errors.push(message.text());
  });
  page.on('response', (response) => {
    if (response.url().includes('/api/') && response.status() >= 400) {
      httpFailures.push({ status: response.status(), path: new URL(response.url()).pathname });
    }
  });
  await verifyLogin(); await verifyPaging(views.map((node) => node.id));
  await verifyAgent('node-a'); await verifyDag(views.find((node) => node.name === 'node-a').id, 'node-a');
  await verifyTeamRetry(); await verifyBrainRetry(); await verifyProjectAndTodo();
  await verifyMobileAndOffline(nodes[1], views.find((node) => node.name === 'node-b').id);
  assert(httpFailures.some(({ status }) => status === 401));
  const unexpectedHttp = httpFailures.filter(({ status, path }) => status !== 401 && !(status === 503 && path.startsWith('/api/executions/agent-offline-owner')));
  assert.deepEqual(unexpectedHttp, []);
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ result: 'PASS', root, httpFailures, expectedConsoleErrors, screenshots: fs.readdirSync(root).filter((name) => name.endsWith('.png')) }));
}

const deadline = setTimeout(() => { children.forEach((child) => child.kill('SIGKILL')); process.exit(1); }, 240_000);
main().catch(async (error) => { console.error(error); console.error(JSON.stringify({ browserErrors: errors, httpFailures })); if (page) { await page.screenshot({ path: path.join(root, 'failure.png') }); console.error((await page.locator('body').innerText()).slice(-5000)); } console.error(`artifacts: ${root}`); process.exitCode = 1; }).finally(async () => {
  if (browser) await browser.close();
  for (const child of children.reverse()) await stop(child);
  if (mock) await new Promise((resolve) => mock.close(resolve));
  clearTimeout(deadline);
});
