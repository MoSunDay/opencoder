// link_login.js -- real-browser acceptance for URL-carried token login
// (免弹窗登录): drives the SPA served by a REAL opencoder-server through
// chromium and proves the fixed-token link contract end to end:
//   1. #token= link logs in without the modal and scrubs the hash
//   2. ?token= link preserves unrelated params, scrubs only the secret
//   3. a URL token overrides stale stored credentials
//   4. a wrong token falls back to the login modal (401 -> cleared)
//   5. the manual modal path still works (no regression)
//   6. a base-only link adopts the base without logging in (token untouched)
//   7. a wrong token + base link: the 401 clears the token but KEEPS the
//      link-delivered base (the modal still points at that fleet)
// and that authenticated /api calls carry `Authorization: Bearer <token>`.
//
// Self-contained: spawns `opencoder-server` on a free loopback port with a
// FIXED shared token (随机生成一次、全程固定 — no ephemeral tickets), temp
// workdir, and tears everything down. Point it at an existing deployment
// with BASE + OC_TOKEN set (spawn skipped).
//
// Usage: node scripts/acceptance/link_login.js
// Env: BASE, OC_TOKEN, CHROME_PATH (default /usr/bin/chromium-browser),
//      SERVER_BIN (default target/release/opencoder-server), SHOTS dir.

const { spawn } = require('child_process');
const { chromium } = require('../../crates/web/spa/node_modules/playwright-core');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');
const assert = require('assert/strict');

const TOKEN = process.env.OC_TOKEN || crypto.randomBytes(12).toString('hex');
const SERVER_BIN = process.env.SERVER_BIN
  || path.join(__dirname, '../../target/release/opencoder-server');
const SHOTS = process.env.SHOTS || fs.mkdtempSync(path.join(os.tmpdir(), 'oc-link-login-'));
const workdir = fs.mkdtempSync(path.join(os.tmpdir(), 'oc-link-login-srv-'));
const results = [];
const apiAuths = [];
let server = null;
let browser = null;
let page = null;
let base = process.env.BASE || null;

const log = (m) => console.log(new Date().toISOString().slice(11, 23), m);

async function step(name, fn) {
  const t0 = Date.now();
  try {
    await fn();
  } catch (e) {
    results.push(`FAIL ${name}: ${String(e && e.message).split('\n')[0].slice(0, 200)}`);
    log(`FAIL ${name}: ${e && e.message}`);
    try { await page.screenshot({ path: `${SHOTS}/fail-${name.replace(/\W+/g, '_')}.png` }); } catch {}
    throw e;
  }
  results.push(`PASS ${name} (${Date.now() - t0}ms)`);
  log(`PASS ${name} (${Date.now() - t0}ms)`);
}

function freePort() {
  return new Promise((resolve, reject) => {
    const srv = http.createServer();
    srv.listen(0, '127.0.0.1', () => { const { port } = srv.address(); srv.close(() => resolve(port)); });
    srv.on('error', reject);
  });
}

async function startServer() {
  if (base) {
    log(`targeting existing server ${base} (spawn skipped)`);
    return;
  }
  const port = await freePort();
  base = `http://127.0.0.1:${port}`;
  server = spawn(SERVER_BIN, [
    '--host', '127.0.0.1', '--port', String(port),
    '--workdir', workdir, '--token', TOKEN,
  ], { stdio: ['ignore', 'pipe', 'pipe'] });
  server.stderr.on('data', (d) => { if (String(d).includes('listening')) log(String(d).trim().split('\n').pop()); });
  await new Promise((resolve, reject) => {
    const deadline = Date.now() + 15000;
    const probe = () => http.get(`${base}/api/time`, (r) => (r.statusCode === 200 ? resolve() : retry()))
      .on('error', retry);
    const retry = () => (Date.now() > deadline ? reject(new Error('server did not come up')) : setTimeout(probe, 250));
    probe();
  });
  log(`server up at ${base} (fixed token)`);
}

/// Fresh context per case: no cookies, no localStorage, one page.
async function freshPage() {
  const context = await browser.newContext();
  const p = await context.newPage();
  p.on('response', async (r) => {
    const u = new URL(r.url());
    if (!u.pathname.startsWith('/api/')) return;
    const h = await r.request().allHeaders().catch(() => ({}));
    if (h.authorization) apiAuths.push(`${h.authorization} -> ${r.status()}`);
  });
  return { context, page: p };
}

// Fleet table landmark (works for both empty fleets and live ones with
// registered nodes — acceptance must pass against either).
const fleetVisible = (p) => p.waitForSelector('.ant-table', { timeout: 15000 });
const modalVisible = (p) => p.waitForSelector('text=Opencoder Fleet · 登录', { timeout: 15000 });
const storedToken = (p) => p.evaluate(() => localStorage.getItem('oc_token'));

