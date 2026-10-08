const path = require('node:path');
const { mkdirSync } = require('node:fs');
module.exports = function probes({ SHOTS, SEGMENT, SELECT, OPTIONS, TABS }) {
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
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
    || (!n.classList.contains('fleet-category-tabs')
      && (n.className || '').toString().split(' ').some((c) => c.startsWith('fleet-')));
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
).filter((el) => el.getBoundingClientRect().width > 0 && getComputedStyle(el).visibility !== 'hidden').length, OVERLAYS);

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
    await page.screenshot({ path: path.join(SHOTS, file), animations: 'disabled', timeout: 60000 });
  } catch (failure) { throw new Error(`screenshot failed (${name}): ${failure.message}`); }
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
    await pause(500);
    await page.waitForFunction(() => [...document.querySelectorAll('.ant-drawer-open, .ant-modal-wrap')].every((overlay) => overlay.getAnimations({ subtree: true }).every((animation) => animation.playState !== 'running' || animation.effect.getTiming().iterations === Infinity)),
      null, { timeout: 20000 });
    // A loaded browser can defer the first animation frame. An empty Web
    // Animations list alone does not mean the drawer has finished entering.
    await page.waitForFunction(() => !document.querySelector('[class*="-motion-"]'),
      null, { timeout: 20000 });
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


return { labels, clickLabel, checkPage, pagesOf, pickPage };
};
