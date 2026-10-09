import assert from 'node:assert/strict';

export async function checkDetails(page) {
  await page.getByRole('button', { name: '浏览器实体', exact: true }).click();
  const drawer = page.locator('.ant-drawer-open');
  await drawer.getByRole('tab', { name: '普通属性', exact: true }).click();
  await drawer.getByRole('textbox', { name: '编辑 负责人' }).fill('保留的草稿');
  await drawer.getByRole('tab', { name: '来源', exact: true }).click();
  await drawer.getByRole('heading', { name: '浏览器来源', exact: true }).waitFor();
  await drawer.getByRole('tab', { name: '拓展信息', exact: true }).click();
  await drawer.getByRole('heading', { name: '浏览器扩展', exact: true }).waitFor();
  await drawer.getByRole('tab', { name: /^普通属性/ }).click();
  assert.equal(await drawer.getByRole('textbox', { name: '编辑 负责人' }).inputValue(), '保留的草稿');
  await drawer.locator('.ant-drawer-close').click();
  await page.getByRole('button', { name: '继续编辑', exact: true }).click();
  await page.locator('.ant-modal-confirm:visible').waitFor({ state: 'hidden' });
  await drawer.getByRole('button', { name: '提交', exact: true }).click();
  await page.getByText('属性已保存', { exact: true }).waitFor();
  await drawer.getByRole('tab', { name: '来源', exact: true }).click();
  await drawer.getByRole('button', { name: /编辑内容/ }).click();
  const source = drawer.getByRole('textbox', { name: /正文$/ });
  await source.fill('# 保留正文草稿');
  await drawer.getByRole('tab', { name: '基本信息', exact: true }).click();
  await drawer.getByRole('tab', { name: /^来源/ }).click();
  assert.equal(await source.inputValue(), '# 保留正文草稿');
  // Navigation remains usable behind the drawer through a normal category action.
  await drawer.locator('.ant-drawer-close').click();
  await page.getByRole('button', { name: '放弃并继续', exact: true }).click();
  await drawer.waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: '浏览器实体', exact: true }).click();
  await drawer.getByRole('tab', { name: '普通属性', exact: true }).click();
  assert.equal(await drawer.getByRole('textbox', { name: '编辑 负责人' }).inputValue(), '保留的草稿');
  await drawer.getByRole('tab', { name: '来源', exact: true }).click();
  await drawer.getByRole('heading', { name: '浏览器来源', exact: true }).waitFor();
  await drawer.locator('.ant-drawer-close').click();
  await drawer.waitFor({ state: 'hidden' });
}

export async function checkGraph(page) {
  await page.locator('.graph-node-title').filter({ hasText: '入口服务' }).waitFor();
  await page.getByRole('button', { name: '恢复 100%', exact: true }).click();
  await page.getByRole('button', { name: '放大', exact: true }).click();
  await page.getByRole('button', { name: '恢复 100%', exact: true }).filter({ hasText: '120%' }).waitFor();
  await page.getByRole('button', { name: '放大画布', exact: true }).click();
  await page.getByRole('button', { name: '恢复画布', exact: true }).click();
  await page.locator('.oc-page-extra').getByText('环境', { exact: true }).click();
  await page.waitForFunction(() => [...document.querySelectorAll('[role="tooltip"]')]
    .every((tooltip) => !tooltip.getClientRects().length || getComputedStyle(tooltip).visibility !== 'visible'));
  assert.equal(await page.getByRole('button', { name: '恢复 100%', exact: true }).innerText(), '120%');
  for (const width of [768, 390, 1200]) {
    await page.setViewportSize({ width, height: 850 });
    await page.waitForFunction(() => {
      const viewport = document.querySelector('.graph-viewport')?.getBoundingClientRect();
      const node = [...document.querySelectorAll('.graph-node-title')].find((element) => element.textContent === '入口服务')?.getBoundingClientRect();
      if (!viewport || !node) return false;
      const x = node.x + node.width / 2, y = node.y + node.height / 2;
      return x >= viewport.x && x <= viewport.right && y >= viewport.y && y <= viewport.bottom;
    });
  }
  assert.equal(await page.getByRole('button', { name: '恢复 100%', exact: true }).innerText(), '120%');
  await page.getByText('列表', { exact: true }).click();
  const results = page.locator('.graph-result-list');
  await results.getByLabel('搜索完整观测结果', { exact: true }).fill('下游');
  await results.getByRole('button', { name: '下游服务', exact: true }).click();
  const drawer = page.locator('.ant-drawer-open');
  await drawer.getByRole('tab', { name: '基本信息', exact: true }).waitFor();
  await drawer.getByRole('button', { name: '入口服务', exact: true }).first().click();
  await drawer.getByRole('button', { name: '返回上一项', exact: true }).click();
  await drawer.locator('.ant-drawer-close').click();
  await drawer.waitFor({ state: 'hidden' });
  await results.getByRole('button', { name: '在图中定位', exact: true }).click();
  await page.locator('canvas:visible').first().waitFor();
  const saved = await page.evaluate(() => JSON.parse(localStorage.getItem('oc_ontology_observation:debug')));
  assert.equal(Object.hasOwn(saved, 'expandNeighbors'), false);
  assert.equal(saved.selection.centerIds.length, 1);
  await page.reload();
  await page.locator('canvas:visible').first().waitFor();
  assert.equal(await page.getByRole('checkbox', { name: '展开跨类型邻居' }).count(), 0);
  await page.getByText('已展示 2/2 个实体 · 2/2 条关系', { exact: true }).waitFor();
}
