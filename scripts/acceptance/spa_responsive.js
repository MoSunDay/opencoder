// scripts/acceptance/spa_responsive.js
//
// Fleet-console SPA phone-viewport horizontal-overflow gate.
//
// Serves the *committed* SPA bundle (crates/web/spa/dist) over localhost with a
// fixture API (see spa_responsive_fixtures.js), drives every mobile-nav page at
// 390x844, and fails on any element that escapes the viewport while the
// document itself cannot scroll sideways. The SPA only hides the mobile nav
// below 768px, so 390x844 is the width the console must survive (also what
// scripts/browser-acceptance.js uses).
//
// Chromium mobile emulation reports an *inflated* window.innerWidth once
// something overflows (390 -> 413), which would make "scrollWidth <= innerWidth"
// vacuously true, so the gate compares against the narrowest of innerWidth /
// documentElement.clientWidth / visualViewport.width and reports the elements
// crossing that edge. Elements inside an ancestor that genuinely scrolls
// sideways (antd `scroll={{ x: ... }}` tables) are excluded -- but the
// .fleet-content pane itself is not an acceptable scroller: it carries the
// mobile nav, so a table wider than the phone drags the nav away with it. An uncaught page error fails its page -- a crashed panel
// renders nothing and would otherwise pass vacuously.
//
// Usage: node scripts/acceptance/spa_responsive.js [--headed] [--port 18099]
//          [--shots /tmp/uitest/responsive] [--only 节点] [--keep]
// Exit: 0 = every visited page fits, 1 = overflow / unreachable page, 2 = harness error.

const { createServer } = require('node:http');
const { readFile } = require('node:fs/promises');
const { existsSync, mkdirSync } = require('node:fs');
const path = require('node:path');
const { FIXTURES, ABSENT, GUARDED } = require('./spa_responsive_fixtures');

const SPA = path.resolve(__dirname, '../../crates/web/spa/dist');
const WIDTH = 390;
const HEIGHT = 844;
const TOKEN = 'fixture-token';
// Mobile nav: main.jsx renders BOTH the category Segmented and the page Select
// with className="fleet-mobile-nav" as siblings inside .fleet-content.
const SEGMENT = '.ant-segmented.fleet-mobile-nav .ant-segmented-item';
const SELECT = '.ant-select.fleet-mobile-nav';
const OPTIONS = '.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option';
const TABS = '.fleet-content .ant-tabs-tab';

function arg(flag, fallback) {
  const i = process.argv.indexOf(`--${flag}`);
  return i >= 0 && process.argv[i + 1] ? process.argv[i + 1] : fallback;
}
const HEADED = process.argv.includes('--headed');
const KEEP = process.argv.includes('--keep');
const PORT = Number(arg('port', process.env.PORT || 18099));
const SHOTS = arg('shots', process.env.SHOTS || '/tmp/uitest/responsive');
const ONLY = arg('only', '');

const pause = (ms) => new Promise((r) => setTimeout(r, ms));

// ---------------------------------------------------------------------------
// Static + fixture server. Only GETs are served (every page is read-only); a
// non-GET proves the gate clicked a mutating control and is answered 405.
// ---------------------------------------------------------------------------
const TYPES = { '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8', '.svg': 'image/svg+xml', '.png': 'image/png',
  '.json': 'application/json; charset=utf-8', '.map': 'application/json' };

async function serve(port) {
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, `http://127.0.0.1:${port}`);
    const send = (code, body, type) => {
      res.writeHead(code, { 'content-type': type || 'application/json; charset=utf-8', 'cache-control': 'no-store' });
      res.end(body);
    };
    if (url.pathname.startsWith('/api/')) {
      if (req.method !== 'GET') return send(405, JSON.stringify({ error: 'gate is read-only' }));
      if (GUARDED.some((g) => url.pathname.startsWith(g)) && (req.headers.authorization || '') !== `Bearer ${TOKEN}`) {
        return send(401, JSON.stringify({ error: 'unauthorized' }));
      }
      const key = `${url.pathname}${url.search}`;
      if (ABSENT.includes(key) || ABSENT.includes(url.pathname)) {
        return send(404, JSON.stringify({ error: 'not found' }));
      }
      if (FIXTURES[url.pathname]) return send(200, JSON.stringify(FIXTURES[url.pathname]));
      console.log(`  fixture miss: GET ${key}`);
      return send(404, JSON.stringify({ error: 'no fixture' }));
    }
    // SPA fallback: unknown paths serve index.html so client routes load.
    const rel = url.pathname === '/' ? 'index.html' : url.pathname.replace(/^\/+/, '');
    const file = path.join(SPA, rel);
    if (!file.startsWith(SPA) || !existsSync(file)) {
      return send(200, await readFile(path.join(SPA, 'index.html')), TYPES['.html']);
    }
    try {
      return send(200, await readFile(file), TYPES[path.extname(file)] || 'application/octet-stream');
    } catch (_) { return send(404, JSON.stringify({ error: 'missing asset' })); }
  });
  await new Promise((r) => server.listen(port, '127.0.0.1', r));
  return { close: () => new Promise((r) => server.close(r)) };
}

