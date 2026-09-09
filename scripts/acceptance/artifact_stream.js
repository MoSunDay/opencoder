// Real server + node + bundled browser download of a 256 MiB DAG artifact.
const { spawn } = require('child_process');
const { chromium } = require('../../crates/web/spa/node_modules/playwright-core');
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { stageWasm } = require('./harness/wasm');

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-artifact-browser-'));
const bin = process.env.PLATFORM_BIN_DIR || path.join(__dirname, '../../target/debug');
const token = crypto.randomBytes(24).toString('hex');
const children = [];
const fixtureBytes = Number(process.env.FIXTURE_BYTES || 256 * 1024 * 1024);
let browser;
let base;
let page;
let sampler;
const failures = [];

const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(check, label, timeout = 60_000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    if (await check()) return;
    await pause(100);
  }
  throw new Error(`timeout: ${label}`);
}
function start(name, args, cwd) {
  const logPath = path.join(root, `${name}.log`);
  const log = fs.openSync(logPath, 'w');
  const child = spawn(path.join(bin, name), args, {
    cwd,
    stdio: ['ignore', log, log],
    env: { ...process.env, HOME: root, XDG_CONFIG_HOME: path.join(root, 'config') },
  });
  fs.closeSync(log);
  child.logPath = logPath;
  children.push(child);
  return child;
}
async function api(method, route, body) {
  const text = body === undefined ? '' : JSON.stringify(body);
  const response = await fetch(base + route, {
    method,
    headers: { Authorization: `Bearer ${token}`, ...(text ? { 'content-type': 'application/json' } : {}) },
    body: text || undefined,
  });
  const value = await response.json();
  assert(response.ok, `${method} ${route}: ${response.status} ${JSON.stringify(value)}`);
  return value;
}
async function sha256(file) {
  const hash = crypto.createHash('sha256');
  for await (const chunk of fs.createReadStream(file, { highWaterMark: 64 * 1024 })) hash.update(chunk);
  return hash.digest('hex');
}
async function stop(child) {
  if (child.exitCode !== null || child.signalCode) return;
  child.kill('SIGTERM');
  try {
    await until(() => child.exitCode !== null || child.signalCode, `stop ${child.pid}`, 15_000);
  } catch {
    console.error(`shutdown timeout for pid ${child.pid}; forcing fixture cleanup`);
    child.kill('SIGKILL');
    await until(() => child.exitCode !== null || child.signalCode, `kill ${child.pid}`, 5_000);
  }
}

async function main() {
  const serverDir = path.join(root, 'server');
  const nodeDir = path.join(root, 'node');
  fs.mkdirSync(serverDir);
  fs.mkdirSync(nodeDir);
  const server = start('opencoder-server', ['--workdir', serverDir, '--port', '0', '--token', token], serverDir);
  await until(() => {
    if (server.exitCode !== null) throw new Error(fs.readFileSync(server.logPath, 'utf8'));
    const match = fs.readFileSync(server.logPath, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);
    if (match) base = match[1];
    return !!base;
  }, 'server');
  stageWasm(path.join(nodeDir, 'state'), 'seed');
  const node = start('opencoder-agent', [
    '--remote', base, '--token', token, '--name', 'artifact-node',
    '--workdir', nodeDir, '--data-dir', path.join(nodeDir, 'state'),
  ], nodeDir);
  await until(async () => (await api('GET', '/api/nodes')).nodes
    .some((item) => item.online && item.snapshot?.ready), 'ready node');
  await api('POST', '/api/executions', {
    id: 'dag-browser-artifact', kind: 'dag', input: { definition: {
      name: 'browser-artifact',
      steps: [{ name: 'first', kind: { type: 'wasm', command: 'stdout.wasm' } }],
    } },
  });
  await until(async () => (await api('GET', '/api/executions/dag-browser-artifact')).execution.status === 'done', 'dag', 90_000);
  const artifact = path.join(nodeDir, 'state/dag/dag-browser-artifact/first/output.txt');
  fs.truncateSync(artifact, fixtureBytes);

  browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH || chromium.executablePath(),
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const context = await browser.newContext({ acceptDownloads: true });
  page = await context.newPage();
  page.on('console', (message) => console.error(`browser ${message.type()}: ${message.text()}`));
  page.on('pageerror', error => failures.push(error.message));
  page.on('response', async (response) => {
    if (response.status() >= 400) {
      const failure = `${response.status()} ${response.url()}`; failures.push(failure);
      console.error(failure, await response.text());
    }
    if (response.url().includes('__opencoder_download')) {
      console.error(`download response ${response.status()} ${response.url()}`, await response.allHeaders());
    }
  });
  page.on('request', (request) => {
    if (request.url().includes('__opencoder_download')) console.error(`download request ${request.url()}`);
  });
  await page.addInitScript((credential) => localStorage.setItem('oc_token', credential), token);
  await page.goto(base, { waitUntil: 'networkidle' });
  await page.locator('.fleet-nav-category').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.getByRole('button', { name: 'dag-browser-artifact' }).click();
  await page.locator('.ant-drawer').waitFor();
  await page.locator('.ant-drawer .ant-select').first().click();
  await page.getByText('first', { exact: true }).last().click();
  await page.waitForFunction(() => !!navigator.serviceWorker.controller);

  const cdp = await context.newCDPSession(page);
  await cdp.send('Performance.enable');
  const heap = async () => (await cdp.send('Performance.getMetrics')).metrics
    .find((metric) => metric.name === 'JSHeapUsedSize').value;
  const baseline = await heap();
  let peak = baseline;
  sampler = setInterval(async () => { peak = Math.max(peak, await heap()); }, 50);
  let resolveDownload;
  const downloadPromise = new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('browser download timeout')), 180_000);
    resolveDownload = (download) => {
      clearTimeout(timer);
      resolve(download);
    };
  });
  page.on('download', resolveDownload);
  context.on('page', (opened) => opened.on('download', resolveDownload));
  await page.getByRole('button', { name: '下载节点产物' }).click();
  await pause(1000);
  console.error('sw state', await page.evaluate(async () => ({
    controller: !!navigator.serviceWorker.controller,
    registrations: (await navigator.serviceWorker.getRegistrations()).map((item) => ({
      scope: item.scope, active: item.active?.state,
    })),
    links: Array.from(document.querySelectorAll('a')).map((item) => item.href),
  })));
  const download = await downloadPromise;
  const saved = path.join(root, 'downloaded-output.txt');
  await download.saveAs(saved);
  clearInterval(sampler);

  assert.equal(fs.statSync(saved).size, fixtureBytes);
  assert.equal(await sha256(saved), await sha256(artifact));
  assert(peak - baseline < 32 * 1024 * 1024, `JS heap grew by ${peak - baseline} bytes`);
  assert.equal(await download.failure(), null);
  assert.equal(node.exitCode, null);
  assert.deepEqual(failures, []);
  console.log(JSON.stringify({ result: 'PASS', bytes: fixtureBytes,
    max_js_heap_growth: peak - baseline, root }));
}

const deadline = setTimeout(() => { children.forEach((child) => child.kill('SIGKILL')); process.exit(1); }, 240_000);
main().catch(async (error) => {
  console.error(error);
  if (page) console.error((await page.locator('body').innerText()).slice(-4000));
  console.error(`artifacts: ${root}`);
  process.exitCode = 1;
}).finally(async () => {
  if (sampler) clearInterval(sampler);
  if (browser) await browser.close();
  for (const child of children.reverse()) await stop(child);
  clearTimeout(deadline);
});