async function main() {
  fs.mkdirSync(SHOTS, { recursive: true });
  await startServer();
  browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });

  await step('hash-link login: no modal, token stored, hash scrubbed', async () => {
    ({ page } = await freshPageWrap());
    await page.goto(`${base}/#token=${TOKEN}`, { waitUntil: 'domcontentloaded' });
    await fleetVisible(page);
    assert.equal(await storedToken(page), TOKEN);
    assert.equal(new URL(page.url()).hash, '');
    assert.ok(apiAuths.some((a) => a === `Bearer ${TOKEN} -> 200`));
    await page.screenshot({ path: `${SHOTS}/1-hash-login.png` });
  });

  await step('query-link login: unrelated params survive, secret scrubbed', async () => {
    ({ page } = await freshPageWrap());
    await page.goto(`${base}/?view=nodes&token=${TOKEN}#fleet`, { waitUntil: 'domcontentloaded' });
    await fleetVisible(page);
    const u = new URL(page.url());
    assert.equal(u.search, '?view=nodes');
    assert.equal(u.hash, '#fleet');
    assert.equal(await storedToken(page), TOKEN);
    await page.screenshot({ path: `${SHOTS}/2-query-login.png` });
  });

  await step('url token overrides stale stored credentials', async () => {
    ({ page } = await freshPageWrap());
    await page.addInitScript(([stale]) => localStorage.setItem('oc_token', stale), ['stale-token']);
    await page.goto(`${base}/?token=${TOKEN}`, { waitUntil: 'domcontentloaded' });
    await fleetVisible(page);
    assert.equal(await storedToken(page), TOKEN);
    await page.screenshot({ path: `${SHOTS}/3-override.png` });
  });

  await step('wrong token: modal fallback, credentials cleared, url scrubbed', async () => {
    ({ page } = await freshPageWrap());
    await page.goto(`${base}/?token=definitely-wrong`, { waitUntil: 'domcontentloaded' });
    await modalVisible(page);
    assert.equal(await storedToken(page), null);
    assert.equal(new URL(page.url()).search, '');
    await page.screenshot({ path: `${SHOTS}/4-bad-token-modal.png` });
  });

  await step('manual modal login still works', async () => {
    ({ page } = await freshPageWrap());
    await page.goto(base + '/', { waitUntil: 'domcontentloaded' });
    await modalVisible(page);
    await page.fill('input[id="token"]', TOKEN);
    await page.click('button:has-text("连 接")');
    await fleetVisible(page);
    assert.equal(await storedToken(page), TOKEN);
    await page.screenshot({ path: `${SHOTS}/5-manual-login.png` });
  });

  await step('base-only link: base adopted, no login, url scrubbed', async () => {
    ({ page } = await freshPageWrap());
    await page.goto(`${base}/#base=${base}`, { waitUntil: 'domcontentloaded' });
    await modalVisible(page); // the link carries no token → login still required
    assert.equal(await page.evaluate(() => localStorage.getItem('oc_base')), base);
    assert.equal(await storedToken(page), ''); // adopted-as-empty, not a secret
    assert.equal(new URL(page.url()).hash, '');
    await page.screenshot({ path: `${SHOTS}/6-base-only.png` });
  });

  await step('wrong token + base link: 401 clears token, base survives', async () => {
    ({ page } = await freshPageWrap());
    await page.goto(`${base}/#token=definitely-wrong&base=${base}`, { waitUntil: 'domcontentloaded' });
    await modalVisible(page);
    assert.equal(await storedToken(page), null);
    // clearToken (not clearCredentials): the token was wrong, the address
    // was not — the link-delivered base must survive the 401.
    assert.equal(await page.evaluate(() => localStorage.getItem('oc_base')), base);
    await page.screenshot({ path: `${SHOTS}/7-bad-token-keeps-base.png` });
  });

  console.log('--- SUMMARY ---');
  results.forEach((r) => console.log(r));
  console.log(`shots: ${SHOTS}`);
  if (results.some((r) => r.startsWith('FAIL'))) process.exitCode = 1;
}

/// freshPageWrap keeps ONE live page/context at a time (page var for step's
/// failure screenshot) and closes the previous one.
let currentContext = null;
async function freshPageWrap() {
  if (currentContext) await currentContext.close().catch(() => {});
  const fresh = await freshPage();
  currentContext = fresh.context;
  return fresh;
}

main().catch((e) => { console.error(e); process.exitCode = 1; }).finally(async () => {
  if (currentContext) await currentContext.close().catch(() => {});
  if (browser) await browser.close().catch(() => {});
  if (server) server.kill('SIGTERM');
  fs.rmSync(workdir, { recursive: true, force: true });
});
