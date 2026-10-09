const assert = require('node:assert/strict');
const path = require('node:path');
const { projectPage } = require('../browser');
const { KINDS, instruction } = require('./scenario');

async function choose(page, input, label) {
  await input.click();
  if (await input.evaluate((element) => element.tagName === 'INPUT' && !element.readOnly)) await input.fill(label);
  const option = page.locator('.ant-select-dropdown:visible .ant-select-item-option-content')
    .filter({ hasText: label }).first();
  // Native selects virtualize long capability/plan lists. Keyboard navigation
  // brings the requested option into view without reaching into React state.
  const deadline = Date.now() + 15000;
  while (!await option.count() && Date.now() < deadline) {
    await input.press('ArrowDown');
    await page.waitForTimeout(50);
  }
  await option.click();
}
async function submit(page, route, method, button) {
  const [response] = await Promise.all([
    page.waitForResponse((response) => new URL(response.url()).pathname === route && response.request().method() === method),
    button.click(),
  ]);
  assert(response.ok(), method + ' ' + route + ': ' + await response.text());
  return response.json();
}
const drawerFor = (page, todo) => page.getByRole('dialog', { name: 'TODO · ' + todo.title, exact: true });
async function closeTodo(page, todo) {
  const drawer = drawerFor(page, todo);
  await drawer.locator('.ant-drawer-close').click();
  await drawer.waitFor({ state: 'hidden' });
}
async function openTodo(page, todo) {
  await projectPage(page);
  await page.getByRole('tab', { name: 'TODO', exact: true }).click();
  await page.getByLabel('筛选TODO', { exact: true }).click();
  const search = page.getByRole('textbox', { name: '搜索TODO', exact: true });
  await search.fill(todo.title);
  await search.press('Enter');
  await page.getByRole('button', { name: todo.title, exact: true }).click();
  const drawer = drawerFor(page, todo);
  await drawer.getByRole('button', { name: '返回 TODO', exact: true }).waitFor();
  return drawer;
}

async function create(page, scenario) {
  await projectPage(page);
  const goalTitle = scenario.tag + ' 能力对接验收';
  const groupTitle = scenario.tag + ' 四类执行';
  await page.getByRole('button', { name: '新建项目', exact: true }).click();
  await page.getByPlaceholder('一句话标题').fill(goalTitle);
  const goal = await submit(page, '/api/project/goals', 'POST',
    page.getByRole('dialog').last().getByRole('button', { name: /保\s*存/ }));
  await page.getByPlaceholder('一句话标题').waitFor({ state: 'hidden' });
  await page.getByRole('tab', { name: '专项', exact: true }).click();
  await page.getByRole('button', { name: '新建专项', exact: true }).click();
  await choose(page, page.getByRole('combobox', { name: 'goal_id' }), goalTitle);
  await page.getByPlaceholder('一句话标题').fill(groupTitle);
  const initiative = await submit(page, '/api/project/initiatives', 'POST',
    page.getByRole('dialog').last().getByRole('button', { name: /保\s*存/ }));
  assert.equal(initiative.goal_id, goal.id);
  await page.getByPlaceholder('一句话标题').waitFor({ state: 'hidden' });
  await page.getByRole('tab', { name: 'TODO', exact: true }).click();
  const todos = {};
  for (const kind of KINDS) {
    const marker = scenario.tag + '-' + kind;
    await page.getByRole('button', { name: '新建 TODO', exact: true }).click();
    const form = page.getByRole('dialog', { name: '新建 TODO', exact: true });
    await form.locator('input#title').fill(marker);
    await form.locator('textarea#draft').fill(instruction(marker));
    await choose(page, form.getByRole('combobox', { name: '所属专项' }), groupTitle);
    const todo = await submit(page, '/api/project/todos', 'POST',
      form.getByRole('button', { name: /创\s*建/ }));
    assert.equal(todo.initiative_id, initiative.id);
    const drawer = drawerFor(page, todo);
    await choose(page, drawer.getByRole('combobox', { name: '执行能力' }), scenario.capabilities[kind].label);
    await submit(page, '/api/project/todos/' + todo.id, 'PATCH', drawer.getByRole('button', { name: '保存 TODO' }));
    todos[kind] = { ...todo, marker };
    await closeTodo(page, todo);
  }
  return { goal, initiative, todos };
}

