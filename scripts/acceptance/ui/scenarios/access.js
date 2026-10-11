const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

async function access({ page, browser, base, api, root }) {
  await page.locator('.fleet-nav-category').getByRole('tab', { name: '后台管理' }).click();
  await page.getByRole('menuitem', { name: '用户权限' }).click();
  await page.getByRole('button', { name: '创建用户' }).click();
  await page.getByLabel('用户名', { exact: true }).fill('accept-viewer');
  await page.getByRole('dialog', { name: '创建用户' }).getByRole('button', { name: '创建用户' }).click();
  await page.getByRole('cell', { name: 'accept-viewer', exact: true }).waitFor();
  assert.equal(await page.getByRole('menuitem', { name: 'Token 管理' }).count(), 0);
  const modal = page.getByRole('dialog', { name: 'Token 仅此一次显示' });
  await modal.waitFor();
  const token = (await modal.innerText()).match(/oc_[0-9A-Z]+/)[0];
  await page.context().grantPermissions(['clipboard-read', 'clipboard-write']);
  await modal.locator('.ant-typography-copy').click();
  assert.equal(await page.evaluate(() => navigator.clipboard.readText()), token);
  const viewport = page.viewportSize();
  for (const width of [1920, 1280, 768, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    const bounds = await modal.boundingBox();
    assert(bounds.x >= 0 && bounds.x + bounds.width <= width, `token dialog overflows at ${width}px`);
    await page.screenshot({ path: path.join(root, `access-created-${width}.png`), mask: [modal.locator('.ant-typography')] });
  }
  await page.setViewportSize(viewport);
  await modal.getByRole('button', { name: '我已保存' }).click();
  await modal.waitFor({ state: 'detached' });
  await page.reload();
  await page.getByRole('cell', { name: 'accept-viewer', exact: true }).waitFor();
  assert.equal(await page.getByText(token, { exact: true }).count(), 0);
  await page.screenshot({ path: path.join(root, 'access-users.png') });
  const request = (method, route, body) => fetch(base + route, { method,
    headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: body && JSON.stringify(body) });
  assert.equal((await request('GET', '/api/project/overview')).status, 200);
  assert.equal((await request('GET', '/api/users')).status, 403);
  assert.equal((await request('POST', '/api/project/goals', { title: 'denied' })).status, 403);
  const context = await browser.newContext({ viewport: { width: 1600, height: 1000 } });
  const reader = await context.newPage();
  const errors = [];
  reader.on('pageerror', (error) => errors.push(error.message));
  try {
    await reader.goto(`${base}/#token=${token}`);
    await reader.getByText('accept-viewer · 只读', { exact: true }).waitFor();
    await reader.locator('.fleet-nav-category').getByRole('tab', { name: '项目', exact: true }).click();
    await reader.locator('.fleet-content .ant-tabs').getByRole('tab', { name: '项目', exact: true }).click();
    await reader.getByRole('menuitem', { name: /Overview$/ }).waitFor();
    assert.equal(await reader.getByRole('tab', { name: '后台管理' }).count(), 0);
    assert(await reader.getByRole('button', { name: '新建项目' }).isDisabled());
    await reader.screenshot({ path: path.join(root, 'access-viewer.png') });
    await page.getByRole('menuitem', { name: '用户权限' }).click();
    await page.getByRole('row').filter({ has: page.getByRole('cell', { name: 'accept-viewer', exact: true }) })
      .getByRole('button', { name: '编辑权限' }).click();
    const editor = page.getByRole('dialog', { name: '编辑权限 · accept-viewer' });
    await editor.getByRole('combobox', { name: '用户角色' }).click();
    await page.locator('.ant-select-item-option-content').getByText('editor · 可编辑', { exact: true }).click();
    await editor.getByRole('button', { name: '保存权限' }).click();
    await editor.waitFor({ state: 'detached' });
    assert.equal(await page.getByRole('dialog', { name: 'Token 仅此一次显示' }).count(), 0);
    const allowed = await request('POST', '/api/project/goals', { title: 'editor project' });
    assert.equal(allowed.status, 200);
    const goal = await allowed.json();
    await reader.reload();
    await reader.getByRole('button', { name: '新建项目' }).waitFor();
    assert(!(await reader.getByRole('button', { name: '新建项目' }).isDisabled()));
    assert.equal((await request('POST', '/api/tokens', {})).status, 403);
    await api('DELETE', `/api/project/goals/${goal.id}`);
    const row = page.getByRole('row').filter({ has: page.getByRole('cell', { name: 'accept-viewer', exact: true }) });
    await row.getByRole('button', { name: '删除用户' }).click();
    await page.getByRole('button', { name: '确认删除' }).click();
    await row.waitFor({ state: 'detached' });
    assert.equal((await request('GET', '/api/me')).status, 401);
    assert.deepEqual(errors, []);
  } catch (error) {
    fs.writeFileSync(path.join(root, 'access-reader-failure.json'), JSON.stringify({ errors, body: await reader.locator('body').innerText() }, null, 2));
    await reader.screenshot({ path: path.join(root, 'access-reader-failure.png') });
    throw error;
  } finally { await context.close(); }
}
module.exports = { access };
