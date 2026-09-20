// Real browser + control/node acceptance; invoked by worker/tests/brain_browser.rs.
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const base = process.argv[2];
const artifacts = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-brain-v3-browser-'));
async function main() {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const page = await browser.newPage({ viewport: { width: 1650, height: 1100 } });
  page.setDefaultTimeout(30000);
  const errors = []; const detailRequests = new Set();
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('request', (request) => { const match = new URL(request.url()).pathname.match(/^\/api\/executions\/([^/]+)$/); if (match) detailRequests.add(decodeURIComponent(match[1])); });
  try {
    await page.addInitScript(() => localStorage.setItem('oc_token', 'browser-fixture'));
    await page.goto(base, { waitUntil: 'networkidle' });
    await page.getByRole('radiogroup').getByText('Agent', { exact: true }).click();
    await page.getByRole('menuitem', { name: '大脑调度' }).click();
    await page.getByRole('tab', { name: '计划库' }).click();
    await page.getByRole('button', { name: '新建计划' }).click();
    await page.getByLabel('计划名称').fill('浏览器五类型调度');
    await page.getByLabel('目标和交付物').fill('验证五种能力执行，汇总节点证据');
    for (const kind of ['agent', 'team', 'dag', 'todos', 'operator']) await page.getByRole('checkbox', { name: new RegExp(`浏览器验收 ${kind}`) }).check();
    assert.equal(await page.getByRole('button', { name: '添加路由' }).count(), 0);
    await page.getByRole('button', { name: '关闭画布' }).click();
    await page.getByRole('button', { name: '新建计划' }).click();
    assert.equal(await page.getByLabel('计划名称').inputValue(), '浏览器五类型调度');
    assert.equal(await page.getByRole('checkbox', { checked: true }).count(), 5);
    await page.screenshot({ path: path.join(artifacts, 'editor.png'), animations: 'disabled' });
    const saved = page.waitForResponse((response) => response.url().endsWith('/api/brain/plan-defs') && response.request().method() === 'POST');
    await page.getByRole('button', { name: '保存计划' }).click();
    const savedResponse = await saved;
    assert.equal(savedResponse.status(), 200, await savedResponse.text());
    const version = (await savedResponse.json()).version;
    assert.equal(version.plan.schema_version, 3);
    assert.equal(version.plan.capability_ids.length, 5);
    await page.getByRole('button', { name: /^执\s*行$/ }).click();
    await page.getByLabel('大脑所在节点').click();
    await page.locator('.ant-select-item-option').filter({ hasText: 'test-node' }).first().click();
    const created = page.waitForResponse((response) => response.url().endsWith('/api/brain/runs') && response.request().method() === 'POST');
    await page.getByRole('button', { name: '开始执行' }).click();
    const createdResponse = await created;
    assert.equal(createdResponse.status(), 202, await createdResponse.text());
    const { run_id: id } = await createdResponse.json();
    assert.ok(id);
    const deadline = Date.now() + 180000;
    let completed;
    while (Date.now() < deadline) {
      const response = await page.request.get(`${base}/api/brain/runs/${id}/view`);
      assert.equal(response.status(), 200, await response.text());
      completed = await response.json();
      assert.ok(!['failed', 'blocked', 'cancelled'].includes(completed.run?.phase), JSON.stringify(completed));
      if (completed.run?.phase === 'completed') break;
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
    assert.equal(completed?.run.phase, 'completed', JSON.stringify(completed));
    // Reload proves rounds and reasons are durable, independent of live events.
    await page.reload({ waitUntil: 'domcontentloaded' });
    await page.locator('.brain-run-header').getByText('已完成', { exact: true }).waitFor();
    const view = await (await page.request.get(`${base}/api/brain/runs/${id}/view`)).json();
    assert.equal(view.rounds.length, 2);
    const operations = view.rounds.flatMap((round) => round.operations);
    assert.deepEqual(operations.map((operation) => operation.execution_kind).sort(), ['agent', 'dag', 'operator', 'team', 'todos']);
    assert.ok(view.rounds.every((round) => round.decisions.length));
    assert.ok(operations.every((operation) => operation.execution_created && operation.status === 'done'));
    await page.locator('.brain-rounds .ant-collapse-header').filter({ hasText: '第 1 轮' }).click();
    await page.getByRole('button', { name: `查看执行 ${operations[0].execution_id}`, exact: true }).click();
    const drawer = page.getByRole('dialog', { name: '能力执行明细' });
    let currentRound = 1;
    for (const operation of operations) {
      if (operation.round !== currentRound) {
        await drawer.getByLabel('明细调度轮次').click();
        await page.locator('.ant-select-item-option-content').getByText(`第 ${operation.round} 轮`, { exact: true }).click();
        currentRound = operation.round;
      }
      await drawer.getByLabel('选择执行').click();
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option').filter({ hasText: operation.execution_id }).click();
      await drawer.getByText('所属节点', { exact: true }).waitFor();
      const detail = await (await page.request.get(`${base}/api/executions/${operation.execution_id}`)).json();
      assert.equal(detail.execution.kind, operation.execution_kind);
      await drawer.locator('.execution-view-full').getByText(detail.execution.node_id, { exact: true }).waitFor();
      assert.equal(await drawer.getByRole('button', { name: '取消（终止）', exact: true }).count(), 0);
      await page.screenshot({ path: path.join(artifacts, `panel-${operation.execution_kind}.png`), animations: 'disabled' });
    }
    for (const operation of operations) assert.ok(detailRequests.has(operation.execution_id), operation.execution_id);
    await drawer.locator('button.ant-drawer-close').click();
    await drawer.waitFor({ state: 'hidden' });
    await page.screenshot({ path: path.join(artifacts, 'run.png'), animations: 'disabled' });
    fs.writeFileSync(path.join(artifacts, 'view.json'), JSON.stringify(view, null, 2));
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ result: 'PASS', id, types: operations.map((operation) => operation.execution_kind), artifacts }));
  } catch (error) {
    fs.writeFileSync(path.join(artifacts, 'page-errors.json'), JSON.stringify(errors, null, 2));
    await page.screenshot({ path: path.join(artifacts, 'failure.png') }).catch(() => {});
    console.error(`Browser artifacts: ${artifacts}`);
    throw error;
  } finally { await browser.close(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
