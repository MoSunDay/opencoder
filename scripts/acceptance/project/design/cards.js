const assert = require('node:assert/strict');

async function seedCards(api, group) {
  const rows = [];
  for (const [status, title, draft] of [
    ['backlog', '补充范围和验收条件', ''],
    ['todo', '实现任务卡片的阅读层次，完整保留特别长的任务标题与操作入口', '## 交付要求\n\n- **实现界面**\n- [验收记录](https://example.com)\n\n完成后检查四种屏宽。'],
    ['in_progress', '联调接口并核对异常提示', '**接口联调**：保存失败时保留任务草稿，确认后再重试。'],
    ['done', '完成发布与回归记录', '页面和接口验收完成，记录实际结果。'],
  ]) rows.push(await api('POST', '/api/project/todos', { title, draft, board_status: status, initiative_id: group.id }));
  return rows;
}

async function verifyCards({ page, api, initiative, rows }) {
  await page.setViewportSize({ width: 1920, height: 1000 });
  const todo = rows[1];
  const title = () => initiative.getByRole('button', { name: todo.title, exact: true });
  const card = () => initiative.locator('.project-task-card').filter({ has: page.getByRole('button', { name: todo.title, exact: true }) });
  const lane = (index) => initiative.locator('.project-board-lane').nth(index);
  const drawer = page.getByRole('dialog', { name: `TODO · ${todo.title}`, exact: true });
  assert.equal(await initiative.getByRole('button', { name: /^拖动 / }).count(), 0, 'no separate drag buttons');
  async function drag(source, index, expected) {
    await source.scrollIntoViewIfNeeded();
    await source.click({ trial: true });
    const from = await source.boundingBox(), target = await lane(index).boundingBox();
    const saved = page.waitForResponse((r) => r.url().endsWith('/api/project/todos/order') && r.request().method() === 'PUT');
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2);
    await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2 + 12, from.y + from.height / 2, { steps: 3 });
    await page.mouse.move(target.x + target.width / 2, target.y + 70, { steps: 16 });
    await page.mouse.up();
    assert.equal((await saved).status(), expected);
    await initiative.getByText('正在保存状态和顺序…', { exact: true }).waitFor({ state: 'hidden' });
    assert.equal(await drawer.isVisible(), false, 'drag must not open task details');
  }
  await drag(title(), 2, 200);
  assert.equal((await api('GET', `/api/project/todos/${todo.id}`)).board_status, 'in_progress');
  await drag(card().locator('.project-card-summary'), 3, 200);
  assert.equal((await api('GET', `/api/project/todos/${todo.id}`)).board_status, 'done');
  const orderRoute = '**/api/project/todos/order';
  await page.route(orderRoute, (route) => route.fulfill({ status: 500, json: { error: '拖动保存失败验收' } }));
  await drag(card().locator('.project-card-summary'), 1, 500);
  await page.getByText(/状态保存失败，已恢复原状态：.*拖动保存失败验收/).waitFor();
  assert.equal((await api('GET', `/api/project/todos/${todo.id}`)).board_status, 'done');
  await lane(3).getByRole('button', { name: todo.title, exact: true }).waitFor();
  await page.unroute(orderRoute);
  const orderRequests = [];
  const observe = (request) => { if (request.url().endsWith('/api/project/todos/order')) orderRequests.push(request); };
  page.on('request', observe);
  await card().getByRole('button', { name: `更多操作 ${todo.title}`, exact: true }).click();
  await page.getByRole('menuitem', { name: /删除任务/ }).waitFor();
  await page.keyboard.press('Escape');
  assert(await initiative.isVisible(), 'menu Escape preserves the initiative');
  assert.equal(orderRequests.length, 0, 'menu must not move the card');
  await title().focus();
  await title().press('Space');
  await page.waitForFunction(() => [...document.querySelectorAll('.project-todo-card')].some((node) => node.style.opacity === '0.35'));
  await page.keyboard.press('ArrowLeft');
  await page.keyboard.press('Escape');
  assert(await initiative.isVisible(), 'Escape cancels the drag without closing the initiative');
  assert.equal(orderRequests.length, 0, 'canceled keyboard drag must not persist');
  assert.equal((await api('GET', `/api/project/todos/${todo.id}`)).board_status, 'done');
  page.off('request', observe);
  await title().focus();
  await title().press('Enter');
  await drawer.getByLabel('任务说明预览', { exact: true }).waitFor();
  assert.equal(await drawer.getByLabel('TODO 标题', { exact: true }).inputValue(), todo.title);
  await drawer.locator('.ant-drawer-close').click(); await drawer.waitFor({ state: 'hidden' });
  await title().click();
  await drawer.getByLabel('任务说明预览', { exact: true }).waitFor();
  await drawer.locator('.ant-drawer-close').click(); await drawer.waitFor({ state: 'hidden' });
  return { title_drag: true, content_drag: true, no_accidental_open: true, save_failure_rollback: true,
    menu_without_drag: true, keyboard_cancel_preserves_drawer: true, title_click_and_enter_open: true };
}

