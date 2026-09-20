// Real browser + Control/Worker HTTP. Only the model transport is scripted.
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
  const errors = []; page.on('pageerror', (error) => errors.push(error.message));
  try {
    await page.addInitScript(() => localStorage.setItem('oc_token', 'browser-fixture'));
    await page.goto(base, { waitUntil: 'networkidle' });
    console.log(JSON.stringify({ stage: 'page_opened' }));
    await page.getByRole('radiogroup').getByText('Agent', { exact: true }).click();
    await page.getByRole('menuitem', { name: '大脑调度' }).click();
    await page.getByRole('button', { name: '开始新任务' }).click();
    await page.getByLabel('目标和交付物').fill('浏览器验证 v3 轮次与执行索引');
    await page.getByLabel('大脑所在节点').click();
    await page.locator('.ant-select-item-option').filter({ hasText: 'test-node' }).first().click();
    await page.getByRole('button', { name: '添加工程参数' }).click();
    await page.getByLabel('工程参数名').fill('repo');
    const repo = { name: '输入验证', branch: 'feature/参数 with spaces', commit: '0123456789' };
    await page.getByLabel('工程参数值').fill(JSON.stringify(repo));
    const created = page.waitForResponse((r) => r.url().endsWith('/api/brain/runs') && r.request().method() === 'POST');
    await page.getByRole('button', { name: '开始调度' }).click();
    const response = await created;
    assert.equal(response.status(), 202, await response.text());
    const submitted = response.request().postDataJSON();
    assert.equal(submitted.schema_version, 3);
    assert.deepEqual(submitted.inputs.repo, repo);
    assert.equal(submitted.mode, undefined);
    const { run_id: id } = await response.json();
    console.log(JSON.stringify({ stage: 'submitted', run_id: id }));
    assert.equal(id, submitted.id);
    let snapshot;
    const deadline = Date.now() + 120000;
    do {
      snapshot = await (await page.request.get(`${base}/api/brain/runs/${id}`)).json();
      assert(!['blocked', 'failed', 'cancelled'].includes(snapshot.run.phase), JSON.stringify(snapshot.run));
      if (snapshot.run.phase === 'completed') break;
      await new Promise((resolve) => setTimeout(resolve, 500));
    } while (Date.now() < deadline);
    assert.equal(snapshot.run.phase, 'completed', JSON.stringify(snapshot));
    await page.locator('.brain-run-header').getByText('已完成', { exact: true }).waitFor();
    console.log(JSON.stringify({ stage: 'completed', run_id: id }));
    assert.equal(new URL(page.url()).searchParams.get('brain_run'), id);
    await page.getByLabel('大脑调度总览画布').waitFor();
    assert.equal(await page.locator('.brain-summary-node').count(), 4);
    assert.equal(await page.locator('.brain-run .react-flow').count(), 0);
    await page.screenshot({ path: path.join(artifacts, 'overview.png'), animations: 'disabled' });
    assert.equal(snapshot.run.phase, 'completed');
    const operation = snapshot.operations[0];
    assert.equal(operation.status, 'done');
    const executionResponse = page.waitForResponse((r) => r.url().endsWith(`/api/executions/${operation.execution_id}`));
    await page.getByRole('button', { name: `查看执行 ${operation.execution_id}` }).click();
    const execution = await (await executionResponse).json();
    assert.deepEqual(execution.request.input.scheduler_inputs.request, repo);
    await page.getByText('会话消息', { exact: true }).waitFor();
    await page.getByText('child scheduler output', { exact: true }).first().waitFor();
    assert.equal(await page.getByRole('button', { name: '在原节点恢复' }).count(), 0);
    await page.screenshot({ path: path.join(artifacts, 'execution.png'), animations: 'disabled' });
    assert.deepEqual(errors, []);
    const receipt = { result: 'PASS', schema_version: 3, run_id: id, execution_id: operation.execution_id, artifacts };
    fs.writeFileSync(path.join(artifacts, 'receipt.json'), JSON.stringify(receipt, null, 2));
    console.log(JSON.stringify(receipt));
  } catch (error) {
    fs.writeFileSync(path.join(artifacts, 'page-errors.json'), JSON.stringify(errors, null, 2));
    await page.screenshot({ path: path.join(artifacts, 'failure.png') }).catch(() => {});
    console.error(`Browser artifacts: ${artifacts}`);
    throw error;
  } finally { await browser.close(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
