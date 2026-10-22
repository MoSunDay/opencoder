import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';

export async function checkEntities(browser, base, output) {
  const result = { measurements: [], unassigned: false, creation: false, retry: false };
  const setup = await browser.newPage();
  const request = async (path, data) => {
    const response = await setup.request.post(`${base}/api/ontology/envs/debug${path}`, { data });
    assert(response.ok(), `${path}: ${await response.text()}`);
    return response.json();
  };
  const type = (await request('/entity-types', { key: 'browser_list', name: '浏览器列表类型' })).item;
  await request('/entities', { request_id: randomUUID(), entity_type_id: type.id,
    name: '无目录测试实体', source: '# Source', ext: '# Notes', attributes: {} });
  await setup.close();
  for (const width of [390, 768, 1280, 1920]) {
    // A new context starts at the actual viewport with no desktop navigation state.
    const page = await browser.newPage({ viewport: { width, height: 1000 } });
    const errors = [];
    page.on('pageerror', error => errors.push(error.message));
    const measure = async (state) => {
      const sizes = await page.evaluate(() => ({ viewport: innerWidth, page: document.documentElement.scrollWidth }));
      assert(sizes.page <= width + 1, `entities overflow: ${JSON.stringify(sizes)}`);
      result.measurements.push({ width, state, ...sizes });
      await page.screenshot({ path: `${output}/entities-${width}-${state}.png`, animations: 'disabled' });
    };
    const choose = async (input, text) => {
      await input.click();
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option').getByText(text, { exact: true }).click();
    };
    try {
      await page.goto(base);
      if (width < 768) {
        await page.locator('.fleet-content > .fleet-mobile-nav[role=tablist]').getByRole('tab', { name: 'Ontology', exact: true }).click();
        await choose(page.locator('.ant-select.fleet-mobile-nav'), '实体');
      } else {
        await page.locator('.fleet-sidebar [role=tablist]').getByRole('tab', { name: 'Ontology', exact: true }).click();
        await page.getByRole('menuitem', { name: /实体$/ }).click();
      }
      const all = page.getByRole('button', { name: '全部实体', exact: true });
      const entity = page.getByRole('button', { name: '无目录测试实体', exact: true });
      const search = page.getByRole('textbox', { name: '搜索名称或标识' });
      await all.waitFor();
      assert.equal(await entity.count(), 0);
      await all.click();
      await search.fill('无目录测试实体');
      await entity.waitFor();
      result.unassigned = true;
      await measure('all');
      if (width < 768) {
        const tree = await page.request.get(`${base}/api/ontology/envs/debug/directories/tree`);
        const items = (await tree.json()).items;
        const platform = items.find(item => item.name === '平台能力');
        const parts = [platform.name];
        let parent = items.find(item => item.id === platform.parent_id);
        while (parent) { parts.unshift(parent.name); parent = items.find(item => item.id === parent.parent_id); }
        await choose(page.getByRole('combobox', { name: '实体目录', exact: true }), parts.join(' / '));
      } else {
        await page.locator('.ant-tree').getByText('平台能力', { exact: true }).click();
      }
      await entity.waitFor({ state: 'hidden' });
      await measure('directory-empty');
      await all.click();
      await entity.waitFor();
      await search.fill('missing-entity');
      await entity.waitFor({ state: 'hidden' });
      await search.fill('无目录测试实体');
      await entity.waitFor();
      if (width === 390) {
        await search.fill('');
        await page.getByRole('button', { name: /新增实体$/ }).click();
        const dialog = page.locator('.ant-modal[role=dialog]:visible');
        await choose(dialog.getByLabel('实体类型', { exact: true }), '浏览器列表类型');
        await dialog.getByLabel('名称', { exact: true }).fill('手机创建的实体');
        await dialog.getByLabel('来源', { exact: true }).fill('# Created in narrow viewport');
        await dialog.getByLabel('扩展信息（Markdown）', { exact: true }).fill('# Notes');
        await measure('create');
        await dialog.getByRole('button', { name: /确\s*定/ }).click();
        await dialog.waitFor({ state: 'hidden' });
        await search.fill('手机创建的实体');
        await page.getByRole('button', { name: '手机创建的实体', exact: true }).waitFor();
        result.creation = true;
        await measure('created');

        // Delay one real list request, then fail it; retry goes to the Server.
        let release, entered;
        const hold = new Promise(resolve => { release = resolve; });
        const loading = new Promise(resolve => { entered = resolve; });
        const pattern = '**/api/ontology/envs/debug/entities?**';
        await page.route(pattern, async route => {
          entered(); await hold;
          await route.fulfill({ status: 503, contentType: 'application/json', body: JSON.stringify({ error: 'temporary list failure' }) });
        });
        await page.reload();
        await loading;
        await page.locator('.ant-spin-spinning').first().waitFor();
        await measure('loading');
        release();
        await page.getByText('实体加载失败', { exact: true }).waitFor();
        await measure('error');
        await page.unroute(pattern);
        await page.getByRole('button', { name: /重\s*试/ }).click();
        await all.click();
        await search.fill('手机创建的实体');
        await page.getByRole('button', { name: '手机创建的实体', exact: true }).waitFor();
        result.retry = true;
        await measure('retry');
      }
      assert.deepEqual(errors, []);
    } catch (error) {
      await page.screenshot({ path: `${output}/entities-${width}-failure.png` });
      throw error;
    } finally {
      await page.close();
    }
  }
  return result;
}
