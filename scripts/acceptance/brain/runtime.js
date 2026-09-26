// Schema 7 browser acceptance: real HTTP, durable scheduler and execution panels.
// The Rust harness supplies only a deterministic model transport.
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const base = process.argv[2];
assert(base, 'Fleet base URL required');
const artifacts = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-brain-v7-browser-'));

async function chooseCapability(page, label) {
  await page.getByRole('combobox', { name: '绑定能力', exact: true }).click();
  await page.locator('.ant-select-dropdown:visible .ant-select-item-option').filter({ hasText: label }).first().click();
  await page.locator('.brain-execution-node.selected span').getByText(label, { exact: true }).waitFor();
}

async function addNode(page, layerName, nodeName, label) {
  await page.locator('.brain-layer-box').filter({ hasText: layerName }).getByRole('button', { name: '＋ 并行执行节点' }).click();
  await page.getByLabel('执行节点名称', { exact: true }).fill(nodeName);
  await page.getByLabel('执行节点任务', { exact: true }).fill(`执行 ${nodeName} 并返回证据`);
  await chooseCapability(page, label);
}

async function addLayer(page, index, name) {
  await page.getByRole('button', { name: index === 1 ? '添加第一个里程碑' : '＋ 里程碑', exact: true }).click();
  await page.getByLabel('里程碑名称', { exact: true }).fill(name);
  await page.getByLabel('里程碑目标', { exact: true }).fill(`完成 ${name} 的能力调度`);
  await page.getByLabel('里程碑达成标准', { exact: true }).fill('所有执行结果都有 node-owned child result');
}

function assertRun(view) {
  assert.equal(view.schema_version, 7);
  assert.equal(view.run.phase, 'completed', view.run.error);
  assert.equal(view.run.round, 2);
  assert.equal(view.run.valid_layers, 2);
  const visits = view.events.filter((event) => event.event_type === 'layer_started');
  assert.deepEqual(visits.map(({ round, layer }) => [round, layer]), [[1, 1], [1, 2], [2, 1], [2, 2]]);
  assert.equal(visits[2].decision_summary, 'reflect_and_return');
  assert(visits[2].reflection, 'return must carry reflection context');
  assert.deepEqual(visits.map((event) => view.operations.filter((op) => op.activation === event.activation).length), [2, 1, 2, 1]);
  assert.equal(new Set(view.operations.map((op) => op.execution_id)).size, 6);
  assert.deepEqual([...new Set(view.operations.map((op) => op.execution_kind))].sort(), ['agent', 'operator']);
  assert(view.operations.every((op) => op.status === 'done'));
  for (let i = 0; i < visits.length - 1; i++) {
    const barrier = view.events.find((event) => event.activation === visits[i].activation && event.event_type === 'layer_barrier_reached');
    assert(barrier && barrier.seq < visits[i + 1].seq, 'next layer started before the barrier');
    for (const op of view.operations.filter((row) => row.activation === visits[i].activation)) {
      assert(view.events.some((event) => event.execution_id === op.execution_id && event.event_type === 'operation_terminal' && event.seq < barrier.seq),
        `operation ${op.execution_id} did not finish before the layer decision`);
    }
  }
  return visits;
}

async function selectVisit(page, round, layer) {
  await page.getByRole('combobox', { name: '选择历史层激活' }).click();
  await page.locator('.ant-select-dropdown:visible .ant-select-item-option').filter({ hasText: `第 ${round} 轮 · 第 ${layer} 层 ·` }).click();
}