// ---------------------------------------------------------------------------
// In-page measurement.
// ---------------------------------------------------------------------------
const PROBE = `(() => {
  const EDGE = Math.min(window.innerWidth || 0, document.documentElement.clientWidth || 0,
    (window.visualViewport && window.visualViewport.width) || window.innerWidth || 0);
  const TOL = 1; // sub-pixel rounding
  const pathOf = (el) => {
    const out = [];
    let n = el;
    while (n && n.nodeType === 1 && out.length < 5) {
      let sel = n.tagName.toLowerCase();
      if (n.id) sel += '#' + n.id;
      else if (typeof n.className === 'string' && n.className.trim()) {
        sel += '.' + n.className.trim().split(/\\s+/).slice(0, 2).join('.');
      }
      const p = n.parentElement;
      if (p) sel += ':nth-of-type(' + (Array.from(p.children).indexOf(n) + 1) + ')';
      out.unshift(sel);
      n = p;
    }
    return out.join(' > ');
  };
  const textOf = (el) => (el.textContent || '').replace(/\\s+/g, ' ').trim().slice(0, 48);
  // Page-level scrollers: .fleet-content is the pane that carries the mobile
  // nav, so when IT scrolls sideways the nav/toolbar/pagination scroll away
  // with the content -- a page overflow, not an acceptable inner scroller.
  const pageLevel = (n) => n === doc || n === document.body
    || (n.className || '').toString().split(' ').some((c) => c.startsWith('fleet-'));
  const clipped = (el) => {
    for (let n = el.parentElement; n; n = n.parentElement) {
      const cs = getComputedStyle(n);
      const scrolls = (cs.overflowX === 'auto' || cs.overflowX === 'scroll')
        && n.scrollWidth > n.clientWidth + TOL;
      if (cs.overflowX === 'hidden' || (scrolls && !pageLevel(n))) return true;
    }
    return false;
  };
  const pane = document.querySelector('.fleet-content');
  const doc = document.documentElement;
  const offenders = [];
  for (const el of doc.querySelectorAll('*')) {
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0 || r.right <= EDGE + TOL) continue;
    if (clipped(el)) continue;
    offenders.push({ path: pathOf(el), text: textOf(el), right: Math.round(r.right),
      left: Math.round(r.left), width: Math.round(r.width) });
  }
  offenders.sort((a, b) => b.right - a.right || b.width - a.width);
  return {
    edge: Math.round(EDGE), innerWidth: Math.round(window.innerWidth),
    docScroll: Math.round(doc.scrollWidth), docClient: Math.round(doc.clientWidth),
    bodyScroll: Math.round(document.body ? document.body.scrollWidth : 0),
    offenders: offenders.slice(0, 6), offenderCount: offenders.length,
    paneScroll: pane ? Math.round(pane.scrollWidth) : 0,
    paneClient: pane ? Math.round(pane.clientWidth) : 0,
  };
})()`;

const overflowOf = (r) => r.docScroll > r.edge + 1 || r.bodyScroll > r.edge + 1
  || r.offenderCount > 0 || r.paneScroll > r.paneClient + 1;

// ---------------------------------------------------------------------------
// Mobile-nav driving. Coordinates are avoided: on an overflowing page click
// points land outside the visual viewport, so everything is a DOM click / key.
// ---------------------------------------------------------------------------
// Distinct visible labels of `sel` (title attr wins: antd Select options).
const labels = (page, sel) => page.evaluate((s) => {
  const lab = (e) => (e.getAttribute('title') || e.textContent || '').replace(/\\s+/g, ' ').trim();
  return Array.from(document.querySelectorAll(s)).map(lab).filter((t, i, a) => t && a.indexOf(t) === i);
}, sel);

// Real DOM click on the first element matching `sel` whose label is `want`.
const clickLabel = (page, sel, want) => page.evaluate(([s, w]) => {
  const lab = (e) => (e.getAttribute('title') || e.textContent || '').replace(/\\s+/g, ' ').trim();
  const el = Array.from(document.querySelectorAll(s)).find((e) => lab(e) === w);
  if (!el) return false;
  el.click();
  return true;
}, [sel, want]);

// antd 6 drawers have no .ant-drawer-content: the open root carries
// .ant-drawer-open and the panel is .ant-drawer-content-wrapper > .ant-drawer-section.
const OVERLAYS = '.ant-drawer-open, .ant-modal-wrap, .ant-modal-content';
const overlayCount = (page) => page.evaluate((sel) => Array.from(
  document.querySelectorAll(sel),
).filter((el) => el.checkVisibility()).length, OVERLAYS);

