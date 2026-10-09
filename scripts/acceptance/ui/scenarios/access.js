const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

async function access({ page, browser, base, api, root }) {
  await page.locator('.fleet-nav-category').getByRole('tab', { name: '后台管理' }).click();
  await page.getByRole('menuitem', { name: '用户权限' }).click();
  await page.getByLabel('用户名', { exact: true }).fill('accept-viewer');
  await page.getByRole('button', { name: '创建用户' }).click();
  await page.getByRole('cell', { name: 'accept-viewer', exact: true }).waitFor();
  await page.getByRole('menuitem', { name: 'Token 管理' }).click();
  await page.getByRole('combobox', { name: 'Token 所属用户' }).click();
  await page.getByText('accept-viewer · viewer', { exact: true }).click();
  await page.getByLabel('Token 名称', { exact: true }).fill('acceptance');
  await page.getByRole('button', { name: '创建 Token', exact: true }).click();
  const modal = page.getByRole('dialog', { name: 'Token 仅此一次显示' });
  await modal.waitFor();
  const token = (await modal.innerText()).match(/oc_[0-9A-Z]+/)[0];
  await modal.getByRole('button', { name: '我已保存' }).click();
  await modal.waitFor({ state: 'detached' });
  await page.screenshot({ path: path.join(root, 'access-tokens.png') });
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
    assert.equal(await reader.getByRole('tab', { name: '后台管理' }).count(), 0);
    assert(await reader.getByRole('button', { name: '新建项目' }).isDisabled());
    await reader.screenshot({ path: path.join(root, 'access-viewer.png') });
    await api('PATCH', '/api/users/accept-viewer', { role: 'editor' });
    const allowed = await request('POST', '/api/project/goals', { title: 'editor project' });
    assert.equal(allowed.status, 200);
    const goal = await allowed.json();
    await reader.reload();
    await reader.getByRole('button', { name: '新建项目' }).waitFor();
    assert(!(await reader.getByRole('button', { name: '新建项目' }).isDisabled()));
    assert.equal((await request('POST', '/api/tokens', {})).status, 403);
    await api('DELETE', `/api/project/goals/${goal.id}`);
    const row = page.getByRole('row').filter({ has: page.getByRole('cell', { name: 'acceptance', exact: true }) });
    await row.getByRole('button', { name: /撤\s*销/ }).click();
    await page.getByRole('button', { name: /确\s*定/ }).click();
    await row.getByText('已撤销', { exact: true }).waitFor();
    assert.equal((await request('GET', '/api/me')).status, 401);
    assert.deepEqual(errors, []);
  } catch (error) {
    fs.writeFileSync(path.join(root, 'access-reader-failure.json'), JSON.stringify({ errors, body: await reader.locator('body').innerText() }, null, 2));
    await reader.screenshot({ path: path.join(root, 'access-reader-failure.png') });
    throw error;
  } finally { await context.close(); }
  await page.getByRole('menuitem', { name: '用户权限' }).click();
  await page.getByRole('row').filter({ has: page.getByRole('cell', { name: 'accept-viewer', exact: true }) }).getByRole('button', { name: '删除用户' }).click();
  await page.getByRole('button', { name: /确\s*定/ }).click();
  await page.getByRole('cell', { name: 'accept-viewer', exact: true }).waitFor({ state: 'detached' });
}
module.exports = { access };
