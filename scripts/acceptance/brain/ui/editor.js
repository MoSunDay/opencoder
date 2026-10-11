// Isolated regression against the real SPA. Only HTTP data uses fixtures.
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const http = require('node:http');
const { chromium } = require('../../../../crates/web/spa/node_modules/playwright-core');
const { FIXTURES } = require('../../spa_responsive_fixtures');
const spa = path.resolve(__dirname, '../../../../crates/web/spa/dist');
const layer = (id) => ({ layer_id: id, title: id, task: '完成任务', objective: '交付结果', success_criteria: '验证通过' });
const node = (id, layer_id) => ({ node_id: id, layer_id, title: id, objective: '执行任务', capability_id: 'coding' });
const capability = { id: 'coding', kind: 'agent', target: 'Coder', summary: '编码能力', version: '1', input_desc: '任务', output_desc: '变更', definition: {} };
const original = { id: 'canvas-edit', version: 1, created_at: 1000, changelog: 'fixture', tags: [], plan: {
  schema_version: 7, title: '画布回退验收', objective: '交付', inputs: {}, max_rounds: 5,
  layers: [layer('coding'), layer('testing')], nodes: [node('code-a', 'coding'), node('code-b', 'coding'), node('test-a', 'testing')],
} };

async function main() {
  const output = process.argv[2] || await fs.mkdtemp('/tmp/opencoder-brain-editor-');
  await fs.mkdir(output, { recursive: true });
  let versions = [structuredClone(original)];
  const errors = [], writes = [], checks = [];
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url, 'http://localhost');
    const send = (status, value) => { response.writeHead(status, { 'content-type': 'application/json' }); response.end(JSON.stringify(value)); };
    try {
      if (url.pathname.startsWith('/api/')) {
        if (request.method === 'POST') {
          let raw = ''; for await (const chunk of request) raw += chunk;
          const body = JSON.parse(raw);
          writes.push({ path: url.pathname, body });
          if (url.pathname === '/api/brain/plan-defs/validate') return send(200, { valid: true });
          if (url.pathname === '/api/brain/plan-defs') {
            assert.equal(body.version, versions.length + 1);
            versions.push(body); return send(200, { definition: { id: body.id, latest_version: body.version }, version: body });
          }
          throw new Error(`Unexpected write: ${url.pathname}`);
        }
        if (url.pathname === '/api/brain/plan-defs') return send(200, { plans: [{ id: original.id, title: original.plan.title, latest_version: versions.length, schema_version: 7 }] });
        if (url.pathname === '/api/brain/library') return send(200, { capabilities: [capability] });
        if (url.pathname === '/api/brain/runs') return send(200, { runs: [] });
        if (url.pathname.endsWith('/versions')) return send(200, { versions: versions.toReversed() });
        if (url.pathname.includes('/versions/')) return send(200, versions[Number(url.pathname.split('/').at(-1)) - 1]);
        if (FIXTURES[url.pathname]) return send(200, FIXTURES[url.pathname]);
        throw new Error(`Unexpected read: ${url.pathname}`);
      }
      const file = url.pathname === '/' ? 'index.html' : url.pathname.slice(1);
      const data = await fs.readFile(path.join(spa, file));
      response.writeHead(200, { 'content-type': file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html' });
      response.end(data);
    } catch (error) { errors.push(error.message); send(500, { error: error.message }); }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--no-proxy-server', '--disable-dev-shm-usage'] });
  let page;
  try {
    for (const key of ['Backspace', 'Delete']) {
      versions = [structuredClone(original)];
      page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
      page.on('pageerror', (error) => errors.push(error.message));
      await page.addInitScript(() => {
        localStorage.setItem('oc_token', 'fixture-token');
        localStorage.setItem('oc_nav_page', JSON.stringify('brain'));
      });
      await page.goto(`http://127.0.0.1:${server.address().port}`, { waitUntil: 'networkidle' });
      await page.getByRole('tab', { name: '计划库', exact: true }).click();
      const openEditor = () => page.getByRole('button', { name: original.plan.title, exact: true }).click();
      await openEditor();
      const canvas = page.getByLabel('里程碑编辑画布', { exact: true });
      const card = (id) => canvas.locator(`[data-id="${id}"]`);
      await card('execution:code-a').waitFor();
      const readDraft = () => page.evaluate(() => {
        const key = Object.keys(localStorage).find((key) => key.startsWith('oc:brain:scheduler-draft:v7:'));
        return key ? JSON.parse(localStorage.getItem(key)) : null;
      });
      const confirm = async () => {
        const dialog = page.getByRole('dialog', { name: '删除选中的里程碑或执行节点？', exact: true });
        await dialog.getByRole('button', { name: /删\s*除/ }).click();
        await dialog.waitFor({ state: 'hidden' });
      };
      await page.getByRole('button', { name: '查看全图', exact: true }).click();
      await card('layer:coding').locator('header').click();
      const bounds = await card('layer:coding').locator('header').boundingBox();
      await page.mouse.move(bounds.x + bounds.width / 2, bounds.y + 10); await page.mouse.down();
      await page.mouse.move(bounds.x + bounds.width / 2 + 80, bounds.y + 60, { steps: 12 }); await page.mouse.up();
      await page.waitForFunction(() => Object.values(localStorage).some((v) => { try { return JSON.parse(v)?.layout?.coding; } catch { return false; } }));
      const dragged = (await readDraft()).layout.coding;
      await card('layer:coding').press('ArrowRight');
      const initialLayout = (await readDraft()).layout;
      assert.deepEqual(initialLayout.coding, { x: dragged.x + 5, y: dragged.y });
      await card('execution:code-a').click(); await page.keyboard.press(key);
      await page.getByRole('dialog', { name: '删除选中的里程碑或执行节点？' }).getByRole('button', { name: /取\s*消/ }).click();
      await card('execution:code-a').click(); await page.keyboard.press(key); await confirm();
      await card('execution:code-a').waitFor({ state: 'detached' });
      await card('execution:code-b').click();
      assert.deepEqual((await readDraft()).version.plan.nodes.map((n) => n.node_id), ['code-b', 'test-a']);
      assert.deepEqual((await readDraft()).layout, initialLayout);
      await page.getByRole('combobox', { name: '所属里程碑' }).click();
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option-content').getByText('testing', { exact: true }).click();
      assert.deepEqual((await readDraft()).layout, initialLayout);
      await page.getByRole('combobox', { name: '所属里程碑' }).click();
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option-content').getByText('coding', { exact: true }).click();
      await card('layer:coding').locator('header').click();
      await page.getByLabel('里程碑要做什么', { exact: true }).fill('编辑后仍保留删除结果');
      await page.getByRole('button', { name: '关闭画布', exact: true }).click();
      await canvas.waitFor({ state: 'hidden' }); await openEditor();
      await card('execution:code-b').waitFor();
      assert.equal(await card('execution:code-a').count(), 0);
      assert.deepEqual((await readDraft()).layout, initialLayout);
      if (key === 'Backspace') for (const width of [1920, 1280, 768, 390]) {
        await page.setViewportSize({ width, height: 1000 });
        await page.getByRole('button', { name: '查看全图', exact: true }).click();
        await page.screenshot({ path: path.join(output, `editor-${width}.png`), fullPage: true, animations: 'disabled' });
        assert.equal(await card('execution:code-a').count(), 0);
        assert.deepEqual((await readDraft()).layout, initialLayout);
      }
      await page.setViewportSize({ width: 1440, height: 1000 });
      await page.getByRole('button', { name: '下一步：计划信息', exact: true }).click();
      await page.getByRole('button', { name: '保存计划版本', exact: true }).click();
      await canvas.waitFor({ state: 'hidden' });
      assert.equal(versions.length, 2);
      assert.deepEqual(versions[1].plan.nodes.map((n) => n.node_id), ['code-b', 'test-a']);
      assert.equal(versions[1].plan.layers[0].task, '编辑后仍保留删除结果');
      await page.reload({ waitUntil: 'networkidle' });
      await page.getByRole('tab', { name: '计划库', exact: true }).click(); await openEditor();
      await card('execution:code-b').waitFor();
      assert.equal(await card('execution:code-a').count(), 0);
      await card('execution:code-b').click();
      await card('execution:test-a').click({ modifiers: ['Control'] });
      assert.equal(await canvas.locator('.react-flow__node-execution.selected').count(), 2);
      await page.keyboard.press(key); await confirm();
      assert.deepEqual((await readDraft()).version.plan.nodes, []);
      checks.push({ key, deleted: 'code-a', savedVersion: 2, layoutPreserved: true, batchDelete: true });
      await page.close(); page = null;
    }
    assert.deepEqual(errors, []);
    assert(writes.every((write) => write.path.startsWith('/api/brain/plan-defs')));
    const receipt = { result: 'PASS', checks, errors };
    await fs.writeFile(path.join(output, 'receipt.json'), JSON.stringify(receipt, null, 2));
    console.log(JSON.stringify({ ...receipt, output }));
  } catch (error) {
    if (page) {
      await page.screenshot({ path: path.join(output, 'failure.png'), fullPage: true }).catch(() => {});
      await fs.writeFile(path.join(output, 'failure.html'), await page.content()).catch(() => {});
    }
    await fs.writeFile(path.join(output, 'failure.json'), JSON.stringify({ error: error.stack, errors }, null, 2));
    throw error;
  } finally { await browser.close(); server.closeAllConnections(); await new Promise((resolve) => server.close(resolve)); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