async function main() {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const page = await browser.newPage({ viewport: { width: 1650, height: 1100 } });
  page.setDefaultTimeout(30000);
  const errors = []; const details = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('request', (request) => { if (/\/api\/executions\/[^/?]+$/.test(request.url())) details.push(request.url()); });
  try {
    await page.addInitScript(() => localStorage.setItem('oc_token', 'browser-fixture'));
    await page.goto(base, { waitUntil: 'networkidle' });
    await page.getByRole('radiogroup').getByText('Agent', { exact: true }).click();
    await page.getByRole('menuitem', { name: '大脑调度' }).click();
    await page.getByRole('tab', { name: '工作台' }).waitFor();
    assert.equal(await page.getByRole('button', { name: '创建并执行计划' }).count(), 0);
    await page.getByRole('tab', { name: '计划库' }).click();
    await page.getByRole('button', { name: '新建计划', exact: true }).click();

    await addLayer(page, 1, 'Coding');
    await addNode(page, 'Coding', '实现任务', 'Agent · act');
    await addNode(page, 'Coding', '并行检查', 'Operator · act');
    await addLayer(page, 2, '测试');
    await addNode(page, '测试', '验证任务', 'Operator · act');
    const layers = page.locator('.react-flow__node-layer');
    await layers.nth(1).locator('[data-handleid="return-out"]').dragTo(layers.nth(0).locator('[data-handleid="return-in"]'));
    await page.getByLabel('扭转条件', { exact: true }).fill('测试证据要求回到 Coding 整改');
    await page.screenshot({ path: path.join(artifacts, 'canvas.png'), animations: 'disabled' });

    await page.getByRole('button', { name: '关闭画布', exact: true }).click();
    await page.getByRole('button', { name: '新建计划', exact: true }).click();
    assert.equal(await page.locator('.brain-execution-node').count(), 3, 'canvas draft must survive closing');
    assert.equal(await page.locator('.react-flow__edge').count(), 2, 'forward and return transitions must survive closing');
    await page.getByRole('button', { name: '下一步：计划信息', exact: true }).click();
    await page.getByLabel('计划名称', { exact: true }).fill('大脑 schema 7 闭环验收');
    await page.getByLabel('整体目标与交付物', { exact: true }).fill('层内并行、测试回退、再次验证后完成');
    const saved = page.waitForResponse((response) => response.url().endsWith('/api/brain/plan-defs') && response.request().method() === 'POST');
    await page.getByRole('button', { name: '保存计划版本', exact: true }).click();
    const response = await saved;
    assert.equal(response.status(), 200, await response.text());
    const { version } = await response.json();
    assert.equal(version.plan.schema_version, 7);
    assert.deepEqual(version.plan.nodes.map((node) => node.capability_id), ['builtin-agent-act', 'builtin-operator', 'builtin-operator']);
    assert.equal(version.plan.transitions.length, 2);
    assert(version.plan.transitions.some((edge) => edge.from === version.plan.layers[1].layer_id && edge.to === version.plan.layers[0].layer_id
      && edge.condition === '测试证据要求回到 Coding 整改'));

    await page.getByRole('button', { name: /^执\s*行$/ }).click();
    await page.getByLabel('大脑所在节点', { exact: true }).click();
    await page.locator('.ant-select-dropdown:visible .ant-select-item-option').filter({ hasText: 'test-node' }).first().click();
    const created = page.waitForResponse((row) => row.url().endsWith('/api/brain/runs') && row.request().method() === 'POST');
    await page.getByRole('button', { name: '开始执行', exact: true }).click();
    const receipt = await created;
    assert.equal(receipt.status(), 202, await receipt.text());
    const id = (await receipt.json()).run_id;
    await page.getByText('有效通过 2 / 2 层', { exact: true }).waitFor({ timeout: 120000 });
    const view = await page.evaluate(async (runId) => {
      const response = await fetch(`/api/brain/runs/${runId}/layered`, { headers: { Authorization: `Bearer ${localStorage.getItem('oc_token')}` } });
      if (!response.ok) throw new Error(`GET layered: ${response.status}`);
      return response.json();
    }, id);
    const visits = assertRun(view);
    for (const event of visits) {
      const detail = await page.evaluate(async ({ runId, layer, activation }) => {
        const response = await fetch(`/api/brain/runs/${runId}/layered/rounds/${layer}?activation=${activation}`, {
          headers: { Authorization: `Bearer ${localStorage.getItem('oc_token')}` },
        });
        if (!response.ok) throw new Error(`GET visit: ${response.status}`);
        return response.json();
      }, { runId: id, layer: event.layer, activation: event.activation });
      assert.equal(detail.visit.activation, event.activation);
      assert.equal(detail.nodes.flatMap((node) => node.operations).length, event.layer === 1 ? 2 : 1);
    }
    for (const round of [1, 2]) {
      await selectVisit(page, round, 1);
      await page.locator('.brain-execution-node').filter({ hasText: '实现任务' }).getByText('Agent · act', { exact: true }).waitFor();
      const operation = view.operations.find((op) => op.round === round && op.node_id === version.plan.nodes[0].node_id);
      await page.locator('.brain-execution-node').filter({ hasText: '实现任务' }).click();
      const drawer = page.getByRole('dialog', { name: '能力执行明细', exact: true });
      await drawer.locator('.execution-view-full').waitFor();
      await drawer.getByText('node-owned child result', { exact: false }).last().waitFor();
      assert(details.some((url) => url.endsWith(`/api/executions/${operation.execution_id}`)), 'execution panel must fetch the selected attempt by ID');
      await drawer.locator('button.ant-drawer-close').click();
      await drawer.waitFor({ state: 'hidden' });
    }
    await selectVisit(page, 2, 2);
    const operator = view.operations.find((op) => op.round === 2 && op.node_id === version.plan.nodes[2].node_id);
    await page.locator('.brain-execution-node').filter({ hasText: '验证任务' }).getByText('Operator · act', { exact: true }).waitFor();
    await page.locator('.brain-execution-node').filter({ hasText: '验证任务' }).click();
    const operatorDrawer = page.getByRole('dialog', { name: '能力执行明细', exact: true });
    await operatorDrawer.locator('.execution-view-full').waitFor();
    await operatorDrawer.getByText('node-owned child result', { exact: false }).last().waitFor();
    assert(details.some((url) => url.endsWith(`/api/executions/${operator.execution_id}`)), 'Operator panel must fetch its own execution ID');
    await page.screenshot({ path: path.join(artifacts, 'completed.png'), animations: 'disabled' });
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ result: 'PASS', schema_version: 7, run_id: id, activations: visits.length, operations: view.operations.length, artifacts }));
  } catch (error) {
    await page.screenshot({ path: path.join(artifacts, 'failure.png') });
    fs.writeFileSync(path.join(artifacts, 'failure.html'), await page.content());
    console.error(JSON.stringify({ artifacts, errors, text: (await page.locator('body').innerText()).slice(-6000) }));
    throw error;
  } finally { await browser.close(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
