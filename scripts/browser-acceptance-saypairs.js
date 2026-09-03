// browser-acceptance-saypairs.js -- real-browser acceptance for the say-pairs
// transcript contract (features/changelog/2026-09-03/say-pairs-*): one user
// input owns one or MORE [N Steps + Say] pairs; every non-empty Say closes
// its sub-turn and the next tool run opens a FRESH ladder strictly below it.
//
// Self-contained: spawns scripts/mock-llm-saypairs.js + `opencoder daemon
// --server` on a temp workdir (opencoder.json points the model at the mock),
// then drives the committed dist bundle in system chromium, auth seeded
// BEFORE first load via addInitScript(localStorage.oc_token) so the SPA
// self-signs every request; node-side setup/queries reuse the same HMAC.
// Scenarios: (a) 多回合交替 + 12px 间距 (b) /act_clear_context 在途回显
// (c) steer 拆梯. Env: SHOTS (default /tmp/uitest/shots-saypairs), CHROME_PATH,
// KEEP=1 keeps the temp workdir. Exit 0 iff every step PASSes.
//
// Browser reality (documented deviation): @ant-design/x Sender refuses
// onSubmit while `loading` (Sender.js triggerSend `!loading`), so Enter cannot
// admit a steer on a busy run from the composer. Mid-run steers here go through
// the SAME signed POST /api/sessions/:id/prompt {delivery:'steer'} the SPA
// uses; the echo lands on the OPEN stream via the steer_consumed frame (tail).
'use strict';

const { spawn, execSync } = require('child_process'); // stdlib only
const crypto = require('crypto'), fs = require('fs'), net = require('net'), os = require('os'), path = require('path');

function loadChromium() {
  const cands = [path.join(__dirname, '..', 'crates', 'web', 'spa', 'node_modules', 'playwright-core'), 'playwright-core'];
  for (const c of cands) { try { return require(c).chromium; } catch {} }
  throw new Error('playwright-core missing: `npm i -D playwright-core` in crates/web/spa');
}
const chromium = loadChromium();
const REPO = path.join(__dirname, '..');
const BIN = path.join(REPO, 'target', 'release', 'opencoder');
const SHOTS = process.env.SHOTS || '/tmp/uitest/shots-saypairs';
const TOKEN = 'tok-' + crypto.randomBytes(12).toString('hex'); // runtime-generated; no secrets
const MOCK_LOG = '/tmp/uitest/mock-saypairs.log';
const DAEMON_LOG = '/tmp/uitest/daemon-saypairs.log';
const ECHO_A = 'STEER-A 场景c 第一步';
const ECHO_B = 'STEER-B 场景c 转向';
const results = [];
const consoleErrors = [];
let apiPromptPosts = 0; // POSTs to /prompt issued by the BROWSER (Enter path)
let page;
let mockChild = null;
let daemonChild = null;
let browser = null;
let sessionId = null;

const log = (m) => console.log(new Date().toISOString().slice(11, 23), m);
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const assert = (cond, msg) => { if (!cond) throw new Error(msg); };
const shot = (n) => page.screenshot({ path: `${SHOTS}/${n}.png`, timeout: 20000 });

async function step(name, fn) {
  const t0 = Date.now();
  try {
    await fn();
    results.push(`PASS ${name} (${Date.now() - t0}ms)`);
  } catch (e) {
    results.push(`FAIL ${name} (${Date.now() - t0}ms): ${e.message.split('\n')[0]}`);
    try { await page.screenshot({ path: `${SHOTS}/fail-${name.replace(/\W+/g, '_')}.png` }); } catch {}
    throw e;
  }
}