async function verifyTouch({ page, initiative, rows }) {
  await page.setViewportSize({ width: 390, height: 1000 });
  const card = initiative.locator('.project-task-card').filter({ has: page.getByRole('button', { name: rows[1].title, exact: true }) });
  const summary = card.locator('.project-card-summary');
  await summary.scrollIntoViewIfNeeded();
  const columns = initiative.locator('.project-board-columns');
  const before = await columns.evaluate((node) => node.scrollLeft);
  const box = await summary.boundingBox();
  const x = Math.min(340, box.x + box.width / 2), y = box.y + box.height / 2;
  const session = await page.context().newCDPSession(page);
  await session.send('Emulation.setTouchEmulationEnabled', { enabled: true });
  const touch = (type, px, py) => session.send('Input.dispatchTouchEvent', { type, touchPoints: type === 'touchEnd' ? [] : [{ x: px, y: py }] });
  await touch('touchStart', x, y);
  await touch('touchMove', x - 75, y);
  await touch('touchEnd');
  await page.waitForFunction((initial) => document.querySelector('.project-board-columns').scrollLeft > initial + 10, before);
  assert.equal(await initiative.locator('.project-todo-card[style*="opacity: 0.35"]').count(), 0, 'ordinary swipe scrolls without dragging');
  await summary.scrollIntoViewIfNeeded();
  const held = await summary.boundingBox();
  await touch('touchStart', held.x + held.width / 2, held.y + held.height / 2);
  await page.waitForFunction(() => [...document.querySelectorAll('.project-todo-card')].some((node) => node.style.opacity === '0.35'));
  await touch('touchMove', held.x + held.width / 2 + 12, held.y + held.height / 2);
  await touch('touchEnd');
  await page.waitForFunction(() => ![...document.querySelectorAll('.project-todo-card')].some((node) => node.style.opacity === '0.35'));
  await session.send('Emulation.setTouchEmulationEnabled', { enabled: false });
  await session.detach();
  return { touch_swipe_scroll: true, touch_hold_drag: true };
}

async function verifyCardSurfaces({ page, api, group, output }) {
  const fs = require('node:fs');
  const path = require('node:path');
  const rows = await seedCards(api, group);
  await page.reload({ waitUntil: 'networkidle' });
  await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: '专项', exact: true }).click();
  await page.getByRole('button', { name: group.title, exact: true }).click();
  const initiative = page.getByRole('dialog', { name: `专项 · ${group.title}`, exact: true });
  const measurements = [];
  for (const width of [1920, 1280, 768, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await initiative.locator('.project-task-card').first().waitFor();
    const layout = await initiative.evaluate((element) => ({
      outerFits: element.scrollWidth <= element.clientWidth + 1,
      cardsFit: [...element.querySelectorAll('.project-task-card')].every((card) => card.scrollWidth <= card.clientWidth + 1),
      titleFits: [...element.querySelectorAll('.project-task-card')].every((card) => card.querySelector('.project-card-title').getBoundingClientRect().right <= card.querySelector('.project-card-controls').getBoundingClientRect().left + 1),
      noDragButton: !element.querySelector('.project-card-drag'),
      titleDraggable: [...element.querySelectorAll('.project-card-title')].every((title) => title.getAttribute('aria-roledescription') === 'sortable'),
      surfaceCursor: [...element.querySelectorAll('.project-task-card')].every((card) => getComputedStyle(card).cursor === 'grab'),
    }));
    assert(Object.values(layout).every(Boolean), JSON.stringify({ width, ...layout }));
    measurements.push({ width, ...layout });
    await page.screenshot({ path: path.join(output, `cards-${width}.png`), animations: 'disabled' });
  }
  return { measurements, ...await verifyTouch({ page, initiative, rows }), ...await verifyCards({ page, api, initiative, rows }) };
}

module.exports = { seedCards, verifyCards, verifyCardSurfaces };
