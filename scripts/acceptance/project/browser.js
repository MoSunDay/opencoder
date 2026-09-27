const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const fs = require('fs');
const path = require('path');
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
    const dialog = page.locator('.ant-drawer').last();
    const response = page.waitForResponse((r) => r.url().endsWith(route) && r.request().method() === 'POST');
    await dialog.getByRole('button', { name: button }).click();
    const saved = await response;
    if (!saved.ok()) throw new Error(`browser create ${route}: ${await saved.text()}`);
    return saved.json();
  }
  await page.getByRole('tab', { name: '项目', exact: true }).click();
  await page.getByRole('button', { name: '新建项目', exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Project replay acceptance');
  const goal = await save('/api/project/goals', /保\s*存/);
  await page.locator('.ant-card-head-title').filter({ hasText: 'Project replay acceptance' }).waitFor();
  await page.getByRole('button', { name: '新建项目', exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Another project');
  await save('/api/project/goals', /保\s*存/);
  await page.getByRole('tab', { name: '里程碑', exact: true }).click();
  await page.getByRole('button', { name: '新建里程碑', exact: true }).click();
  await page.getByRole('combobox', { name: 'goal_id' }).click();
  await page.locator('.ant-select-item-option-content').filter({ hasText: goal.id }).click();
  await page.getByPlaceholder('一句话标题').fill('Complete replay');
  const milestone = await save('/api/project/milestones', /保\s*存/);
  await page.getByRole('tab', { name: '专项', exact: true }).click();
  await page.getByRole('button', { name: '新建专项', exact: true }).click();
  await page.getByRole('combobox', { name: 'goal_id' }).click();
  await page.locator('.ant-select-item-option-content').filter({ hasText: goal.id }).click();
  await page.getByPlaceholder('一句话标题').fill('Project initiative');
  const initiative = await save('/api/project/initiatives', /保\s*存/);
  await page.getByPlaceholder('一句话标题').waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: '新建专项', exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Standalone initiative');
  const standaloneInitiative = await save('/api/project/initiatives', /保\s*存/);
  await page.getByRole('tab', { name: 'TODO', exact: true }).click();
  async function todo(title, draft, groupId) {
    await page.getByRole('button', { name: '新建 TODO', exact: true }).click();
    const drawer = page.locator('.ant-drawer:visible').last();
    await drawer.locator('input#title').fill(title);
    await drawer.locator('textarea#draft').fill(draft);
    if (groupId) {
      await drawer.locator('#milestone_id').click();
      await page.locator('.ant-select-item-option-content').filter({ hasText: groupId }).click();
    }
    const created = await save('/api/project/todos', /创\s*建/);
    await page.getByText(`TODO · ${title}`, { exact: true }).waitFor();
    await page.locator('.ant-drawer-close').click();
    await page.getByText(`TODO · ${title}`, { exact: true }).waitFor({ state: 'hidden' });
    return created;
  }
  const main = await todo('Acceptance TODO', 'input 界 '.repeat(10000), milestone.id);
  const initiativeTodo = await todo('Initiative acceptance', 'specialized work', initiative.id);
  const standaloneTodo = await todo('Standalone initiative TODO', 'independent work', standaloneInitiative.id);
  const backlog = await todo('Backlog acceptance', 'standalone TODO', null);
  return { goal, milestone, initiative, standaloneInitiative, todo: main, initiativeTodo, standaloneTodo, backlog };
}
async function verifyWorkbench(page, root, executionId) {
  await page.getByRole('tab', { name: 'TODO', exact: true }).click();
  await page.getByRole('button', { name: 'Acceptance TODO', exact: true }).click();
  await page.getByText('关联执行', { exact: true }).waitFor();
  await page.getByRole('button', { name: '从能力发起执行' }).waitFor();
  const row = page.locator('tr').filter({ hasText: executionId });
  await row.getByText('Agent', { exact: true }).waitFor();
  await row.getByRole('button', { name: '查看' }).click();
  await page.getByRole('button', { name: '返回 TODO' }).waitFor();
  await page.screenshot({ path: path.join(root, 'project-workbench.png'), fullPage: true });
  fs.writeFileSync(path.join(root, 'workbench-browser.json'), JSON.stringify({ linked_execution_id: executionId, detail_opened: true }, null, 2));
}
module.exports = { openBrowser, createHierarchy, projectPage, verifyWorkbench };
