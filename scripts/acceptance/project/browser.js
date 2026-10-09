const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const fs = require('fs');
const path = require('path');
async function projectPage(page) {
  await page.locator('.fleet-nav-category').getByText('项目', { exact: true }).click();
  await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: '项目', exact: true }).waitFor();
}
async function openBrowser(h, errors) {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage', '--no-proxy-server'] });
  const page = await browser.newPage({ viewport: { width: 1500, height: 1000 } });
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript((token) => localStorage.setItem('oc_token', token), h.token);
  try {
    if (!(await page.evaluate(() => typeof Array.prototype.at === 'function'))) {
      throw new Error('Chromium lacks Array.prototype.at; use Playwright Chromium or a newer browser');
    }
    await page.goto(h.base, { waitUntil: 'networkidle' });
    await projectPage(page);
  } catch (error) {
    fs.writeFileSync(path.join(h.root, 'browser-failure.json'), JSON.stringify({ url: page.url(), pageErrors: errors, error: error.message }, null, 2));
    try { fs.writeFileSync(path.join(h.root, 'browser-failure.html'), await page.content()); } catch { /* browser may already be closed */ }
    try { await page.screenshot({ path: path.join(h.root, 'browser-failure.png'), fullPage: true }); } catch { /* retain the original failure */ }
    await browser.close(); throw error;
  }
  return { browser, page };
}
async function createHierarchy(page) {
  async function save(route, button) {
    const dialog = page.locator('.ant-drawer:visible').last();
    const response = page.waitForResponse((r) => r.url().endsWith(route) && r.request().method() === 'POST');
    await dialog.getByRole('button', { name: button }).click();
    const saved = await response;
    if (!saved.ok()) throw new Error(`browser create ${route}: ${await saved.text()}`);
    return saved.json();
  }
  await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: '项目', exact: true }).click();
  await page.getByRole('button', { name: '新建项目', exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Project replay acceptance');
  const goal = await save('/api/project/goals', /保\s*存/);
  await page.getByRole('button', { name: 'Project replay acceptance', exact: true }).waitFor();
  await page.getByRole('button', { name: '新建项目', exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Another project');
  await save('/api/project/goals', /保\s*存/);
  await page.getByRole('tab', { name: '专项', exact: true }).click();
  await page.getByRole('button', { name: '新建专项', exact: true }).click();
  await page.getByRole('combobox', { name: 'goal_id' }).click();
  await page.locator('.ant-select-item-option-content').getByText('Project replay acceptance', { exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Project initiative');
  const initiative = await save('/api/project/initiatives', /保\s*存/);
  await page.getByPlaceholder('一句话标题').waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: '新建专项', exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill('Standalone initiative');
  const standaloneInitiative = await save('/api/project/initiatives', /保\s*存/);
  await page.getByRole('tab', { name: 'TODO', exact: true }).click();
  async function todo(title, draft, groupId, capability) {
    await page.getByRole('button', { name: '新建 TODO', exact: true }).click();
    const drawer = page.locator('.ant-drawer:visible').last();
    await drawer.locator('input#title').fill(title);
    await drawer.locator('textarea#draft').fill(draft);
    if (groupId) {
      await drawer.getByRole('combobox', { name: '所属专项' }).click();
      const label = groupId === initiative.id ? 'Project initiative' : 'Standalone initiative';
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option-content').filter({ hasText: label }).click();
    }
    const created = await save('/api/project/todos', /创\s*建/);
    await page.getByText(`TODO · ${title}`, { exact: true }).waitFor();
    if (capability) {
      const detail = page.getByRole('dialog', { name: `TODO · ${title}` });
      await detail.getByRole('combobox', { name: '执行能力' }).click();
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option-content').getByText(capability === 'Agent' ? 'General purpose agent · act' : 'Execute an explicit host operation using the registered Operator · act', { exact: true }).click();
      const updated = page.waitForResponse((r) => r.url().endsWith(`/api/project/todos/${created.id}`) && r.request().method() === 'PATCH');
      await detail.getByRole('button', { name: '保存 TODO' }).click();
      if (!(await updated).ok()) throw new Error('Failed to save TODO capability');
    }
    await page.locator('.ant-drawer-close').click();
    await page.getByText(`TODO · ${title}`, { exact: true }).waitFor({ state: 'hidden' });
    return created;
  }
  const main = await todo('Acceptance TODO', 'input 界 '.repeat(10000), initiative.id, 'Agent');
  const initiativeTodo = await todo('Initiative acceptance', 'specialized work', initiative.id, 'Agent');
  const standaloneTodo = await todo('Standalone initiative TODO', 'independent work', standaloneInitiative.id);
  const backlog = await todo('Backlog acceptance', 'standalone TODO', null, 'Operator');
  return { goal, initiative, standaloneInitiative, todo: main, initiativeTodo, standaloneTodo, backlog };
}
async function verifyWorkbench(page, root, executionId) {
  await page.getByRole('tab', { name: 'TODO', exact: true }).click();
  await page.getByRole('button', { name: 'Acceptance TODO', exact: true }).click();
  await page.getByRole('button', { name: '返回 TODO' }).click();
  await page.getByText('指派记录', { exact: true }).waitFor();
  await page.getByRole('button', { name: '指派所选能力' }).waitFor();
  const row = page.locator('tr').filter({ hasText: executionId });
  await row.getByText('Agent', { exact: true }).waitFor();
  await row.getByRole('button', { name: '查看' }).click();
  await page.getByRole('button', { name: '返回 TODO' }).waitFor();
  await page.screenshot({ path: path.join(root, 'project-workbench.png'), fullPage: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await page.waitForTimeout(350);
  await page.screenshot({ path: path.join(root, 'project-workbench-mobile.png'), fullPage: true });
  const mobile = await page.evaluate(() => {
    const drawer = document.querySelector('.ant-drawer-content-wrapper');
    const title = document.querySelector('.ant-drawer-title');
    const bounds = (element) => { const { left, right, width } = element.getBoundingClientRect(); return { left, right, width }; };
    return { viewport: window.innerWidth, drawer: bounds(drawer), title: bounds(title) };
  });
  if (mobile.drawer.left < -1 || mobile.drawer.right > mobile.viewport + 1 || mobile.title.left < 0 || mobile.title.right > mobile.viewport) {
    throw new Error(`Mobile workbench drawer is clipped: ${JSON.stringify(mobile)}`);
  }
  await page.setViewportSize({ width: 1500, height: 1000 });
  fs.writeFileSync(path.join(root, 'workbench-browser.json'), JSON.stringify({ linked_execution_id: executionId, detail_opened: true }, null, 2));
}
async function launchFromTodo(page, todoTitle, agent) {
  await page.locator('.ant-drawer-close').click();
  await page.getByRole('button', { name: todoTitle, exact: true }).click();
  await page.getByRole('button', { name: '返回 TODO' }).click();
  if (agent) {
    await page.getByRole('combobox', { name: '执行能力' }).click();
    await page.locator('.ant-select-dropdown:visible .ant-select-item-option-content').filter({ hasText: 'acceptance-agent' }).click();
  }
  await page.getByRole('button', { name: '指派所选能力' }).click();
  await page.getByRole('textbox', { name: '执行任务' }).fill('通过 TODO 发起执行验收');
  const submitted = page.waitForResponse((response) => response.url().endsWith('/dispatch') && response.request().method() === 'POST');
  await page.getByRole('button', { name: '开始执行' }).click();
  const response = await submitted;
  if (!response.ok()) throw new Error(`browser capability launch: ${await response.text()}`);
  const execution = await response.json();
  await page.getByRole('button', { name: '返回 TODO' }).click();
  await page.locator('tr').filter({ hasText: execution.execution_id }).waitFor();
  return execution.execution_id;
}
async function verifyNativeAgentLaunch(page, todoTitle) {
  await page.getByRole('button', { name: '返回 TODO' }).click();
  return launchFromTodo(page, todoTitle, true);
}
async function verifyNativeOperatorLaunch(page, todoTitle) {
  return launchFromTodo(page, todoTitle, false);
}
module.exports = { openBrowser, createHierarchy, projectPage, verifyWorkbench, verifyNativeAgentLaunch, verifyNativeOperatorLaunch };
