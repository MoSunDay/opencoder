// Bundled SPA -> server -> node -> Codex binary -> existing transcript ladder.
// node scripts/acceptance/harness/codex.js [absolute-path-to-real-codex]
// PLATFORM_BIN_DIR selects built binaries. All application state is temporary.
const { spawn } = require('child_process');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const os = require('os');
const path = require('path');

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-wrap-browser-'));
const bin = process.env.PLATFORM_BIN_DIR || path.join(__dirname, '../../../target/debug');
const real = process.argv[2];
const token = crypto.randomBytes(24).toString('hex');
const children = [];
const errors = [];
let base, browser, page;
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(check, label, timeout = 30000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    if (await check()) return;
    await pause(150);
  }
  throw new Error(`timeout: ${label}`);
}
async function api(method, route, body) {
  const response = await fetch(base + route, { method, signal: AbortSignal.timeout(10000), headers: {
    authorization: `Bearer ${token}`, 'content-type': 'application/json',
  }, body: body === undefined ? undefined : JSON.stringify(body) });
  const data = await response.json();
  assert(response.ok, `${route}: ${response.status}: ${JSON.stringify(data)}`);
  return data;
}
function start(name, args, cwd) {
  const logfile = path.join(root, `${name}.log`);
  const fd = fs.openSync(logfile, 'w');
  const child = spawn(path.join(bin, name), args, { cwd, stdio: ['ignore', fd, fd], env: {
    ...process.env, OPENCODER_AGENTS_DIR: path.join(root, "agent-cards"),
    ...(name === "opencoder-agent" ? { PATH: `${path.join(root, "bin")}:${process.env.PATH}`,
      ...(real ? { CODEX_HOME: process.env.CODEX_HOME || path.join(os.homedir(), ".codex") } : {}) } : {}),
    HOME: root, XDG_CONFIG_HOME: path.join(root, 'config'), XDG_DATA_HOME: path.join(root, 'data'),
  } });
  fs.closeSync(fd);
  child.logfile = logfile;
  child.on('error', (error) => errors.push(error.message));
  children.push(child);
  return child;
}
function fixture(directory) {
  const file = path.join(directory, 'codex');
  if (real) { fs.symlinkSync(path.resolve(real), file); return; }
  fs.writeFileSync(file, `#!/usr/bin/python3
import json, os, sys
if '--version' in sys.argv:
    print('codex-cli fixture'); sys.exit(0)
prompt = sys.stdin.read()
def emit(data): print(json.dumps(data), flush=True)
emit({'type':'thread.started','thread_id':'browser-thread'})
emit({'type':'turn.started'})
emit({'type':'item.completed','item':{'type':'reasoning','id':'r','text':'Inspect the file and environment'}})
emit({'type':'item.completed','item':{'type':'command_execution','id':'c0','command':'recoverable failure','aggregated_output':'simulated tool error','exit_code':1,'status':'failed'}})
output = open('sample.txt').read().strip() + ': ' + os.environ['WRAP_CHECK_VALUE']
emit({'type':'item.completed','item':{'type':'command_execution','id':'c','command':'cat sample.txt','aggregated_output':output,'exit_code':0,'status':'completed'}})
emit({'type':'item.completed','item':{'type':'agent_message','id':'a','text':('WRAP_BROWSER_SECOND' if 'resume' in sys.argv else 'WRAP_BROWSER_OK') + ': ' + output}})
emit({'type':'turn.completed','usage':{'input_tokens':10,'output_tokens':5,'cached_input_tokens':0}})
`, { mode: 0o755 });
}
async function transcript(id) {
  let route = `/api/executions/${id}/messages`, text = '';
  do {
    const page = await api('GET', route);
    for (const chunk of page.chunks) text += Buffer.from(chunk.bytes_b64, 'base64').toString();
    route = page.next_cursor ? `/api/executions/${id}/messages?seq=${page.next_cursor.seq}&offset=${page.next_cursor.offset}` : null;
  } while (route);
  return text;
}
async function expand() {
  await page.getByText(/^\d+ Steps?(?:\s*error)?$/).last().click();
  for (const step of await page.getByText(/^Step\(\d+\)(?:\s*error)?$/).all()) await step.click();
  for (const calls of await page.getByText(/^\d+ Function calls?$/).all()) await calls.click();
  for (const call of await page.getByText(/^🔧 bash/).all()) await call.click();
  const output = (await page.getByText('output:', { exact: true }).locator('..').allTextContents()).join('\n');
  assert(output.includes('sample-content') && output.includes('LOCAL_OK'), `expanded tool output missing: ${output}`);
  await page.evaluate(async () => Promise.all(document.getAnimations()
    .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
    .map((animation) => animation.finished.catch(() => {}))));
}
async function main() {
  const workdirs = ['server', 'node', 'bin'].map((name) => {
    const dir = path.join(root, name); fs.mkdirSync(dir); return dir;
  });
  fixture(workdirs[2]);
  fs.writeFileSync(path.join(workdirs[1], 'sample.txt'), 'sample-content\n');
  const server = start('opencoder-server', ['--workdir', workdirs[0], '--port', '0', '--token', token], workdirs[0]);
  await until(async () => {
    assert.equal(server.exitCode, null, fs.readFileSync(server.logfile, 'utf8'));
    const match = fs.readFileSync(server.logfile, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);
    if (match) base = match[1];
    return !!base;
  }, 'server');
  start('opencoder-agent', ['--remote', base, '--token', token, '--name', 'wrap-node', '--workdir', workdirs[1], '--data-dir', path.join(workdirs[1], 'state')], workdirs[1]);
  await until(async () => (await api('GET', '/api/nodes')).nodes.some((node) => node.online && node.snapshot.ready), 'node ready');
  browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  page = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
  page.setDefaultTimeout(20000);
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
  await page.goto(base, { waitUntil: 'networkidle' });
  await page.locator('.ant-layout-sider').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.getByPlaceholder('act / 定义名称 / 任务 ID').fill('act');
  await page.getByLabel('agent-harness').click();
  await page.locator('.ant-select-item-option-content').getByText('Codex', { exact: true }).click();
  const envs = [`PATH=${workdirs[2]}:/usr/local/bin:/usr/bin:/bin`, 'WRAP_CHECK_VALUE=LOCAL_OK'];
  if (real) envs.push(`CODEX_HOME=${process.env.CODEX_HOME || path.join(os.homedir(), '.codex')}`);
  await page.getByLabel('agent-envs').fill('INVALID');
  await page.getByRole('button', { name: '启动执行', exact: true }).click();
  await page.getByText('环境变量必须为 KEY=VALUE，每行一个', { exact: true }).waitFor();
  assert.equal((await api('GET', '/api/executions')).executions.length, 0, 'invalid input must not dispatch');
  await page.screenshot({ path: path.join(root, 'invalid-environment.png'), animations: 'disabled' });
  await page.getByLabel('agent-envs').fill(envs.join('\n'));
  await page.getByLabel('任务要求').fill('Read sample.txt and WRAP_CHECK_VALUE through a shell command. Do not edit files or contact anyone. Reply exactly: WRAP_BROWSER_OK: sample-content: LOCAL_OK');
  await page.getByRole('button', { name: '启动执行', exact: true }).click();
  let id;
  await until(async () => { id = (await api('GET', '/api/executions')).executions[0]?.id; return !!id; }, 'accepted execution');
  await until(async () => {
    const detail = await api('GET', `/api/executions/${id}`);
    assert(!detail.error, JSON.stringify(detail));
    return (await transcript(id)).includes('WRAP_BROWSER_OK') && detail.execution.status === 'idle';
  }, 'Codex first response', real ? 600000 : 30000);
  await page.getByText(/WRAP_BROWSER_OK: sample-content: LOCAL_OK/).first().waitFor();
  await expand();
  await page.screenshot({ path: path.join(root, 'expanded.png'), animations: 'disabled' });
  await page.reload({ waitUntil: 'networkidle' });
  await page.locator('.ant-layout-sider').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.getByRole('button', { name: id, exact: true }).click();
  await page.getByText(/WRAP_BROWSER_OK: sample-content: LOCAL_OK/).first().waitFor();
  await expand();
  const detail = await api('GET', `/api/executions/${id}`);
  assert.equal(detail.session.harness, 'codex');
  assert.equal(detail.request.input.envs.WRAP_CHECK_VALUE, '[redacted]');
  assert(Object.values(detail.request.input.envs).every((value) => value === '[redacted]'), 'launch environment must be redacted');
  await api('POST', `/api/executions/${id}/commands`, { action: 'prompt', input: { prompt: 'Without editing any files, reply exactly: WRAP_BROWSER_SECOND: sample-content: LOCAL_OK' } });
  await until(async () => (await transcript(id)).includes('WRAP_BROWSER_SECOND') && (await api('GET', `/api/executions/${id}`)).execution.status === 'idle', 'Codex resume', real ? 600000 : 30000);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.evaluate(async () => Promise.all(document.getAnimations()
    .filter((animation) => animation.effect?.getComputedTiming().iterations !== Infinity)
    .map((animation) => animation.finished.catch(() => {}))));
  assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'mobile viewport overflows');
  const mobile = await page.locator('.oc-execution-detail .ant-drawer-content-wrapper').boundingBox();
  assert(mobile && Math.abs(mobile.width - 390) <= 1, 'mobile execution drawer must use full viewport');
  assert(await page.locator('.oc-execution-detail .ant-drawer-title').evaluate((el) => el.scrollWidth <= el.clientWidth + 1), 'execution id is clipped');
  await page.getByText(/WRAP_BROWSER_OK: sample-content: LOCAL_OK/).last().scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(root, 'mobile.png'), animations: 'disabled' });
  if (real) await require("./project.js")({ api, until, nodeDir: workdirs[1], root });
  assert.deepEqual(errors, []);
  console.log(JSON.stringify({ result: 'PASS', real: !!real, execution_id: id, artifacts: root }));
}
main().catch(async (error) => {
  console.error(error);
  if (errors.length) console.error('browser/process errors:', errors);
  if (page) await page.screenshot({ path: path.join(root, 'failure.png') }).catch(() => {});
  console.error(`artifacts: ${root}`);
  process.exitCode = 1;
}).finally(async () => {
  if (browser) await browser.close();
  for (const child of children.reverse()) {
    if (child.exitCode !== null || child.signalCode) continue;
    child.kill('SIGTERM');
    await until(async () => child.exitCode !== null || child.signalCode, 'process cleanup').catch(() => child.kill('SIGKILL'));
  }
});
