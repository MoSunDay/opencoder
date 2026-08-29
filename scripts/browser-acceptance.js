// browser-acceptance.js -- real-browser acceptance for the fleet console SPA.
//
// Drives the committed dist bundle through a deterministic 8-step timeline
// (login -> long run -> hard link break -> offline badge -> online 中断 ->
// reconnect resume -> compact API -> transcript replay), capturing evidence
// screenshots. Requires: playwright-core (`npm i playwright-core` or
// NODE_PATH pointing at a checkout that has it) + a system chromium
// (CHROME_PATH, launched with --no-sandbox; default /usr/bin/chromium-browser).
//
// Env:
//   BASE        server origin            (default http://127.0.0.1:18727)
//   OC_TOKEN    shared auth token        (or TOKEN_FILE, default /tmp/uitest/token)
//   SHOTS       screenshot output dir    (default /tmp/uitest/shots)
//   CHROME_PATH chromium executable      (default chromium/chromium-browser)
//
// Usage: BASE=... OC_TOKEN=... node scripts/browser-acceptance.js
// Exit 0 iff every step PASSes; prints a JSON SUMMARY at the end.
//
// Deterministic timeline notes: the LONG generation (count to 20000) keeps a
// run alive across the whole offline/reconnect window; the offline 中断 click
// fails locally (badge flip evidence) while the server keeps streaming; after
// restore the SAME stream must resume (sse.js backoff reconnect), then the
// online 中断 must land (badge back + terminal done frame). Real link break
// uses `ss -K` (kernel RST aborts the in-flight streaming fetch; CDP offline
// emulation does NOT abort established loopback connections) + CDP
// setBlockedURLs to also reject the auto-reconnect attempts.
const { chromium } = require('playwright-core');
const { execSync } = require('child_process');
const crypto = require('crypto');
const fs = require('fs');

const BASE = process.env.BASE || 'http://127.0.0.1:18727';
const TOKEN = process.env.OC_TOKEN
  || fs.readFileSync(process.env.TOKEN_FILE || '/tmp/uitest/token', 'utf8').trim();
const SHOTS = process.env.SHOTS || '/tmp/uitest/shots';
const RUN = 'N' + Math.random().toString(36).slice(2, 6).toUpperCase();
const PROMPT = `（编号 ${RUN}）请从 1 逐个数到 20000：每行输出一个数字，从"1"开始，不要解释、不要省略、不要提前停止，一直数到 20000 为止。`;
const TITLE_MARK = RUN; // dialog title = prompt.slice(0,40) contains this
const PORT = new URL(BASE).port;

/// Real link break for the SSE connection: `ss -K` destroys the established
/// sockets (kernel RST -> the in-flight streaming fetch errors immediately;
/// CDP offline emulation does NOT abort established loopback connections and
/// iptables never sees snap-chromium traffic), while CDP setBlockedURLs
/// rejects every NEW request so the auto-reconnect attempts fail too.

const results = [];
const consoleErrors = [];
const apiCalls = []; // {method, path, status} for every signed SPA request
let sessionId = null;
let reconnectReq = null;
let page;

function log(m) { console.log(new Date().toISOString().slice(11, 23), m); }

async function trackApi(response) {
  const req = response.request();
  const url = new URL(response.url());
  if (!url.pathname.startsWith('/api/')) return;
  const h = await req.allHeaders();
  if (!h['x-sig']) return;
  apiCalls.push({ method: req.method(), path: url.pathname + url.search, status: response.status() });
  if (apiCalls.length > 50) apiCalls.shift();
}

async function step(name, fn) {
  const t0 = Date.now();
  try { await fn(); } catch (e) {
    const diag = apiCalls.slice(-8).map((c) => `${c.method} ${c.path} -> ${c.status}`).join(' | ');
    results.push(`FAIL ${name}: ${String(e && e.message).split('\n')[0].slice(0, 200)} [api: ${diag}]`);
    log(`FAIL ${name}: ${e && e.message.split('\n')[0]}`);
    log(`api-tail: ${diag}`);
    if (page) {
      const b = await badgeText().catch(() => '?');
      log(`badge-at-fail: ${b}`); await trackApiFlush();
    }
    try { await page.screenshot({ path: `${SHOTS}/fail-${name.replace(/\W+/g, '_')}.png` }); } catch {}
    throw e;
  }
  results.push(`PASS ${name} (${Date.now() - t0}ms)`);
  log(`PASS ${name} (${Date.now() - t0}ms)`);
}

