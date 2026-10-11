// Browser acceptance against an existing Web entry and real model configuration.
// Only creates labeled read-only conversations; never changes identities or auth.
// node scripts/acceptance/conversations/browser.js --base URL --token-file FILE \
//   --node NODE --agent AGENT --output NEW_DIRECTORY
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const ui = require('./support');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const options = Object.fromEntries(Array.from({ length: Math.floor((process.argv.length - 2) / 2) }, (_, i) => [process.argv[2 + i * 2], process.argv[3 + i * 2]]));
for (const key of ['--base', '--token-file', '--node', '--agent', '--output']) {
  assert(options[key], `required argument: ${key}`);
}
const root = path.resolve(options['--output']);
fs.mkdirSync(root, { recursive: false, mode: 0o700 });
const base = new URL(options['--base']).origin;
const token = fs.readFileSync(options['--token-file'], 'utf8').trim();
const nodeName = options['--node'];
const agent = options['--agent'];
const report = { started: new Date().toISOString(), base, nodeName, agent, mocked: false, conversations: [] };
const save = () => fs.writeFileSync(path.join(root, 'result.json'), JSON.stringify(report, null, 2), { mode: 0o600 });
const api = ui.api;
const enter = (page, mode) => ui.enter(page, base, mode, nodeName);
const chooseAgent = (page, mode) => ui.chooseAgent(page, mode, agent);
async function awaitAnswer(page, id, marker, result, label) {
  const end = Date.now() + 240000;
  while (Date.now() < end) {
    const execution = await api(page, `/api/executions/${id}`);
    result.last_execution = { status: execution.status, execution: execution.body.execution,
      error: execution.body.error, result: execution.body.result };
    const transcript = await page.locator('.ant-bubble-list').innerText().catch(() => '');
    const assistant = await page.locator('.ant-bubble-start').allTextContents();
    if (['idle', 'done'].includes(execution.body.execution?.status) && assistant.some(t => t.includes(marker))) {
      result[label] = { passed: true, at: new Date().toISOString(), status: execution.body.execution.status, assistant };
      await page.locator('.ant-sender-actions-btn-loading-button').waitFor({ state: 'hidden' });
      await page.screenshot({ path: path.join(root, `${result.mode}-${label}.png`), fullPage: true });
      fs.writeFileSync(path.join(root, `${result.mode}-${label}.txt`), transcript);
      save();
      console.log(result.mode, label, 'PASS', id);
      return;
    }
    if (['failed', 'error', 'cancelled', 'interrupted'].includes(execution.body.execution?.status)) throw new Error(`execution ${execution.body.execution.status}: ${transcript}`);
    await new Promise(resolve => setTimeout(resolve, 1500));
  }
  throw new Error(`No completed browser answer for ${label}: ${JSON.stringify(result.last_execution)}`);
}
async function verifyTool(page, result) {
  const snapshot = await api(page, `/api/sessions/${result.id}`);
  assert.equal(snapshot.status, 200, 'completed session must remain readable');
  const blocks = (snapshot.body.messages || []).flatMap(message => message.blocks || []);
  const calls = blocks.filter(block => block.kind === 'tool_use');
  assert.equal(calls.length, 1, 'read-only acceptance must execute exactly one tool');
  const call = calls[0];
  assert(/\bpwd\b/.test(JSON.stringify(call.input)), 'tool must execute pwd');
  const output = blocks.find(block => block.kind === 'tool_result' && block.tool_use_id === call.id);
  assert(output && !output.is_error, 'pwd must have a successful persisted result');
  result.tool = { name: call.name, input: call.input, output: output.content, passed: true };
  const steps = page.locator('.ant-bubble-list').getByText(/^1 Step$/);
  if (await steps.count() === 1) await steps.click();
  await page.screenshot({ path: path.join(root, `${result.mode}-tool.png`), fullPage: true });
  save();
}
async function run(browser, mode) {
  const result = { mode, network: [], browser_errors: [] };
  report.conversations.push(result); save();
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } });
  await context.addInitScript(value => localStorage.setItem('oc_token', value), token);
  const page = await context.newPage();
  page.setDefaultTimeout(90000);
  page.on('request', request => {
    if (new URL(request.url()).pathname === '/api/sessions' && request.method() === 'POST') {
      result.submit_request = request.postDataJSON(); result.submit_at = new Date().toISOString(); save();
      console.log(mode, 'REQUEST', JSON.stringify(result.submit_request));
    }
  });
  page.on('pageerror', e => result.browser_errors.push(e.message));
  page.on('response', response => {
    const url = new URL(response.url());
    if (url.pathname.startsWith('/api/sessions') || response.status() >= 400) {
      result.network.push({ at: new Date().toISOString(), path: url.pathname + url.search, method: response.request().method(), status: response.status() });
      save();
    }
  });
  try {
    console.log(mode, 'ENTER');
    await enter(page, mode);
    report.health = (await api(page, '/api/health')).body;
    const nodes = (await api(page, '/api/nodes')).body.nodes;
    const node = nodes.find(n => n.name === nodeName);
    report.node = { id: node.id, version: node.version, ready: node.snapshot?.ready };
    console.log(mode, 'SELECT');
    const sender = await chooseAgent(page, mode);
    console.log(mode, 'READY');
    const marker = `WEB_${mode.toUpperCase()}_${Date.now()}`;
    result.marker = marker;
    await sender.fill(`这是 Web Agent 入口的最小只读验收。不要修改文件、配置、数据库，不要发消息或部署。请只调用一次 shell 执行 pwd，然后回复 ${marker} 和实际工作目录，结束本轮。`);
    await page.screenshot({ path: path.join(root, `${mode}-before-submit.png`), fullPage: true });
    const accepted = page.waitForResponse(r => new URL(r.url()).pathname === '/api/sessions' && r.request().method() === 'POST');
    await page.locator('.ant-sender button:has(.anticon-arrow-up)').click();
    const response = await accepted;
    result.admission_ms = Date.now() - Date.parse(result.submit_at);
    result.submit = { status: response.status(), request: response.request().postDataJSON(), body: await response.json() };
    save(); console.log(mode, 'SUBMITTED', JSON.stringify(result.submit));
    assert(response.ok(), `submit failed: ${JSON.stringify(result.submit)}`);
    assert(result.admission_ms <= 30000, `admission exceeded 30 seconds: ${result.admission_ms} ms`);
    assert.equal(result.submit.request.agent, agent);
    assert.equal(result.submit.request.kind || 'operator', mode);
    result.id = result.submit.body.id; assert(result.id); save();
    await awaitAnswer(page, result.id, marker, result, 'first');
    await verifyTool(page, result);
    const followup = `${marker}_CONTINUE`;
    await sender.fill(`继续刚才的会话。不调用工具，只回复 ${followup}。`);
    const continued = page.waitForResponse(r => new URL(r.url()).pathname === `/api/sessions/${result.id}/prompt` && r.request().method() === 'POST');
    await page.locator('.ant-sender button:has(.anticon-arrow-up)').click();
    assert((await continued).ok(), 'continuation rejected');
    await awaitAnswer(page, result.id, followup, result, 'continuation');
    await enter(page, mode);
    await ui.reopen(page, node, mode, result.id);
    await page.locator('.ant-bubble-start').filter({ hasText: followup }).last().waitFor();
    await page.screenshot({ path: path.join(root, `${mode}-reopened.png`), fullPage: true });
    result.reopened = true;
    result.passed = result.browser_errors.length === 0 && result.network.every(response => response.status < 400);
  } catch (error) {
    result.passed = false; result.error = error.message;
    await page.screenshot({ path: path.join(root, `${mode}-failure.png`), fullPage: true }).catch(() => {});
    fs.writeFileSync(path.join(root, `${mode}-failure.txt`), await page.locator('body').innerText().catch(() => ''));
    console.log(mode, 'FAIL', error.message);
  } finally { save(); await context.close(); }
}
(async () => {
  const browser = await chromium.launch({ headless: true, executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  try { for (const mode of ['agent', 'operator']) await run(browser, mode); }
  finally { await browser.close(); }
  report.finished = new Date().toISOString();
  report.passed = report.conversations.length === 2 && report.conversations.every(c => c.passed);
  save(); console.log('RESULT', report.passed ? 'PASS' : 'FAIL');
  process.exitCode = report.passed ? 0 : 1;
})().catch(error => { console.error(error.message); process.exitCode = 1; });
