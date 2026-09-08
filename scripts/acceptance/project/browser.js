const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const fs = require('fs');
const path = require('path');
const assert = require('assert/strict');
async function projectPage(page) {
  await page.locator('.fleet-nav-category').getByText('项目', { exact: true }).click();
  await page.getByRole('menuitem').filter({ hasText: '项目' }).click();
}
async function openBrowser(h, errors) {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const page = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript((token) => localStorage.setItem('oc_token', token), h.token);
  try {
    await page.goto(h.base, { waitUntil: 'networkidle' });
    await projectPage(page);
  } catch (error) {
    fs.writeFileSync(path.join(h.root, 'browser-failure.html'), await page.content());
    await page.screenshot({ path: path.join(h.root, 'browser-failure.png'), fullPage: true });
    await browser.close(); throw error;
  }
  return { browser, page };
}
async function createHierarchy(page) {
  async function save(route, button) {
    const dialog = page.locator('.ant-modal[role="dialog"]');
    const response = page.waitForResponse((r) => r.url().endsWith(route) && r.request().method() === 'POST');
    await dialog.getByRole('button', { name: button }).click();
    const saved = await response;
    if (!saved.ok()) throw new Error(`browser create ${route}: ${await saved.text()}`);
    await dialog.waitFor({ state: 'hidden' });
    return saved.json();
  }
  await page.getByRole('tab', { name: '项目目标', exact: true }).click();
  await page.getByRole('button', { name: '新建目标', exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Project replay acceptance');
  const goal = await save('/api/project/goals', /保\s*存/);
  await page.locator('.ant-card-head-title').filter({ hasText: 'Project replay acceptance' }).waitFor();
  await page.getByRole('button', { name: '新建目标', exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Another project');
  await save('/api/project/goals', /保\s*存/);
  await page.getByRole('tab', { name: '里程碑', exact: true }).click();
  await page.getByRole('button', { name: '新建里程碑', exact: true }).click();
  await page.getByRole('combobox', { name: 'goal_id' }).click();
  await page.locator('.ant-select-item-option-content').getByText('Project replay acceptance', { exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Complete replay');
  const milestone = await save('/api/project/milestones', /保\s*存/);
  await page.getByRole('tab', { name: 'TODO', exact: true }).click();
  async function todo(title, draft, grouped) {
    await page.getByRole('button', { name: '新建 TODO', exact: true }).click();
    await page.getByPlaceholder('要完成的一件事').fill(title);
    await page.getByRole('textbox', { name: 'draft' }).fill(draft);
    if (grouped) {
      await page.getByRole('combobox', { name: 'milestone_id' }).click();
      await page.locator('.ant-select-item-option-content').getByText('Project replay acceptance / Complete replay', { exact: true }).click();
    }
    const created = await save('/api/project/todos', /创\s*建/);
    await page.getByText(`TODO · ${title}`, { exact: true }).waitFor();
    await page.locator('.ant-drawer-close').click();
    await page.getByText(`TODO · ${title}`, { exact: true }).waitFor({ state: 'hidden' });
    return created;
  }
  const main = await todo('Acceptance TODO', 'input 界 '.repeat(10000), true);
  const backlog = await todo('Backlog acceptance', 'standalone TODO', false);
  return { goal, milestone, todo: main, backlog };
}
async function replayOldest(page, root) {
  await page.getByRole('tab', { name: 'TODO', exact: true }).click();
  await page.getByText('Acceptance TODO', { exact: true }).click();
  await page.getByText('加载更早记录', { exact: true }).click();
  const earliest = page.locator('.ant-timeline-item').filter({ has: page.getByText('v1', { exact: true }) });
  await earliest.getByText('查看本次输入与过程', { exact: true }).click();
  await page.getByText('第 1 次 · plan · plan', { exact: true }).waitFor();
  await page.getByText('本次输入与 Agent 版本', { exact: true }).click();
  await page.getByRole('button', { name: '分段查看', exact: true }).click();
  await page.locator('.execution-large-field pre').waitFor();
  assert((await page.locator('.execution-large-field pre').textContent()).includes('"agent"'));
  const next = page.waitForResponse((response) => response.url().includes('detail-field') && response.url().includes('offset=65536'));
  await page.getByRole('button', { name: '下一段', exact: true }).click();
  assert.equal((await next).status(), 200);
  await page.getByText('本次输入与 Agent 版本', { exact: true }).click();
  await page.getByText(/本次过程事件（/).click();
  await page.getByRole('button', { name: '刷新事件', exact: true }).waitFor();
  await page.screenshot({ path: path.join(root, 'project-replay.png'), fullPage: true });
  fs.writeFileSync(path.join(root, 'oldest-browser.json'), JSON.stringify({ oldest_version: 1, chunk_offset: 65536, events_opened: true }, null, 2));
}
module.exports = { openBrowser, createHierarchy, projectPage, replayOldest };