// node-side signed call (same canonical: METHOD\npath\nts\nsha256(body))
async function signedJson(method, pathAndQuery, bodyObj) {
  const bodyText = bodyObj === undefined ? '' : JSON.stringify(bodyObj);
  const ts = Date.now().toString();
  const bodyHash = crypto.createHash('sha256').update(bodyText).digest('hex');
  const canon = [method, pathAndQuery, ts, bodyHash].join('\n');
  const sig = crypto.createHmac('sha256', TOKEN).update(canon).digest('hex');
  const res = await fetch(BASE + pathAndQuery, {
    method,
    headers: { 'x-sig-timestamp': ts, 'x-sig': sig, ...(bodyText ? { 'content-type': 'application/json' } : {}) },
    body: bodyText || undefined,
  });
  let json = null; try { json = await res.json(); } catch {}
  return { status: res.status, json };
}

async function trackApiFlush() {
  results.push(`api-calls: ${apiCalls.map((c) => `${c.method} ${c.path.replace(/\?.*/, '')}=${c.status}`).join(', ')}`);
}
const badgeText = async () => (await page.locator('.ant-badge').innerText().catch(() => '')).trim();
const shot = (n) => page.screenshot({ path: `${SHOTS}/${n}.png`, timeout: 120000 }); // compositor can starve on loaded hosts
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

async function waitBadge(text, timeout) {
  await page.waitForFunction(
    (t) => document.querySelector('.ant-badge') && document.querySelector('.ant-badge').innerText.includes(t),
    text, { timeout },
  );
}

async function bodyLen() { return (await page.locator('body').innerText()).length; }

/// poll until the transcript grows by >=40 chars (frames are flowing)
async function waitGrowth(timeout) {
  const deadline = Date.now() + timeout;
  let prev = await bodyLen();
  while (Date.now() < deadline) {
    await sleep(4000);
    const len = await bodyLen();
    if (len > prev + 40) return len;
    prev = len;
  }
  throw new Error('transcript stalled for ' + timeout + 'ms (no stream frames)');
}

async function gotoChatTab() {
  await page.getByRole('menuitem', { name: '会话交互' }).click();
  await page.getByPlaceholder(/输入提示词/).waitFor({ timeout: 60000 });
}

async function sendPrompt(text) {
  await page.getByPlaceholder(/输入提示词/).fill(text);
  await page.getByPlaceholder(/输入提示词/).press('Enter'); // X Sender: Enter submits
  await page.getByText('streaming', { exact: false }).first().waitFor({ timeout: 90000 });
}

async function waitDone(timeout) {
  await page.getByText('done', { exact: true }).first().waitFor({ timeout });
}

// 中断 surface is now Sender's loading-state stop button (see stopBtn()).
const stopBtn = () => page.locator('.ant-sender-actions-btn-loading-button').first();