const freePort = () => new Promise((resolve, reject) => { // OS-picked loopback port
  const srv = net.createServer();
  srv.listen(0, '127.0.0.1', () => { const p = srv.address().port; srv.close(() => resolve(p)); });
  srv.on('error', reject);
});
async function waitGetOk(url, timeoutMs) {
  const deadline = Date.now() + timeoutMs; // poll unsigned GET until ok
  for (;;) {
    try { const r = await fetch(url); if (r.ok) return; } catch {}
    if (Date.now() > deadline) throw new Error('timeout waiting ' + url);
    await sleep(250);
  }
} // startMock spawns the SSE mock as a child and waits for its health endpoint
async function startMock(port) {
  fs.writeFileSync(MOCK_LOG, '');
  mockChild = spawn(process.execPath, [path.join(__dirname, 'mock-llm-saypairs.js'), String(port)], { stdio: ['ignore', 'pipe', 'pipe'] });
  mockChild.stdout.pipe(fs.createWriteStream(MOCK_LOG, { flags: 'a' }));
  const err = [];
  mockChild.stderr.on('data', (d) => err.push(String(d)));
  await waitGetOk(`http://127.0.0.1:${port}/health`, 15000).catch((e) => {
    throw new Error('mock not healthy: ' + err.join('') + e.message);
  });
  log(`mock up on :${port}`);
} // startDaemon builds/spawns opencoder daemon with signed-token auth
async function startDaemon(port, workdir) {
  if (!fs.existsSync(BIN)) {
    log('building release binary...');
    execSync('cargo build --release', { cwd: REPO, stdio: 'inherit' });
  }
  const args = ['--workdir', workdir, 'daemon', '--server', '--host', '127.0.0.1', '--port', String(port), '--token', TOKEN];
  daemonChild = spawn(BIN, args, { detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  const out = fs.createWriteStream(DAEMON_LOG, { flags: 'w' });
  daemonChild.stdout.pipe(out);
  daemonChild.stderr.pipe(out);
  daemonChild.on('exit', (c) => log(`daemon exited code=${c}`));
  await waitGetOk(`http://127.0.0.1:${port}/api/time`, 30000); // unsigned clock endpoint
  log(`daemon up on :${port}`);
}

function killTree(child) { // process-group + direct SIGTERM, both best-effort
  if (!child || child.exitCode !== null) return;
  try { process.kill(-child.pid, 'SIGTERM'); } catch {}
  try { child.kill('SIGTERM'); } catch {}
}
// node-side signed call (canonical: METHOD\npathAndQuery\nts\nsha256(body))
function makeSigned(base) {
  return async (method, pathAndQuery, bodyObj) => {
    const bodyText = bodyObj === undefined ? '' : JSON.stringify(bodyObj);
    const ts = Date.now().toString();
    const bodyHash = crypto.createHash('sha256').update(bodyText).digest('hex');
    const sig = crypto.createHmac('sha256', TOKEN).update([method, pathAndQuery, ts, bodyHash].join('\n')).digest('hex');
    const res = await fetch(base + pathAndQuery, { method,
      headers: { 'x-sig-timestamp': ts, 'x-sig': sig, ...(bodyText ? { 'content-type': 'application/json' } : {}) },
      body: bodyText || undefined });
    let json = null; try { json = await res.json(); } catch {}
    return { status: res.status, json };
  };
}
// --- in-page probe toolkit (selectors: contains | containsEnd | ladder(forward
// index, v:0 = first ladder) | ladderAbove/ladderBelow(echo text): the nearest
// ladder bubble above/below that user echo — the split's structural anchor). ---

async function probe(spec) {
  return page.evaluate((s) => {
    const bubbles = [...document.querySelectorAll('.ant-bubble-list .ant-bubble')];
    const txt = (b) => (b.innerText || '').replace(/\s+/g, ' ').trim();
    const isLadder = (b) => /❯ \d+ Steps?/.test(txt(b));
    const echoIdx = (v) => bubbles.findIndex((b) => b.classList.contains('ant-bubble-end') && txt(b).includes(v));
    const nearLadder = (from, dir) => {
      for (let i = from + dir; i >= 0 && i < bubbles.length; i += dir) if (isLadder(bubbles[i])) return i;
      return -1;
    };
    let ladderSeen = -1;
    const matchers = {
      contains: (v) => (b) => txt(b).includes(v),
      containsEnd: (v) => (b) => b.classList.contains('ant-bubble-end') && txt(b).includes(v),
      ladder: (v) => (b) => { if (!isLadder(b)) return false; ladderSeen += 1; return ladderSeen === v; },
      ladderAbove: (v) => (b, i) => i === nearLadder(echoIdx(v), -1),
      ladderBelow: (v) => (b, i) => i === nearLadder(echoIdx(v), 1),
    };
    const findIdx = (sel) => bubbles.findIndex(matchers[sel.type](sel.v));
    const info = (b) => {
      const tag = [...b.querySelectorAll('.ant-tag')].find((t) => t.textContent.trim() === 'running') || null;
      const says = [...b.querySelectorAll('div')].filter((d) => d.style.marginTop === '8px' && txt(d));
      const headerM = txt(b).match(/❯ (\d+) Steps?/);
      return {
        end: b.classList.contains('ant-bubble-end'), text: txt(b).slice(0, 220),
        header: headerM ? headerM[0] : null, running: !!tag,
        runningMarginLeft: tag ? getComputedStyle(tag).marginLeft : null,
        says: says.map((d) => ({ text: txt(d), marginTop: getComputedStyle(d).marginTop })) };
    };
    if (s.op === 'bubbles') return bubbles.map(info);
    if (s.op === 'bubble') { const i = findIdx(s.sel); return i < 0 ? null : info(bubbles[i]); }
    if (s.op === 'laddersAbove') return bubbles.slice(0, echoIdx(s.v)).filter(isLadder).length;
    if (s.op === 'below') {
      const i = findIdx(s.above); const j = findIdx(s.below);
      if (i < 0 || j < 0) return { above: i, below: j, ok: false };
      return { above: i, below: j, ok: !!(bubbles[i].compareDocumentPosition(bubbles[j]) & Node.DOCUMENT_POSITION_FOLLOWING) };
    }
    if (s.op === 'countContains') return bubbles.filter(matchers[s.sel.type](s.sel.v)).length;
    return null;
  }, spec);
}

const bubbleInfo = (sel) => probe({ op: 'bubble', sel });
const bubblesDump = () => probe({ op: 'bubbles' });
const countContains = (v) => probe({ op: 'countContains', sel: { type: 'contains', v } });
async function isBelow(above, below) {
  const r = await probe({ op: 'below', above, below });
  if (!r || !r.ok) throw new Error(`DOM order failed: ${JSON.stringify({ above, below, r })}`);
}

/// Drill the nth ladder: L0 `❯ N Steps` -> `❯ Step(k)` -> `❯ N Function
/// call(s)` -> `🔧 bash`. Open collapses are skipped (antd headers toggle).
async function drillLadder(n) {
  const ladder = page.locator('.ant-bubble-list .ant-bubble').filter({ hasText: /❯ \d+ Steps?/ }).nth(n);
  for (let i = 0; i < 4; i++) {
    const header = ladder.locator('.ant-collapse-header').nth(i);
    if ((await header.count()) === 0) break;
    const open = await header.evaluate((h) => h.parentElement.classList.contains('ant-collapse-item-active'));
    if (!open) { await header.click(); await sleep(350); }
  }
}

const composer = () => page.locator('.ant-sender textarea');
async function submit(text) {
  await composer().fill(text);
  await composer().press('Enter'); // X Sender: Enter submits when not loading
  await page.locator('.ant-sender-actions-btn-loading-button').first().waitFor({ timeout: 20000 });
}

async function ensureSessionId() { // sessionId is captured from POST /api/sessions
  const deadline = Date.now() + 5000;
  while (!sessionId && Date.now() < deadline) await sleep(100);
  assert(sessionId, 'sessionId not captured from POST /api/sessions');
}

async function waitDraining(signed, want, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    const r = await signed('GET', `/api/sessions/${sessionId}`);
    if (r.json && r.json.draining === want) return r.json;
    if (Date.now() > deadline) throw new Error(`draining!==${want} after ${timeoutMs}ms`);
    await sleep(300);
  }
} // GET /api/sessions/:id .draining is the run-liveness probe

const waitText = (text, timeoutMs) => page.waitForFunction(
  (t) => document.body.innerText.includes(t), text, { timeout: timeoutMs, polling: 200 });
// User echo bubbles render TUI-parity "❯ " before the text: strip it.
const waitUserEcho = async (exactText, timeoutMs) => {
  try {
    await page.waitForFunction((t) => [...document.querySelectorAll('.ant-bubble-end')]
      .some((b) => (b.innerText || '').trim().replace(/^❯\s*/, '') === t),
    exactText, { timeout: timeoutMs, polling: 150 });
  } catch (e) {
    throw new Error(`${e.message.split('\n')[0]} | echo=${exactText} bubbles=${JSON.stringify(await bubblesDump())}`);
  }
};
const waitAnyRunningTag = (timeoutMs) => page.waitForFunction(
  () => [...document.querySelectorAll('.ant-bubble-list .ant-bubble .ant-tag')].some((t) => t.textContent.trim() === 'running'),
  undefined, { timeout: timeoutMs, polling: 150 });

async function steer(signed, text) { // same delivery the SPA uses for steers
  const r = await signed('POST', `/api/sessions/${sessionId}/prompt`, { prompt: text, delivery: 'steer' });
  assert(r.status === 200 && r.json && r.json.ok !== false, `steer POST -> ${r.status} ${JSON.stringify(r.json)}`);
}
(async () => { // --- orchestration: mock -> daemon -> browser -> scenarios ---
  fs.mkdirSync(SHOTS, { recursive: true });
  fs.mkdirSync(path.dirname(MOCK_LOG), { recursive: true });
  const mockPort = await freePort(); const daemonPort = await freePort();
  const workdir = fs.mkdtempSync(path.join(os.tmpdir(), 'oc-saypairs-'));
  const BASE = `http://127.0.0.1:${daemonPort}`; const signed = makeSigned(BASE);
  fs.writeFileSync(path.join(workdir, 'opencoder.json'), JSON.stringify({
    providers: { 'mock-saypairs': { base_url: `http://127.0.0.1:${mockPort}/v1`, api_key: 'sk-dummy' } },
    model: 'mock-saypairs/m-1',
    cache_salt: false,
  }, null, 2) + '\n');
  log(`workdir=${workdir} mock=:${mockPort} daemon=:${daemonPort}`);

  await startMock(mockPort);
  await startDaemon(daemonPort, workdir);
  browser = await chromium.launch({
    executablePath: process.env.CHROME_PATH || '/usr/bin/chromium-browser',
    args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage'],
  });
  const ctx = await browser.newContext({ viewport: { width: 1440, height: 900 } });
  page = await ctx.newPage();
  page.on('pageerror', (e) => consoleErrors.push('pageerror: ' + e.message));
  page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text().slice(0, 200)); });
  page.on('response', async (r) => { // capture session id; count browser POSTs
    const url = r.url(); const m = r.request().method();
    if (!sessionId && url.includes('/api/sessions') && m === 'POST' && !url.includes('/prompt')) {
      try { const j = await r.json(); if (j && j.id) sessionId = j.id; } catch {}
    }
    if (url.includes('/prompt') && m === 'POST') apiPromptPosts += 1;
  });
  try {
    await step('00_bootstrap_auth_seed', async () => {
      await page.addInitScript((t) => localStorage.setItem('oc_token', t), TOKEN);
      await page.goto(BASE + '/', { waitUntil: 'domcontentloaded', timeout: 30000 });
      await page.locator('.ant-badge').filter({ hasText: '已连接' }).waitFor({ timeout: 20000 });
      await page.getByRole('menuitem', { name: '会话交互' }).click();
      await composer().waitFor({ timeout: 20000 });
      await shot('00-chat-panel'); // login modal never opened (seeded token)
    });
    await step('a1_turn1_running_tag_margin', async () => {
      await submit('第一回合 开始执行');
      await ensureSessionId();
      await waitAnyRunningTag(20000); // tool round live: ladder exists + running
      const b = await bubbleInfo({ type: 'ladder', v: 0 });
      assert(b && b.running && b.runningMarginLeft === '12px',
        `ladder1 mid-run: ${JSON.stringify(b)} (want running @ margin-left 12px)`);
      assert(b.header === '❯ 1 Step', `ladder1 header=${b.header}, want ❯ 1 Step`);
      await shot('a1-running-tag');
      await waitText('Say-第一回合-done', 20000);
      await waitDraining(signed, false, 20000);
      await sleep(400); // done -> store-snapshot reload
    });
    await step('a2_turn1_frozen_ladder_say8px_drill', async () => {
      const b = await bubbleInfo({ type: 'ladder', v: 0 });
      assert(!b.running && b.header === '❯ 1 Step', `ladder1 must be frozen: ${JSON.stringify(b)}`);
      assert(b.says.length === 1 && b.says[0].text === 'Say-第一回合-done' && b.says[0].marginTop === '8px',
        `say1 wrapper: ${JSON.stringify(b.says)}`);
      await drillLadder(0);
      const d = await bubbleInfo({ type: 'ladder', v: 0 });
      assert(d.text.includes('❯ Step(1)') && d.text.includes('❯ 1 Function call'), `ladder1 drill incomplete: ${d.text}`);
      assert(d.text.includes('🔧 bash') && d.text.includes('output:') && /hi \d/.test(d.text),
        `drilled ladder1 missing 🔧 bash / output "hi N": ${d.text}`);
      await shot('a2-ladder1-drilled');
    });
    await step('a3_turn2_alternates_strict_order', async () => {
      await submit('第二回合 继续执行');
      await waitAnyRunningTag(20000);
      await waitText('Say-第二回合-done', 20000);
      await waitDraining(signed, false, 20000);
      await sleep(400);
      const l1 = await bubbleInfo({ type: 'ladder', v: 0 });
      const l2 = await bubbleInfo({ type: 'ladder', v: 1 });
      assert(l2 && l2.header === '❯ 1 Step', `ladder2 missing/header wrong: ${JSON.stringify(l2)}`);
      assert(!l1.running && !l2.running, 'running tags must not survive done');
      assert(l2.says.length === 1 && l2.says[0].text === 'Say-第二回合-done' && l2.says[0].marginTop === '8px',
        `say2 wrapper: ${JSON.stringify(l2.says)}`);
      await isBelow({ type: 'containsEnd', v: '第一回合' }, { type: 'ladder', v: 0 });
      await isBelow({ type: 'ladder', v: 0 }, { type: 'containsEnd', v: '第二回合' });
      await isBelow({ type: 'containsEnd', v: '第二回合' }, { type: 'ladder', v: 1 });
      assert(await countContains('Say-第一回合-done') === 1 && await countContains('Say-第二回合-done') === 1,
        'say bubbles duplicated');
      await shot('a3-two-turns');
    });
    await step('b1_midrun_compound_steer_echo', async () => {
      await submit('SLOW 场景b 慢速输出开始');
      await shot('b1-slow-streaming');
      // Type the compound command + Enter: the Sender loading gate refuses the
      // submit (by design); the steer then goes via the signed POST below.
      await composer().fill('/act_clear_context 收尾总结');
      const postsBefore = apiPromptPosts;
      await composer().press('Enter');
      await sleep(800);
      assert(apiPromptPosts === postsBefore, `Enter admitted a prompt while busy (${apiPromptPosts - postsBefore} POSTs)`);
      // App contract (web/src/handle.rs "Steers interrupt the current turn"):
      // the steer POST fires turn_cancel, cutting the slow in-flight Say, and
      // the consumed batch emits steer_consumed with the compound tail echo.
      await steer(signed, '/act_clear_context 收尾总结');
      await waitUserEcho('收尾总结', 20000); // steer_consumed echo on the open stream
      await shot('b1-echo-landed');
    });
    await step('b2_echo_survives_reset_once_last', async () => {
      await waitDraining(signed, false, 20000);
      await sleep(500); // transcript_reset -> snapshot refetch -> done reload
      const echo = await bubbleInfo({ type: 'containsEnd', v: '收尾总结' });
      assert(echo && echo.text.replace(/^❯\s*/, '') === '收尾总结', `echo must be EXACTLY the tail: ${JSON.stringify(echo)}`);
      assert(await countContains('收尾总结') === 1, 'echo duplicated across bubbles');
      const all = await bubblesDump();
      assert(all.length > 0 && all[all.length - 1].end && all[all.length - 1].text.replace(/^❯\s*/, '') === '收尾总结',
        `last bubble is not the echo: ${JSON.stringify(all[all.length - 1] || null)}`);
      await shot('b2-after-reset-done');
    });
    await step('c1_steer_interrupts_say_ladder_frozen', async () => {
      await submit(ECHO_A);
      // A's round 1 runs `sleep 1 && echo hi N` — wait for the running tag,
      // then for round 2's slow Say (8x700ms) so the tool result is safely
      // recorded before the steer fires.
      await waitAnyRunningTag(20000);
      await waitText('Initial findings', 20000);
      // Real contract (web/src/handle.rs): a steer POST fires turn_cancel —
      // the in-flight Say is cut and its partial text discarded; A's ladder
      // freezes at the completed round-1 step and B owns the next turn.
      await steer(signed, ECHO_B);
      await waitUserEcho(ECHO_B, 20000); // steer_consumed echo on the open stream
      const dump = await bubblesDump();
      log(`c1 bubbles=${JSON.stringify(dump.map((b) => b.text.slice(0, 60)))}`);
      assert(await probe({ op: 'laddersAbove', v: ECHO_B }) === 1, 'exactly ladder-A above the echo-B');
      const a = await bubbleInfo({ type: 'ladderAbove', v: ECHO_B });
      assert(!a.running && a.header === '❯ 1 Step', `ladder-A must be frozen at ❯ 1 Step: ${JSON.stringify(a)}`);
      await drillLadder(0);
      const pre = await bubbleInfo({ type: 'ladderAbove', v: ECHO_B });
      assert(/hi \d/.test(pre.text), `ladder-A drilled output missing: ${pre.text}`);
      await shot('c1-ladder1-frozen');
    });
    await step('c2_ladder2_below_echoB_then_say2', async () => {
      await waitAnyRunningTag(20000); // ladder-B's own tool round (sleep 4)
      await shot('c2-split-moment');
      const b = await bubbleInfo({ type: 'ladderBelow', v: ECHO_B });
      assert(b && b.running, 'ladder-B running tag missing during its tool round');
      await waitText('Steer-B handled.', 20000);
      await waitDraining(signed, false, 20000);
      await sleep(500); // done -> reloadAfterDone rebuilds from the store snapshot
      const bDone = await bubbleInfo({ type: 'ladderBelow', v: ECHO_B });
      assert(!bDone.running && bDone.says.length === 1 && bDone.says[0].text === 'Steer-B handled.'
        && bDone.says[0].marginTop === '8px', `ladder-B final: ${JSON.stringify(bDone)}`);
      const aAfter = await bubbleInfo({ type: 'ladderAbove', v: ECHO_B });
      assert(!aAfter.running && aAfter.header === '❯ 1 Step' && aAfter.says.length === 0,
        `ladder-A must stay frozen at 1 step with the interrupted Say discarded: ${JSON.stringify(aAfter)}`);
      // Strict transcript order post-done: echoA < ladderA < echoB < ladderB.
      await isBelow({ type: 'containsEnd', v: ECHO_A }, { type: 'ladderAbove', v: ECHO_B });
      await isBelow({ type: 'ladderAbove', v: ECHO_B }, { type: 'containsEnd', v: ECHO_B });
      await isBelow({ type: 'containsEnd', v: ECHO_B }, { type: 'ladderBelow', v: ECHO_B });
      await shot('c3-final-split-state');
    });
    process.exitCode = results.every((r) => r.startsWith('PASS')) ? 0 : 1;
  } catch (e) {
    log('ABORT: ' + e.message.split('\n')[0]);
    process.exitCode = 1;
  } finally {
    console.log('SUMMARY ' + JSON.stringify({ sessionId, mockPort, daemonPort, results, consoleErrors }, null, 2));
    await browser.close().catch(() => {});
    killTree(daemonChild);
    killTree(mockChild);
    if (!process.env.KEEP) {
      try { fs.rmSync(workdir, { recursive: true, force: true }); } catch {}
    } else {
      log(`KEEP=1: workdir=${workdir} logs=${MOCK_LOG},${DAEMON_LOG}`);
    }
  }
})();