async function launch(page, scenario, todo, kind, loseReceipt = false) {
  const drawer = await openTodo(page, todo);
  const createPath = '/api/project/todos/' + todo.id + '/dispatch';
  const createUrl = '**' + createPath;
  const bodies = [];
  const countSubmission = (request) => {
    if (request.method() === 'POST' && new URL(request.url()).pathname === createPath) {
      bodies.push(request.postDataJSON());
    }
  };
  page.on('request', countSubmission);
  let interruptedReceipt;
  if (loseReceipt) await page.route(createUrl, async (route) => {
    if (route.request().method() !== 'POST' || interruptedReceipt) return route.continue();
    const accepted = await route.fetch();
    assert(accepted.ok(), 'receipt interruption requires actual acceptance');
    interruptedReceipt = await accepted.json();
    return route.fulfill({ status: 503, contentType: 'application/json', body: '{"error":"injected lost acceptance receipt"}' });
  });
  await drawer.getByRole('button', { name: '返回 TODO', exact: true }).click();
  await drawer.getByRole('button', { name: '指派所选能力' }).click();
  const prompt = drawer.getByRole('textbox', { name: '执行任务', exact: true });
  assert((await prompt.inputValue()).includes(todo.marker), 'TODO input was not forwarded');
  if (kind === 'brain') await drawer.getByRole('textbox', { name: '能力输入参数' }).fill(JSON.stringify({ todo: todo.draft }));
  const send = async () => {
    const [response] = await Promise.all([
      page.waitForResponse((response) => new URL(response.url()).pathname === createPath && response.request().method() === 'POST'),
      drawer.getByRole('button', { name: /开始执行$/ }).click(),
    ]);
    return response;
  };
  if (loseReceipt) {
    assert.equal((await send()).status(), 503);
    assert(interruptedReceipt, 'the original request must already be accepted');
    await drawer.getByText(/injected lost acceptance receipt/).waitFor();
  }
  const response = await send();
  assert(response.ok(), await response.text());
  const receipt = await response.json();
  if (loseReceipt) {
    assert.deepEqual(receipt, interruptedReceipt, 'retry changed the durable acceptance');
  }
  const id = receipt.execution_id;
  assert(id && receipt.linked, 'dispatch must return the linked execution ID');
  assert.equal(receipt.capability_id, scenario.capabilities[kind].id);
  await drawer.locator('.execution-view-full, .execution-view-conversation').waitFor({ timeout: 30000 });
  if (loseReceipt) await page.unroute(createUrl);
  await drawer.getByRole('button', { name: '返回 TODO' }).click();
  await drawer.getByRole('row').filter({ hasText: id }).waitFor();
  page.off('request', countSubmission);
  assert.equal(bodies.length, loseReceipt ? 2 : 1, 'unexpected execution creation request');
  if (loseReceipt) assert.deepEqual(bodies[0], bodies[1], 'retry must retain the original execution ID and input');
  await closeTodo(page, todo);
  return id;
}

async function inspect(page, todo, id, root, status) {
  const drawer = await openTodo(page, todo);
  await drawer.locator('.execution-view-full, .execution-view-conversation').waitFor();
  await drawer.getByRole('button', { name: '返回 TODO', exact: true }).click();
  const row = drawer.getByRole('row').filter({ hasText: id });
  await drawer.getByRole('columnheader', { name: /状态/ }).waitFor();
  await row.getByText(status, { exact: true }).waitFor();
  const statusColumn = true;
  const statusText = status === 'idle' ? '等待继续' : '已完成';
  let result = null;
  if (id.startsWith('dag-')) {
    await drawer.getByText('打开执行明细查看各步骤结果和产物', { exact: true }).waitFor();
  } else {
    await drawer.locator('.md-body').filter({ hasText: todo.marker }).waitFor();
    result = await drawer.locator('.md-body').filter({ hasText: todo.marker }).innerText();
  }
  const [loaded] = await Promise.all([
    page.waitForResponse((response) => new URL(response.url()).pathname === '/api/executions/' + id),
    row.getByRole('button', { name: '查看', exact: true }).click(),
  ]);
  assert(loaded.ok(), 'native execution detail unavailable');
  await drawer.locator('.execution-view-full, .execution-view-conversation').waitFor();
  await drawer.locator('.ant-descriptions').getByText(statusText, { exact: true }).first().waitFor();
  await page.screenshot({ path: path.join(root, todo.marker + '.png'), animations: 'disabled' });
  await closeTodo(page, todo);
  await page.locator('.fleet-nav-category').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.locator('.ant-table-tbody .ant-table-row').first().waitFor();
  const executionRow = page.getByRole('row').filter({ hasText: id });
  while (!await executionRow.count()) {
    const next = page.locator('.ant-pagination-next:not(.ant-pagination-disabled)');
    assert(await next.count(), 'execution missing from Agent tab: ' + id);
    const previous = await page.locator('.ant-pagination-item-active').getAttribute('title');
    await next.click();
    await page.waitForFunction((prior) => document.querySelector('.ant-pagination-item-active')?.title !== prior, previous);
  }
  await executionRow.getByRole('button', { name: id, exact: true }).click();
  await page.locator('.oc-execution-detail .execution-view-full').waitFor();
  await page.locator('.oc-execution-detail .ant-drawer-close').click();
  await page.locator('.oc-execution-detail').waitFor({ state: 'hidden' });
  return { result, statusColumn };
}

async function markDone(page, todo) {
  const drawer = await openTodo(page, todo);
  await drawer.getByRole('button', { name: '返回 TODO', exact: true }).click();
  await choose(page, drawer.getByRole('combobox', { name: 'TODO 看板列' }), '已完成');
  await submit(page, '/api/project/todos/' + todo.id, 'PATCH', drawer.getByRole('button', { name: '保存 TODO' }));
  await closeTodo(page, todo);
}

async function progress(page, hierarchy, count, root) {
  await projectPage(page);
  for (const [tab, entity] of [['项目', hierarchy.goal], ['专项', hierarchy.initiative]]) {
    await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: tab, exact: true }).click();
    await page.getByLabel('筛选' + tab, { exact: true }).click();
    const search = page.getByRole('textbox', { name: '搜索' + tab, exact: true });
    await search.fill(entity.title); await search.press('Enter');
    const indicator = page.getByRole('row').filter({ hasText: entity.title }).locator('.project-progress');
    await indicator.getByText(count + '/4', { exact: true }).waitFor();
    await indicator.getByText(count * 25 + '%', { exact: true }).waitFor();
    if ([0, 1, 4].includes(count)) await page.screenshot({
      path: path.join(root, entity.id + '-progress-' + count + '.png'), animations: 'disabled',
    });
  }
}
module.exports = { create, launch, inspect, openTodo, closeTodo, markDone, progress, submit };