async function closeOverlay(page) {
  for (let i = 0; i < 3 && (await overlayCount(page)) > 0; i += 1) {
    await page.keyboard.press('Escape');
    await pause(350);
    if (await overlayCount(page)) {
      await page.evaluate(() => document.querySelector('.ant-modal-close, .ant-drawer-close')?.click());
      await pause(350);
    }
  }
  return overlayCount(page);
}

async function measure(page, tag, results) {
  const r = await page.evaluate(PROBE);
  const overflow = overflowOf(r);
  console.log(`    ${overflow ? 'FAIL' : 'PASS'} ${tag} doc=${r.docScroll}/${r.edge}`
    + ` pane=${r.paneScroll}/${r.paneClient} inner=${r.innerWidth}`
    + (overflow ? ` offenders=${r.offenderCount}` : ''));
  for (const o of r.offenders) {
    console.log(`      right=${o.right} left=${o.left} w=${o.width} ${o.path}`
      + (o.text ? ` "${o.text}"` : ''));
  }
  results.push({ tag, ...r, overflow });
  return overflow;
}

// Sequenced prefix: page names are CJK, which strips to '_' and would collide.
let shotSeq = 0;
async function screenshot(page, name) {
  const file = `${String(++shotSeq).padStart(2, '0')}-${name.replace(/[^\w.-]+/g, '_')}.png`;
  try {
    mkdirSync(SHOTS, { recursive: true });
    await page.screenshot({ path: path.join(SHOTS, file) });
  } catch (e) { console.log(`    screenshot failed (${name}): ${e.message}`); }
}

// Non-active tabs of the current page (the active one is the base measurement).
const inactiveTabs = (page) => page.evaluate((s) => {
  const lab = (e) => (e.textContent || '').replace(/\\s+/g, ' ').trim();
  return Array.from(document.querySelectorAll(s))
    .filter((e) => !e.className.includes('ant-tabs-tab-active'))
    .map(lab).filter((t, i, a) => t && a.indexOf(t) === i);
}, TABS);

// Click the first overlay opener (新建/创建/...) inside the page body, if any.
const openOverlay = (page) => page.evaluate(() => {
  const lab = (e) => (e.textContent || '').replace(/\\s+/g, ' ').trim();
  const btn = Array.from(document.querySelectorAll('.fleet-content button'))
    .find((e) => /^(新建|创建|添加|新增|导入)/.test(lab(e)) && lab(e).length <= 12);
  if (!btn) return '';
  btn.click();
  return lab(btn);
});

async function checkPage(page, name, results) {
  await measure(page, name, results);
  const opened = await openOverlay(page);
  if (opened) {
    // Overlays that fetch before opening (e.g. DAG 新建定义) appear seconds
    // later, so poll; measure only once the slide-in motion ends, or a drawer is
    // caught off-screen at left=390 and reports an overflow it does not have.
    // Actions revealing an inline form (todoPanel 新建模板) change the layout
    // too, so the post-click state is measured either way.
    for (let i = 0; i < 5 && !(await overlayCount(page)); i += 1) await pause(400);
    await page.waitForFunction(() => !document.querySelector('[class*="-motion-"]'),
      null, { timeout: 3000 }).catch(() => {});
    const kind = (await overlayCount(page)) > 0 ? 'overlay' : 'after';
    await measure(page, `${name} ${kind}:${opened}`, results);
    const left = await closeOverlay(page);
    if (left) console.log(`    note: overlay "${opened}" stayed open (${left})`);
  }
  if (await overlayCount(page)) await closeOverlay(page); // never walk tabs behind one
  for (const tab of await inactiveTabs(page)) {
    if (await clickLabel(page, TABS, tab)) {
      await pause(700);
      await measure(page, `${name} tab:${tab}`, results);
    }
  }
  await screenshot(page, name);
}

// Page options of the selected category; leaves the dropdown closed.
async function pagesOf(page) {
  await page.locator(`${SELECT} input`).focus();
  await page.keyboard.press('ArrowDown');
  await page.waitForSelector(OPTIONS, { timeout: 4000 });
  const pages = await labels(page, OPTIONS);
  await page.keyboard.press('Escape');
  await pause(150);
  return pages;
}

// Open the page Select, retrying once behind a stray overlay that eats keys.
async function openSelect(page) {
  for (let i = 0; i < 2; i += 1) {
    await page.locator(`${SELECT} input`).focus();
    await page.keyboard.press('ArrowDown');
    await pause(300);
    if (await page.$(OPTIONS)) return true;
    await closeOverlay(page);
  }
  return false;
}

