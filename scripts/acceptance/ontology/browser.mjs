import assert from 'node:assert/strict';
import { checkDetails, checkGraph } from './experience.mjs';
import { chromium } from '../../../crates/web/spa/node_modules/playwright-core/index.mjs';

const [base, output] = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
const page = await browser.newPage({ viewport: { width: 1200, height: 850 } });
const failures = [];
page.on('pageerror', (error) => failures.push(error.message));
const dialog = page.locator('.ant-modal[role="dialog"]:visible');
// Sider page menu; scoping avoids the brand panel's copies of the same items while it closes.
const sider = page.locator('#fleet-page-menu');
const headerless = async (ready) => {
  await ready.waitFor();
  assert.equal(await page.locator('.oc-page-title, .oc-page-desc').count(), 0);
  assert.equal(await page.getByRole('combobox', { name: 'Ontology 环境', exact: true }).count(), 0);
  assert.equal(await page.getByRole('menuitem', { name: /环境管理$/ }).count(), 0);
};
const choose = async (scope, label, text, multiple = false) => {
  await scope.getByLabel(label, { exact: true }).click();
  const dropdown = page.locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)');
  await dropdown.locator('.ant-select-item-option').getByText(text, { exact: true }).click();
  if (multiple) await scope.locator('.ant-modal-title').click();
  await dropdown.waitFor({ state: 'hidden' });
};
const submit = async () => {
  await dialog.getByRole('button', { name: /确\s*定/ }).click();
  await dialog.waitFor({ state: 'hidden' });
};
try {
  await page.addInitScript(() => localStorage.setItem('oc_ontology_env', JSON.stringify('browser_env')));
  await page.goto(`${base}/?token=ontology-fixture`);
  // Desktop: the Sider carries no category tablist; every category is reached through the
  // brand panel (shell/brandNav.jsx). Tab overflow is still verified below on the mobile nav.
  assert.equal(await page.locator('.fleet-sidebar [role=tablist]').count(), 0);
  await page.locator('.fleet-brand').hover();
  const panel = page.locator('.fleet-nav-panel');
  const category = await panel.evaluate((element) => [...element.querySelectorAll('[role=menuitem]')].map((item) => item.textContent.trim()));
  for (const name of ['图谱', '实体', '实体类型', '关系类型', '数据保鲜']) assert(category.includes(name), `brand panel lacks ${name}`);
  await panel.getByRole('menuitem', { name: '图谱', exact: true }).click();
  await headerless(page.getByRole('button', { name: '调整范围', exact: true }));
  await sider.getByRole('menuitem', { name: /实体类型$/ }).click();
  await headerless(page.getByRole('cell', { name: '服务', exact: true }));
  await page.getByRole('button', { name: /新\s*增/ }).click();
  await dialog.getByLabel('类型 Key').fill('browser_type');
  await dialog.getByLabel('名称', { exact: true }).fill('浏览器类型');
  await dialog.getByRole('button', { name: /确\s*定/ }).click();
  await dialog.waitFor({ state: 'hidden' });
  await page.getByRole('cell', { name: '浏览器类型', exact: true }).waitFor();
  await page.getByRole('cell', { name: '浏览器类型', exact: true }).click();
  const drawer = page.locator('.ant-drawer-open');
  await drawer.getByRole('tab', { name: '属性', exact: true }).click();
  await drawer.getByRole('button', { name: /新增属性$/ }).click();
  await dialog.getByLabel('属性 Key').fill('owner');
  await dialog.getByLabel('名称', { exact: true }).fill('负责人');
  await choose(dialog, '类型', 'string');
  await submit();
  await drawer.getByRole('cell', { name: '负责人', exact: true }).waitFor();
  await drawer.getByRole('tab', { name: 'Action', exact: true }).click();
  await drawer.getByRole('button', { name: /新增 Action$/ }).click();
  await choose(dialog, '读写', '读');
  await dialog.getByLabel('操作', { exact: true }).fill('read_status');
  await submit();
  await drawer.getByText('read: read_status', { exact: true }).waitFor();
  await drawer.locator('.ant-drawer-close').click();
  await drawer.waitFor({ state: 'hidden' });
  await sider.getByRole('menuitem', { name: /实体$/ }).click();
  // The Server seeds capability entities on start, so the 20-row table paginates; rows are looked up through the search box.
  const search = page.getByRole('textbox', { name: '搜索名称或标识' });
  await search.fill('入口');
  await headerless(page.getByRole('cell', { name: '入口服务', exact: true }));
  await page.getByRole('cell', { name: '入口服务', exact: true }).waitFor();
  await page.getByRole('button', { name: /新增实体$/ }).click();
  await choose(dialog, '实体类型', '浏览器类型');
  await dialog.getByLabel('名称', { exact: true }).fill('浏览器实体');
  await dialog.getByLabel('来源', { exact: true }).fill('# 浏览器来源');
  await dialog.getByLabel('扩展信息（Markdown）', { exact: true }).fill('# 浏览器扩展');
  await dialog.getByLabel('负责人', { exact: true }).fill('tester');
  await submit();
  await search.fill('浏览器实体');
  await page.getByRole('cell', { name: '浏览器实体', exact: true }).waitFor();
  await checkDetails(page);
  await search.fill('入口');
  assert.equal(await page.getByRole('cell', { name: '下游服务', exact: true }).count(), 0);
  await sider.getByRole('menuitem', { name: /关系类型$/ }).click();
  await headerless(page.getByRole('cell', { name: '调用', exact: true }));
  await page.getByRole('button', { name: /新\s*增/, exact: true }).click();
  await dialog.getByLabel('类型 Key').fill('browser_relation');
  await dialog.getByLabel('名称', { exact: true }).fill('浏览器关系');
  await choose(dialog, '源实体类型', '服务');
  await choose(dialog, '允许目标类型', '服务', true);
  await submit();
  const relationRow = page.getByRole('row').filter({ has: page.getByRole('cell', { name: '浏览器关系', exact: true }) });
  await relationRow.getByRole('button', { name: '创建关系', exact: true }).click();
  await choose(dialog, '源实体', '入口服务');
  await choose(dialog, '目标实体', '下游服务');
  await submit();
  await sider.getByRole('menuitem', { name: /图谱$/ }).click();
  // The Server seeds a built-in aspect, so once metadata and aspects load the page restores to
  // 切面观测; the full custom filters live on the 自定义观测 tab.
  await page.waitForFunction(() => { const refresh = document.querySelector('button[aria-label="刷新当前观测"]');
    return refresh && !refresh.classList.contains('ant-btn-loading'); });
  await page.getByRole('tab', { name: '自定义观测', exact: true }).click();
  assert.equal(await page.getByRole('checkbox', { name: '展开跨类型邻居' }).count(), 0);
  await page.getByRole('button', { name: '调整范围', exact: true }).click();
  const types = page.getByRole('combobox', { name: '实体类型多选' });
  await types.click();
  await page.locator('.ant-select-dropdown:visible .ant-select-item-option').getByText('服务', { exact: true }).click();
  await page.locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)').getByRole('button', { name: /完\s*成/ }).click();
  await page.locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)').waitFor({ state: 'hidden' });
  const centers = page.getByRole('combobox', { name: '实体多选' });
  await centers.click();
  await page.locator('.ant-select-dropdown:visible .ant-select-item-option').getByText('入口服务', { exact: true }).click();
  await page.locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)').getByRole('button', { name: /完\s*成/ }).click();
  await page.locator('.ant-select-dropdown:not(.ant-select-dropdown-hidden)').waitFor({ state: 'hidden' });
  await page.locator('canvas:visible').first().waitFor();
  await page.locator('.ant-popover:visible').getByRole('button', { name: /^完\s*成$/ }).click();
  await checkGraph(page);
  await page.getByRole('button', { name: '保存切面', exact: true }).click();
  await dialog.getByLabel('名称', { exact: true }).fill('浏览器切面');
  await dialog.getByRole('button', { name: /保\s*存/, exact: true }).click();
  await dialog.waitFor({ state: 'hidden' });
  await page.getByRole('tab', { name: '切面观测', exact: true }).click();
  await choose(page, '切面选择', '浏览器切面');
  await page.locator('canvas:visible').first().waitFor();
  await page.screenshot({ path: `${output}/desktop.png` });
  await sider.getByRole('menuitem', { name: /实体类型$/ }).click();
  await headerless(page.getByRole('cell', { name: '浏览器类型', exact: true }));
  await page.setViewportSize({ width: 240, height: 740 });
  const mobile = page.locator('.fleet-mobile-nav[role=tablist]');
  await mobile.getByRole('tab', { name: 'Ontology', exact: true }).click();
  await mobile.hover();
  await page.mouse.wheel(80, 0);
  await page.waitForFunction(() => document.querySelector('.fleet-mobile-nav[role=tablist]').scrollLeft > 0);
  const narrow = await mobile.evaluate((element) => ({ width: element.clientWidth, total: element.scrollWidth, left: element.scrollLeft,
    wrap: getComputedStyle(element).flexWrap, labels: [...element.children].map((tab) => tab.scrollWidth <= tab.clientWidth) }));
  assert(narrow.total > narrow.width && narrow.left > 0 && narrow.wrap === 'nowrap');
  assert(narrow.labels.every(Boolean));
  await page.screenshot({ path: `${output}/mobile.png` });
  assert.deepEqual(failures, []);
  console.log(JSON.stringify({ passed: true, pages: 4, attribute_action_config: true, entity_created: true,
    relationship_created: true, aspect_saved: true, draft_guard: true, detail_tabs: true, graph_controls: true, observation_restored: true, server_environment: true, desktop: category, mobile: narrow }));
} catch (error) {
  await page.screenshot({ path: `${output}/browser-failure.png` });
  console.error(JSON.stringify({ error: error.message, failures }));
  throw error;
} finally { await browser.close(); }
