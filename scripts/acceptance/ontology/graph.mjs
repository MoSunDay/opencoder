import assert from 'node:assert/strict';
import { randomUUID } from 'node:crypto';
import { writeFile } from 'node:fs/promises';
import { chromium } from '../../../crates/web/spa/node_modules/playwright-core/index.mjs';

const [base, output] = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(),
  args: ['--no-sandbox', '--disable-dev-shm-usage', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'] });
const context = await browser.newContext({ viewport: { width: 1280, height: 850 } });
const page = await context.newPage();
const errors = [];
page.on('pageerror', (error) => errors.push(error.message));
const api = async (path, body) => {
  const response = await context.request.fetch(`${base}/api/ontology/envs/debug${path}`, {
    method: body ? 'POST' : 'GET', data: body, headers: { Authorization: 'Bearer ontology-fixture' },
  });
  assert(response.ok(), `${path}: ${response.status()} ${await response.text()}`); return response.json();
};
const settle = async () => {
  await page.locator('.graph-3d[aria-busy="false"]:visible').waitFor({ timeout: 20000 });
  await page.waitForFunction(() => [...document.querySelectorAll('.graph-3d-node')].some((node) => getComputedStyle(node).visibility === 'visible'));
};
const positions = () => page.locator('.graph-3d-node').evaluateAll((nodes) => nodes.map((node) => ({
  id: node.dataset.nodeId, x: Number(node.dataset.anchorX), y: Number(node.dataset.anchorY),
})));
const movement = (a, b) => Math.max(...a.map((node) => { const next = b.find((item) => item.id === node.id); return next ? Math.hypot(next.x - node.x, next.y - node.y) : 0; }));
const closeDrawer = async () => { await page.locator('.ant-drawer-open .ant-drawer-close').click(); await page.locator('.ant-drawer-open').waitFor({ state: 'hidden' }); };
const chooseAspect = async (name) => {
  await page.getByRole('combobox', { name: '切面选择' }).click();
  await page.locator('.ant-select-dropdown:visible .ant-select-item-option').getByText(name, { exact: true }).click();
  await settle();
};
try {
  const suffix = randomUUID().slice(0, 8);
  const type = (await api('/entity-types', { key: `graph_3d_${suffix}`, name: '组件' })).item;
  const nodes = [];
  for (const name of ['组件 A', '组件 B', '组件 C']) nodes.push((await api('/entities', {
    request_id: randomUUID(), entity_type_id: type.id, name, source: '# 来源', ext: '# 说明',
  })).item);
  const relation = (await api('/relationship-types', { key: `graph_3d_link_${suffix}`, name: '关联', source_entity_type_id: type.id, target_entity_type_ids: [type.id] })).item;
  for (const [from, to] of [[0, 1], [1, 0], [1, 2], [2, 0]]) await api('/relationships', {
    relationship_type_id: relation.id, source_entity_id: nodes[from].id, target_entity_id: nodes[to].id,
  });
  const aspect = (await api('/graph-aspects', { aspect_key: `graph_3d_${suffix}`, name: '三维组件', entity_type_ids: [type.id],
    relationship_type_ids: [relation.id], default_center_ids: [nodes[0].id], default_upstream_depth: 3, default_downstream_depth: 3 })).item;
  await page.goto(`${base}/?token=ontology-fixture#ontology_aspect=${aspect.aspect_key}&ontology_center=${nodes[0].id}`);
  await settle();
  assert.equal(await page.locator('.graph-3d canvas').count(), 1);
  assert.equal(await page.locator('.graph-3d-node').count(), 3);
  assert.equal(await page.locator('.graph-3d-relation').count(), 4);
  await page.screenshot({ path: `${output}/initial.png` });
  let before = await positions();
  const viewport = await page.locator('.graph-viewport:visible').boundingBox();
  await page.mouse.move(viewport.x + 20, viewport.y + 60); await page.mouse.down();
  await page.mouse.move(viewport.x + 150, viewport.y + 130, { steps: 12 }); await page.mouse.up();
  await page.waitForTimeout(350);
  assert(movement(before, await positions()) > 5, 'Dragging must rotate the 3D scene');
  before = await positions();
  await page.getByRole('button', { name: '放大', exact: true }).click(); await page.waitForTimeout(150);
  assert(movement(before, await positions()) > 1, 'Zoom must change projected positions');
  await page.getByRole('button', { name: '重置视角', exact: true }).click(); await page.waitForTimeout(150);
  before = await positions();
  await page.getByRole('button', { name: '刷新当前观测' }).click(); await settle();
  assert(movement(before, await positions()) < 10, 'Identical refresh must preserve the layout and camera');
  await page.locator('.graph-3d-node').filter({ has: page.getByText('组件 A', { exact: true }) }).getByRole('button', { name: '组件 A', exact: true }).click();
  await page.locator('.ant-drawer-open').getByRole('tab', { name: '基本信息' }).waitFor(); await closeDrawer();
  await page.locator('.graph-3d-relation').first().click();
  await page.locator('.ant-drawer-open').waitFor(); await closeDrawer();
  await page.getByRole('button', { name: '另存切面' }).click();
  const dialog = page.locator('.ant-modal[role="dialog"]:visible');
  await dialog.getByLabel('名称', { exact: true }).fill(`三维组件副本 ${suffix}`);
  await dialog.getByRole('button', { name: /^保\s*存$/ }).click(); await dialog.waitFor({ state: 'hidden' });
  await chooseAspect(`三维组件副本 ${suffix}`);
  await page.getByRole('button', { name: '放大画布', exact: true }).click();
  await page.getByRole('button', { name: '恢复画布', exact: true }).click();
  const measurements = [];
  for (const width of [1920, 1280, 768, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.waitForTimeout(250);
    await page.getByRole('button', { name: '自动取景', exact: true }).click();
    await page.mouse.move(0, 0);
    await page.waitForTimeout(150);
    const measurement = await page.evaluate(() => {
      const host = document.querySelector('.graph-3d'); const canvas = host.querySelector('canvas');
      const rect = host.getBoundingClientRect(), canvasRect = canvas.getBoundingClientRect();
      return { width: innerWidth, pageWidth: document.documentElement.scrollWidth, host: { width: rect.width, height: rect.height },
        canvas: { width: canvasRect.width, height: canvasRect.height } };
    });
    assert(measurement.pageWidth <= width + 1, 'Page must not overflow');
    assert(Math.abs(measurement.host.width - measurement.canvas.width) < 2);
    assert(Math.abs(measurement.host.height - measurement.canvas.height) < 2);
    await page.screenshot({ path: `${output}/graph-${width}.png` }); measurements.push(measurement);
  }
  const touch = await context.newCDPSession(page);
  await touch.send('Emulation.setTouchEmulationEnabled', { enabled: true, maxTouchPoints: 2 });
  const touchBox = await page.locator('.graph-viewport:visible').boundingBox();
  const origin = { x: touchBox.x + touchBox.width / 2, y: touchBox.y + 65 };
  before = await positions();
  await touch.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ ...origin, id: 0 }] });
  for (let step = 1; step <= 6; step++) await touch.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: origin.x + step * 8, y: origin.y + step * 4, id: 0 }] });
  await touch.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await page.waitForTimeout(200); assert(movement(before, await positions()) > 2, 'Single-finger rotation must work');
  before = await positions();
  await touch.send('Input.dispatchTouchEvent', { type: 'touchStart', touchPoints: [{ x: origin.x - 25, y: origin.y, id: 0 }, { x: origin.x + 25, y: origin.y, id: 1 }] });
  for (let step = 1; step <= 6; step++) await touch.send('Input.dispatchTouchEvent', { type: 'touchMove', touchPoints: [{ x: origin.x - 25 - step * 4, y: origin.y + step * 3, id: 0 }, { x: origin.x + 25 + step * 4, y: origin.y + step * 3, id: 1 }] });
  await touch.send('Input.dispatchTouchEvent', { type: 'touchEnd', touchPoints: [] });
  await page.waitForTimeout(200); assert(movement(before, await positions()) > 2, 'Two-finger zoom and pan must work');
  await touch.send('Emulation.setTouchEmulationEnabled', { enabled: false }); await touch.detach();
  await page.setViewportSize({ width: 1280, height: 850 });
  await page.locator('.graph-surface:visible').getByText('列表', { exact: true }).click();
  await page.locator('.graph-result-list').getByLabel('搜索完整观测结果').fill('组件 C');
  await page.getByRole('button', { name: '在图中定位' }).click();
  await page.waitForTimeout(200);
  const focus = await page.locator(`[data-node-id="${nodes[2].id}"]`).evaluate((node) => ({ x: Number(node.dataset.anchorX), y: Number(node.dataset.anchorY),
    width: node.closest('.graph-3d').clientWidth, height: node.closest('.graph-3d').clientHeight }));
  assert(Math.abs(focus.x - focus.width / 2) < 3 && Math.abs(focus.y - focus.height / 2) < 3, 'Search result must be centered');
  await page.reload(); await settle();
  assert.equal((await page.evaluate(() => JSON.parse(localStorage.getItem('oc_ontology_observation:debug')))).mode, 'aspect-observe');
  await page.route(/\/api\/ontology\/envs\/debug\/graph(?:\?|$)/, (route) => route.fulfill({ status: 500, contentType: 'application/json', body: '{"error":"graph test failure"}' }));
  await page.getByRole('button', { name: '刷新当前观测' }).click();
  await page.getByText(/更新失败，当前显示上一次结果/).waitFor(); assert.equal(await page.locator('.graph-3d-node').count(), 3);
  await page.unroute(/\/api\/ontology\/envs\/debug\/graph(?:\?|$)/);
  await page.getByRole('button', { name: /^重\s*试$/ }).click(); await settle();
  await page.locator('.graph-3d canvas').evaluate((canvas) => canvas.dispatchEvent(new Event('webglcontextlost', { cancelable: true })));
  await page.getByText('三维画布暂不可用，已显示完整列表').waitFor();
  assert.equal(await page.locator('.graph-result-list tbody tr[data-row-key]').count(), 3);
  await page.getByRole('button', { name: '重试三维画布' }).click(); await settle();
  const denseType = (await api('/entity-types', { key: `dense_${suffix}`, name: '图谱节点' })).item;
  const denseRelation = (await api('/relationship-types', { key: `dense_rel_${suffix}`, name: '连接', source_entity_type_id: denseType.id, target_entity_type_ids: [denseType.id] })).item;
  const denseNodes = [];
  for (let i = 0; i < 40; i++) denseNodes.push((await api('/entities', { request_id: randomUUID(), entity_type_id: denseType.id, name: `节点 ${String(i).padStart(2, '0')}`, source: '# 来源', ext: '# 说明' })).item);
  for (const node of denseNodes.slice(1)) await api('/relationships', { relationship_type_id: denseRelation.id, source_entity_id: denseNodes[0].id, target_entity_id: node.id });
  await api('/graph-aspects', { aspect_key: `dense_${suffix}`, name: `较大图谱 ${suffix}`, entity_type_ids: [denseType.id], relationship_type_ids: [denseRelation.id], default_center_ids: [denseNodes[0].id], default_upstream_depth: 1, default_downstream_depth: 1 });
  await page.getByRole('button', { name: '刷新当前观测' }).click(); await settle(); await chooseAspect(`较大图谱 ${suffix}`);
  await page.waitForFunction(() => document.querySelectorAll('.graph-3d-node').length === 30);
  await settle();
  assert.equal(await page.locator('.graph-3d-node').count(), 30);
  await page.getByRole('button', { name: '继续展开' }).click();
  await page.waitForFunction(() => document.querySelectorAll('.graph-3d-node').length === 40);
  await settle();
  assert.equal(await page.locator('.graph-3d-node').count(), 40);
  await page.getByRole('button', { name: '自动取景' }).click(); await page.mouse.move(0, 0);
  await page.screenshot({ path: `${output}/graph-expanded.png` });
  const sync = await api("/sync/check"); assert.equal(sync.in_sync, true);
  assert.deepEqual(errors, []);
  const receipt = { passed: true, rotation: true, zoom: true, details: true, aspects: true, refresh_preserves_view: true,
    search_focus: true, observation_link: true, observation_restore: true, query_failure: true, context_loss_retry: true, touch_rotation: true, touch_zoom_pan: true, expanded_nodes: 40, catalog_in_sync: true, measurements };
  await writeFile(`${output}/browser.json`, JSON.stringify(receipt, null, 2));
  console.log(JSON.stringify(receipt));
} catch (error) {
  await page.screenshot({ path: `${output}/failure.png` }).catch(() => {});
  await writeFile(`${output}/failure.txt`, `${error.stack}\nPage errors: ${JSON.stringify(errors)}\n${await page.locator('body').innerText()}`);
  throw error;
} finally { await browser.close(); }
