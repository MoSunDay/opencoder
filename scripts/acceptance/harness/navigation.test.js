// navigation.test.js — the shared page-navigation helper against the SPA
// source of truth (crates/web/spa/src/nav.js) and a scripted stand-in for the
// Playwright page; no browser and no dist bundle are needed here.
const assert = require('node:assert/strict');
const { test } = require('node:test');
const { categoryOfPage, navCategories, openPage } = require('./navigation');

const SIDER = '#fleet-page-menu';
const PANEL_PAGE = '.fleet-nav-panel > role=menuitem';
const SIDER_PAGE = `${SIDER} > role=menuitem`;
const TAB = '.fleet-mobile-nav[role="tablist"] [role="tab"]';
const ACTIVE_TAB = `${TAB}[aria-selected="true"]`;
const SELECT_INPUT = '.ant-select.fleet-mobile-nav input';
const OPTION = '.ant-select-dropdown:visible .ant-select-item-option-content';
const escapeRe = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');

// A scripted stand-in for the Playwright page: locator steps accumulate into
// one key (`a > b`), terminal calls read `world.visible` / `world.text` /
// `world.count` by that key and append to `world.log`, so a test asserts the
// exact walk openPage() performs. `world.onClick(key)` mutates the world the
// way the SPA would after that click; `world.waitFor(state, key)` throws to
// script a Playwright failure.
function fakePage(world) {
  const at = (steps) => {
    const key = steps.join(' > ');
    return {
      locator: (selector) => at([...steps, selector]),
      getByRole: (role, { name, exact = false } = {}) => at([...steps, `role=${role} ${String(name)}${exact ? ' exact' : ''}`]),
      getByText: (text, { exact = false } = {}) => at([...steps, `text=${String(text)}${exact ? ' exact' : ''}`]),
      hover: () => { world.log.push(`hover ${key}`); },
      click: () => { world.log.push(`click ${key}`); world.onClick?.(key); },
      isVisible: () => Boolean(world.visible?.[key]),
      count: () => world.count?.[key] ?? 0,
      textContent: () => world.text?.[key] ?? '',
      waitFor: ({ state }) => { world.log.push(`wait ${state} ${key}`); world.waitFor?.(state, key); },
    };
  };
  // The real page also exposes the viewport and the mouse (pointer parking
  // before a brand hover); 1440 wide parks the pointer at x = 720. A world
  // sets `width: 390` to script the narrow layout (<768px: tabs + Select).
  return Object.assign(at([]), {
    viewportSize: () => ({ width: world.width ?? 1440, height: 900 }),
    mouse: { move: (x, y) => world.log.push(`move ${x},${y}`) },
  });
}

const loaded = navCategories();
const categoryByKey = async (key) => (await loaded).find((category) => category.key === key);

test('navCategories() loads NAV_CATEGORIES from the SPA source of truth once per process', async () => {
  const categories = await loaded;
  assert.equal(navCategories(), loaded);
  assert.deepEqual(categories.map((category) => category.key), ['project', 'agent', 'ontology', 'admin']);
  for (const category of categories) {
    assert(typeof category.label === 'string' && category.label.trim() === category.label, category.key);
    assert(category.items.length >= 1, `${category.key} needs a landing page`);
    for (const item of category.items) assert(typeof item.menu === 'string' && item.menu.trim() === item.menu, item.page);
  }
  // String labels are matched with role={ name, exact }: page labels must not
  // repeat, and a category label may only coincide with a page of its own.
  const pageLabels = categories.flatMap((category) => category.items.map((item) => item.menu));
  assert.equal(new Set(pageLabels).size, pageLabels.length, 'page labels must stay unambiguous for role-name matching');
  for (const category of categories) {
    const owner = categoryOfPage(categories, category.label);
    assert(owner === undefined || owner === category, `${category.label} collides with another category's page`);
  }
});

test('every page label resolves back to its own category, as a string and as a RegExp', async () => {
  const categories = await loaded;
  for (const category of categories) {
    for (const item of category.items) {
      assert.equal(categoryOfPage(categories, item.menu), category, item.menu);
      assert.equal(categoryOfPage(categories, new RegExp(`^${escapeRe(item.menu)}$`)), category, item.menu);
    }
  }
});

test("'Agent' resolves to the Agent category through its chat page, never through the category label", async () => {
  const categories = await loaded;
  const agent = await categoryByKey('agent');
  assert.equal(categoryOfPage(categories, 'Agent'), agent);
  assert.equal(categoryOfPage(categories, /^Agent$/), agent);
  assert.equal(agent.items.find((item) => item.menu === 'Agent').page, 'chat');
  // A category label that is not a page label is not a navigation target.
  for (const category of categories) {
    if (!category.items.some((item) => item.menu === category.label)) {
      assert.equal(categoryOfPage(categories, category.label), undefined, category.label);
    }
  }
});

test("'实体' resolves to ontology; unknown or partial labels resolve to nothing", async () => {
  const categories = await loaded;
  const ontology = await categoryByKey('ontology');
  assert.equal(categoryOfPage(categories, '实体'), ontology);
  assert.equal(categoryOfPage(categories, /^实体$/), ontology);
  assert.notEqual(categoryOfPage(categories, '实体'), categoryOfPage(categories, '实体类型') === ontology ? undefined : ontology);
  assert.equal(categoryOfPage(categories, /^实体/), ontology, 'a RegExp is matched, not compared');
  assert.equal(categoryOfPage(categories, '不存在的页面'), undefined);
  assert.equal(categoryOfPage(categories, 'Agent 配'), undefined, 'string labels match exactly');
  assert.equal(categoryOfPage(categories, /不存在的页面/), undefined);
  assert.equal(categoryOfPage(categories, ''), undefined);
});

