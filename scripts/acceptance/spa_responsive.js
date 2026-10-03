const path = require('node:path');
const { existsSync } = require('node:fs');
const REPO = path.resolve(__dirname, '../..');
const SPA = path.join(REPO, 'crates/web/spa/dist');
function arg(flag, fallback) { const index = process.argv.indexOf(`--${flag}`); return index >= 0 ? process.argv[index + 1] : fallback; }
const WIDTH = Number(arg('width', '390'));
const HEIGHT = Number(arg('height', '1080'));
const TOKEN = 'fixture-token';
const SEGMENT = WIDTH < 768 ? '.fleet-mobile-nav[role="tablist"] [role="tab"]' : '.fleet-nav-category [role="tab"]';
const SELECT = '.ant-select.fleet-mobile-nav';
const OPTIONS = '.ant-select-dropdown:not(.ant-select-dropdown-hidden) .ant-select-item-option';
const TABS = '.fleet-content .ant-tabs-tab';
const HEADED = process.argv.includes('--headed');
const KEEP = process.argv.includes('--keep');
const REQUIRE_COMMITTED = process.argv.includes('--require-committed');
const DRIFT = process.argv.includes('--drift');
const PORT = Number(arg('port', '18099'));
const SHOTS = arg('shots', '/tmp/uitest/responsive');
const ONLY = arg('only', '');
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const { serve, bundleProvenance, runDriftCheck } = require('./ui/responsive/services')(REPO, TOKEN);
const { labels, clickLabel, checkPage, pagesOf, pickPage } = require('./ui/responsive/probes')({ SHOTS, SEGMENT, SELECT, OPTIONS, TABS });
async function main() {
  if (!existsSync(path.join(SPA, 'index.html'))) {
    throw new Error(`SPA bundle not found at ${SPA} (run: cd crates/web/spa && npm install && npm run build)`);
  }
  // Pre-flight: say out loud which bundle is about to be measured, and refuse
  // to measure an unpinned one when the caller asked for the commit itself.
  const provenance = bundleProvenance();
  console.log(provenance.line);
  if (provenance.warn) console.warn(provenance.warn);
  if (REQUIRE_COMMITTED && !provenance.pinned) {
    console.error(`REFUSING (--require-committed): ${provenance.refusal}.`
      + ' Run scripts/build-spa.sh, commit dist/, and re-run to measure HEAD.');
    process.exit(2);
  }
  if (DRIFT) runDriftCheck();
  const { chromium } = require(path.resolve(REPO, 'crates/web/spa/node_modules/playwright-core'));
  const srv = await serve(PORT);
  const base = `http://127.0.0.1:${PORT}`;
  const browser = await chromium.launch({
    headless: !HEADED, executablePath: process.env.CHROME_PATH || chromium.executablePath(),
    args: ['--no-sandbox', '--disable-dev-shm-usage'],
  });
  const page = await browser.newPage({
    viewport: { width: WIDTH, height: HEIGHT }, isMobile: WIDTH < 768, hasTouch: WIDTH < 768, deviceScaleFactor: 2,
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
    await page.waitForSelector(WIDTH < 768 ? '.fleet-mobile-nav' : '#fleet-page-menu', { timeout: 15000 });
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
      const pages = WIDTH < 768 ? await pagesOf(page) : await labels(page, '.fleet-sidebar .ant-menu-item');
      console.log(`category ${seg}: ${pages.join(' | ')}`);
      for (const want of pages) {
        if (ONLY && !want.includes(ONLY)) continue;
        if (!(WIDTH < 768 ? await pickPage(page, want) : await clickLabel(page, '.fleet-sidebar .ant-menu-item', want))) {
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
    if (!visited.length) throw new Error('no navigation page was visited');
    const { NAV_CATEGORIES } = await import(path.join(REPO, 'crates/web/spa/src/nav.js'));
    const expected = NAV_CATEGORIES.flatMap((category) => category.items.map((item) => `${category.label}/${item.menu}`));
    if (!ONLY && (visited.length !== expected.length || expected.some((name) => !visited.includes(name)))) throw new Error('navigation coverage is incomplete');
    if (srv.fixtureMisses.length) throw new Error(`unhandled fixture requests: ${srv.fixtureMisses.join(', ')}`);
    const receipt = { passed: !failed, viewport: { width: WIDTH, height: HEIGHT }, visited, measurements: results, fixture_misses: srv.fixtureMisses, page_errors: pageErrors };
    require('node:fs').writeFileSync(path.join(SHOTS, 'receipt.json'), JSON.stringify(receipt, null, 2));
  } finally {
    if (!KEEP) await browser.close();
    await srv.close();
  }
  if (failed) {
    console.error('FAIL: horizontal overflow (or unreachable page) at phone viewport');
    process.exit(1);
  }
  console.log(`PASS: every registered page fits the ${WIDTH}px viewport`);
  process.exit(0);
}

main().catch((e) => {
  console.error('ABORT:', e && e.message ? e.message : e);
  process.exit(2);
});
