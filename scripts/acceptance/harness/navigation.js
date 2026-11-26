// navigation.js — page navigation shared by browser acceptance scripts.
// The header brand menu lists the first-level tabs only (one entry per
// category; crates/web/spa/src/shell/brandNav.jsx). The pages of the active
// category live in the Sider Menu on desktop and in the 「页面导航」 Select
// below 768px, so openPage() walks the same two steps a user takes: brand →
// category tab, then page menu → page. Category ownership is read from the
// SPA source of truth (crates/web/spa/src/nav.js) so nothing here can drift.
const path = require('node:path');

const REPO = path.resolve(__dirname, '../../..');
const BRAND = '.fleet-brand';
const PANEL = '.fleet-nav-panel';
const SIDER = '#fleet-page-menu';
const CATEGORY_TAB = '.fleet-mobile-nav[role="tablist"] [role="tab"]';
const ACTIVE_CATEGORY_TAB = `${CATEGORY_TAB}[aria-selected="true"]`;
const PAGE_SELECT = '.ant-select.fleet-mobile-nav';
const PAGE_OPTIONS = '.ant-select-dropdown:visible .ant-select-item-option-content';
// The SPA's own breakpoint (app.css): below 768px the Sider is display:none
// and the Content carries the category tabs + page Select instead.
const NARROW_BELOW = 768;

let categoriesPromise;

/// NAV_CATEGORIES from the SPA module (ESM, loaded once per process).
function navCategories() {
  categoriesPromise ||= import(path.join(REPO, 'crates/web/spa/src/nav.js')).then((spa) => spa.NAV_CATEGORIES);
  return categoriesPromise;
}

/// The category owning a page menu label: an exact string, or a RegExp
/// matched against every page label in menu order.
function categoryOfPage(categories, name) {
  const matches = typeof name === 'string' ? (label) => label === name : (label) => name.test(label);
  return categories.find((category) => category.items.some((item) => matches(item.menu)));
}

/// Hover opens the panel on pointer ENTRY: a panel closed while the pointer
/// stayed on the trigger (Escape, a tab pick) never sees a second mouseenter,
/// so the brand would only react after the click fallback below. Park the
/// pointer in the middle of the header first — empty space far from the wordmark
/// and the header-side controls — so every hover is a real entry.
async function parkPointer(page) {
  const size = page.viewportSize();
  if (size) await page.mouse.move(Math.floor(size.width / 2), 2);
}

/// Brand → category tab. Hover opens the panel for fine pointers; a coarse
/// pointer only opens it on click.
async function openCategory(page, label) {
  const brand = page.locator(BRAND);
  const tab = page.locator(PANEL).getByRole('menuitem', { name: label, exact: true });
  await parkPointer(page);
  await brand.hover();
  try {
    await tab.waitFor({ state: 'visible', timeout: 3000 });
  } catch (failure) {
    if (failure.name !== 'TimeoutError') throw failure;
    await brand.click();
    await tab.waitFor({ state: 'visible' });
  }
  await tab.click();
  await tab.waitFor({ state: 'hidden' });
}

/// Which page menu this viewport uses, once the shell has rendered it. The
/// Sider and the tabs only mount after /api/me, so this auto-waits: a caller
/// that navigates straight after goto() must not be mistaken for a narrow
/// viewport (isVisible() on a not-yet-mounted Sider is simply false).
async function pageSurface(page) {
  const width = page.viewportSize()?.width ?? await page.evaluate(() => window.innerWidth);
  const narrow = width < NARROW_BELOW;
  await page.locator(narrow ? CATEGORY_TAB : SIDER).waitFor({ state: 'visible' });
  return narrow;
}

/// Open one page by its Sider menu label (exact string or RegExp).
async function openPage(page, name) {
  const categories = await navCategories();
  const category = categoryOfPage(categories, name);
  if (!category) throw new Error(`no page menu label matches ${name}`);
  if (await pageSurface(page)) {
    // Narrow viewport: category tabs + the page Select replace the Sider.
    const active = page.locator(ACTIVE_CATEGORY_TAB);
    if (!(await active.count()) || (await active.textContent()).trim() !== category.label) {
      await page.locator(CATEGORY_TAB).getByText(category.label, { exact: true }).click();
    }
    await page.locator(`${PAGE_SELECT} input`).click();
    await page.locator(PAGE_OPTIONS).getByText(name, { exact: true }).click();
    return;
  }
  const sider = page.locator(SIDER);
  const target = sider.getByRole('menuitem', { name, exact: true });
  if (!(await target.isVisible())) await openCategory(page, category.label);
  await target.click();
}

module.exports = { categoryOfPage, navCategories, openPage };