test('openPage() walks brand → category tab → Sider page when the page is outside the active category', async () => {
  const world = {
    log: [],
    visible: { [SIDER]: true },
    // Picking the 项目 tab in the brand panel scopes the Sider to that category.
    onClick: (key) => { if (key === `${PANEL_PAGE} 项目 exact`) world.visible[`${SIDER_PAGE} Overview exact`] = true; },
  };
  await openPage(fakePage(world), 'Overview');
  assert.deepEqual(world.log, [
    `wait visible ${SIDER}`,
    'move 720,2',
    'hover .fleet-brand',
    `wait visible ${PANEL_PAGE} 项目 exact`,
    `click ${PANEL_PAGE} 项目 exact`,
    `wait hidden ${PANEL_PAGE} 项目 exact`,
    `click ${SIDER_PAGE} Overview exact`,
  ]);
});

test('openPage() only clicks the Sider page when its category is already active', async () => {
  const world = { log: [], visible: { [SIDER]: true, [`${SIDER_PAGE} Tag exact`]: true } };
  await openPage(fakePage(world), 'Tag');
  assert.deepEqual(world.log, [`wait visible ${SIDER}`, `click ${SIDER_PAGE} Tag exact`]);
});

test('openCategory() falls back to clicking the brand when hovering does not open the panel', async () => {
  let waits = 0;
  const world = {
    log: [],
    visible: { [SIDER]: true },
    waitFor: (state, key) => {
      // Only the brand panel tab fails to appear; the shell wait up front succeeds.
      if (state === 'visible' && key === `${PANEL_PAGE} 项目 exact` && waits++ === 0) throw Object.assign(new Error('Timeout 3000ms exceeded.'), { name: 'TimeoutError' });
    },
    onClick: (key) => { if (key === `${PANEL_PAGE} 项目 exact`) world.visible[`${SIDER_PAGE} Overview exact`] = true; },
  };
  await openPage(fakePage(world), 'Overview');
  assert.deepEqual(world.log, [
    `wait visible ${SIDER}`,
    'move 720,2',
    'hover .fleet-brand',
    `wait visible ${PANEL_PAGE} 项目 exact`,
    'click .fleet-brand',
    `wait visible ${PANEL_PAGE} 项目 exact`,
    `click ${PANEL_PAGE} 项目 exact`,
    `wait hidden ${PANEL_PAGE} 项目 exact`,
    `click ${SIDER_PAGE} Overview exact`,
  ]);
});

test('openPage() switches the category tab, then picks the page Select when the viewport is narrow', async () => {
  // The right category is already selected: only the page Select is touched.
  const same = { log: [], width: 390, count: { [ACTIVE_TAB]: 1 }, text: { [ACTIVE_TAB]: ' Agent ' } };
  await openPage(fakePage(same), /^Agent$/);
  assert.deepEqual(same.log, [
    `wait visible ${TAB}`,
    `click ${SELECT_INPUT}`,
    `click ${OPTION} > text=/^Agent$/ exact`,
  ]);
  // Another category is selected: its tab is clicked first.
  const other = { log: [], width: 390, count: { [ACTIVE_TAB]: 1 }, text: { [ACTIVE_TAB]: '后台管理' } };
  await openPage(fakePage(other), 'Agent 配置');
  assert.deepEqual(other.log, [
    `wait visible ${TAB}`,
    `click ${TAB} > text=Agent exact`,
    `click ${SELECT_INPUT}`,
    `click ${OPTION} > text=Agent 配置 exact`,
  ]);
  // No tab reports aria-selected: the category tab is clicked too.
  const none = { log: [], width: 390 };
  await openPage(fakePage(none), '实体');
  assert.deepEqual(none.log, [
    `wait visible ${TAB}`,
    `click ${TAB} > text=Ontology exact`,
    `click ${SELECT_INPUT}`,
    `click ${OPTION} > text=实体 exact`,
  ]);
});

test('openPage() waits for the shell instead of mistaking a not-yet-mounted Sider for a narrow viewport', async () => {
  // Straight after goto(): the Sider mounts only after /api/me, so isVisible()
  // is false at first sight and the tabs never appear at desktop width.
  const world = {
    log: [],
    waitFor: (state, key) => { if (key === SIDER && state === 'visible') world.visible = { [SIDER]: true }; },
    onClick: (key) => { if (key === `${PANEL_PAGE} 项目 exact`) world.visible[`${SIDER_PAGE} Overview exact`] = true; },
  };
  await openPage(fakePage(world), 'Overview');
  assert.deepEqual(world.log, [
    `wait visible ${SIDER}`,
    'move 720,2',
    'hover .fleet-brand',
    `wait visible ${PANEL_PAGE} 项目 exact`,
    `click ${PANEL_PAGE} 项目 exact`,
    `wait hidden ${PANEL_PAGE} 项目 exact`,
    `click ${SIDER_PAGE} Overview exact`,
  ]);
});

test('openPage() surfaces non-timeout failures and rejects unknown labels before touching the page', async () => {
  const broken = { log: [], visible: { [SIDER]: true }, waitFor: () => { throw Object.assign(new Error('detached frame'), { name: 'Error' }); } };
  await assert.rejects(openPage(fakePage(broken), 'Overview'), /detached frame/);
  const untouched = { log: [] };
  await assert.rejects(openPage(fakePage(untouched), '不存在的页面'), /no page menu label matches 不存在的页面/);
  assert.deepEqual(untouched.log, []);
});