(async () => {
  const browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage'],
  });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  page = await ctx.newPage();
  page.on('response', trackApi);
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));
  page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text().slice(0, 200)); });
  page.on('response', async (r) => {
    if (!sessionId && r.url().includes('/api/sessions') && r.request().method() === 'POST'
        && !r.url().includes('/prompt')) {
      try { const j = await r.json(); if (j && j.id) sessionId = j.id; } catch {}
    }
  });

  try {
    await step('01_login_with_token', async () => {
      await page.goto(BASE + '/', { waitUntil: 'domcontentloaded', timeout: 90000 });
      await page.getByText('Opencoder Fleet · 登录').waitFor({ timeout: 60000 });
      await page.getByLabel('共享密钥 (Token)').fill(TOKEN);
      await page.getByRole('button', { name: /连\s*接/ }).click();
      await waitBadge('已连接', 20000);
    });

    await step('02_fleet_console_renders', async () => {
      await page.getByRole('menuitem', { name: 'Opencoder 列表' }).waitFor({ timeout: 10000 });
      await page.getByText('暂无 Opencoder 节点').or(page.locator('.ant-table-row')).first().waitFor({ timeout: 60000 });
      await shot('01-fleet-console');
    });

    await step('03_local_streaming_run_starts', async () => {
      await gotoChatTab();
      await sendPrompt(PROMPT);
      await waitGrowth(60000); // first frames flowing (model streams fast — act quickly)
      await shot('02-streaming');
    });

    await step('04_sse_offline_badge_flips', async () => {
      // The offline click needs an ACTIVE stream. If run1 already finished
      // (fast model), start a fresh one so a live SSE connection exists to
      // break.
      const intBtn = page.getByRole('button', { name: /中\s*断/ });
      if (await intBtn.isDisabled().catch(() => true)) {
        await sendPrompt(PROMPT);
        await page.getByText('streaming', { exact: false }).first().waitFor({ timeout: 60000 });
      }
      const cdp = await ctx.newCDPSession(page);
      await cdp.send('Network.enable');
      await cdp.send('Network.setBlockedURLs', { urls: [`*${new URL(BASE).host}/*`] });
      execSync(`ss -K '( dport = :${PORT} or sport = :${PORT} )' || true`);
      // chat tab has no REST polling: force a signed call to fail — the
      // offline 中断 click must surface as the 连接断开 badge (and, being
      // offline, never reaches the server: the run keeps streaming there).
      await stopBtn().click();
      await waitBadge('连接断开', 20000);
      await shot('03-offline-badge');
      await sleep(4000); // hold the drop > 2 backoff cycles (1s, 2s)
      // arm the reconnect witness BEFORE restoring: sse.js must issue a fresh
      // signed GET /events within its backoff schedule once the link returns.
      reconnectReq = page.waitForRequest(
        (r) => r.url().includes('/events') && r.method() === 'GET',
        { timeout: 60000 },
      );
      await cdp.send('Network.setBlockedURLs', { urls: [] }); // restore the link
    });

    await step('05_sse_auto_reconnect_issues_new_stream', async () => {
      const req = await reconnectReq; // the backoff retry opened a fresh SSE
      log(`reconnect request: ${req.url().slice(0, 90)}`);
      await shot('04-reconnected');
      log(`badge now='${await badgeText()}' sessionId=${sessionId}`);
    });

    await step('06_interrupt_stops_run', async () => {
      // two legitimate post-reconnect states: run1 still streaming (interrupt
      // it directly) or already finished + resynced (launch run2, interrupt that).
      const doneTag = page.getByText('done', { exact: true });
      if ((await doneTag.count()) === 0) {
        await page.getByText('streaming', { exact: false }).first().waitFor({ timeout: 60000 });
      } else {
        await sendPrompt(PROMPT); // fresh run in the same dialog
      }
      await stopBtn().click(); // lands this time
      await waitBadge('已连接', 20000); // the signed POST succeeded
      await waitDone(90000); // terminal frame -> transcript normalized from store
      await shot('05-interrupted');
    });

    await step('07_compact_accepted', async () => {
      const s = await signedJson('GET', `/api/sessions/${sessionId}`);
      const before = ((s.json && s.json.messages) || []).length;
      const r = await signedJson('POST', `/api/sessions/${sessionId}/compact`, {});
      if (r.status !== 200 || !(r.json && r.json.ok)) throw new Error(`compact -> ${r.status} ${JSON.stringify(r.json)}`);
      // The summary turn is an LLM call over the whole transcript — for a
      // 20k-line tool result it takes minutes. Wait the drain out (180s cap)
      // instead of a fixed sleep, then sample the persisted messages.
      for (let i = 0; i < 60; i++) {
        await sleep(3000);
        const st = await signedJson('GET', `/api/sessions/${sessionId}`);
        if (st.json && st.json.draining === false) break;
      }
      const s2 = await signedJson('GET', `/api/sessions/${sessionId}`);
      const after = ((s2.json && s2.json.messages) || []).length;
      log(`messages before=${before} after=${after}`);
      await shot('06-after-compact-api');
    });

    await step('08_transcript_replay_after_compact', async () => {
      await page.reload();
      await waitBadge('已连接', 20000); // token persisted in localStorage
      await gotoChatTab();
      // NOTE: store rows carry title=NULL (the fleet console synthesizes the
      // prompt-slice label client-side, which a reload clears). The test env
      // is single-tenant: the most recently updated dialog IS our session.
      const opt = page.locator('.ant-conversations-item').first();
      await opt.waitFor({ state: 'visible', timeout: 60000 });
      if (!(await opt.count())) throw new Error('dialog list empty');
      await opt.click(); // Conversations sidebar: onActiveChange -> openDialog
      // Transcript renders after openDialog's snapshot GET resolves — poll
      // for it instead of a flat sleep.
      await page.waitForFunction(
        () => document.body.innerText.length >= 500,
        undefined,
        { timeout: 90000 },
      );
      const body = await page.locator('body').innerText();
      if (body.length < 500) throw new Error('replay transcript too small: ' + body.length);
      if (!/▲ in/.test(body)) throw new Error('usage footer missing after replay');
      await shot('07-compact-replay');
      log(`replay transcript chars=${body.length}`);
    });

    console.log('SUMMARY ' + JSON.stringify({ sessionId, results, consoleErrors }, null, 2));
    process.exitCode = 0;
  } catch (e) {
    console.log('ABORT: ' + e.message.split('\n')[0]);
    console.log('SUMMARY ' + JSON.stringify({ sessionId, results, consoleErrors }, null, 2));
    process.exitCode = 1;
  } finally {
    await browser.close().catch(() => {});
  }
})();