const selectValue = (page) => page.evaluate((s) => {
  const el = document.querySelector(`${s} .ant-select-content`) || document.querySelector(s);
  return el ? (el.getAttribute('title') || el.textContent || '').trim() : '';
}, SELECT);

async function pickPage(page, want) {
  for (let attempt = 0; attempt < 2 && (await selectValue(page)) !== want; attempt += 1) {
    await closeOverlay(page); // a stray drawer/modal swallows focus + keys
    if (!(await openSelect(page))) continue;
    if (!(await clickLabel(page, OPTIONS, want))) await page.keyboard.press('Escape');
    await pause(900);
  }
  return (await selectValue(page)) === want;
}

async function main() {
  if (!existsSync(path.join(SPA, 'index.html'))) {
    throw new Error(`SPA bundle not found at ${SPA} (run: cd crates/web/spa && npm install && npm run build)`);
  }
  const { chromium } = require(path.resolve(__dirname, '../../crates/web/spa/node_modules/playwright-core'));
  const srv = await serve(PORT);
  const base = `http://127.0.0.1:${PORT}`;
  const browser = await chromium.launch({
    headless: !HEADED, executablePath: process.env.CHROME || '/usr/bin/chromium-browser',
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage({
    viewport: { width: WIDTH, height: HEIGHT }, isMobile: true, hasTouch: true, deviceScaleFactor: 2,
  });
  page.setDefaultTimeout(20000);
  const consoleErrors = [];
  const pageErrors = [];
  page.on('console', (m) => { if (m.type() === 'error') consoleErrors.push(m.text().slice(0, 200)); });
  page.on('pageerror', (e) => {
    const text = String(e && e.message ? e.message : e).slice(0, 160);
    pageErrors.push(text);
    consoleErrors.push(`pageerror: ${text}`);
  });

  const visited = [];
  const results = [];
  let failed = false;
  try {
    await page.goto(base, { waitUntil: 'domcontentloaded' });
    await page.evaluate((tok) => {
      localStorage.setItem('oc_token', tok);
      localStorage.setItem('oc_base', '');
    }, TOKEN);
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.waitForSelector('.fleet-mobile-nav', { timeout: 15000 });
    await pause(1200);

    const segments = await labels(page, SEGMENT);
    console.log(`viewport=${WIDTH}x${HEIGHT} base=${base} shots=${SHOTS}`);
    console.log(`nav categories: ${segments.join(' | ')}`);
    if (!segments.length) throw new Error('mobile nav has no categories');

    for (const seg of segments) {
      if (!(await clickLabel(page, SEGMENT, seg))) {
        console.log(`  !! cannot select category ${seg}`);
        failed = true;
        continue;
      }
      await pause(400);
      const pages = await pagesOf(page);
      console.log(`category ${seg}: ${pages.join(' | ')}`);
      for (const want of pages) {
        if (ONLY && !want.includes(ONLY)) continue;
        if (!(await pickPage(page, want))) {
          console.log(`  !! cannot navigate to ${seg}/${want}`);
          failed = true;
          continue;
        }
        visited.push(`${seg}/${want}`);
        const before = results.length;
        const errBefore = pageErrors.length;
        await checkPage(page, `${seg}-${want}`, results);
        const bad = results.slice(before).filter((r) => r.overflow);
        const crashed = pageErrors.slice(errBefore);
        failed = failed || bad.length > 0 || crashed.length > 0;
        for (const c of crashed) console.log(`    CRASH ${c}`);
        console.log(`  ${bad.length || crashed.length ? 'FAIL' : 'OK  '} ${seg}/${want}`
          + (bad.length ? `: ${bad.map((b) => b.tag).join(', ')}` : ''));
      }
    }
    console.log(`visited ${visited.length} pages: ${visited.join(', ')}`);
    const overflows = results.filter((r) => r.overflow);
    console.log(`\nSUMMARY measurements=${results.length} overflowing=${overflows.length}`);
    for (const r of overflows) {
      console.log(`  ${r.tag} doc=${r.docScroll}/${r.edge} pane=${r.paneScroll}/${r.paneClient} offenders=${r.offenderCount}`);
    }
    if (consoleErrors.length) {
      console.log(`console errors (${consoleErrors.length}): ${consoleErrors.slice(0, 6).join(' ;; ')}`);
    }
    if (!visited.length) throw new Error('no mobile-nav page was visited');
  } finally {
    if (!KEEP) await browser.close();
    await srv.close();
  }
  if (failed) {
    console.error('FAIL: horizontal overflow (or unreachable page) at phone viewport');
    process.exit(1);
  }
  console.log('PASS: every visited page fits the 390px viewport');
  process.exit(0);
}

main().catch((e) => {
  console.error('ABORT:', e && e.message ? e.message : e);
  process.exit(2);
});
