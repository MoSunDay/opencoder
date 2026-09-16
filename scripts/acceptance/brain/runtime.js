// Real browser + control/node acceptance; invoked by worker/tests/brain_browser.rs.
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const base = process.argv[2];
const artifacts = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-brain-v2-browser-'));
async function main() {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const page = await browser.newPage({ viewport: { width: 1650, height: 1100 } });
  page.setDefaultTimeout(30000);
  const errors = []; page.on('pageerror', (e) => errors.push(e.message));
  try {
    await page.addInitScript(() => localStorage.setItem('oc_token', 'browser-fixture'));
    await page.goto(base, { waitUntil: 'networkidle' });
    await page.getByRole('radiogroup').getByText('Agent', { exact: true }).click();
    await page.getByRole('menuitem', { name: '大脑调度' }).click();
    await page.getByRole('tab', { name: '计划库' }).click();
    await page.getByRole('button', { name: '新建计划' }).click();
    await page.getByRole('button', { name: '修复—复测循环示例' }).click();
    await page.getByLabel('要做什么').fill('根据具名文档修复并提供验证依据');
    await page.getByRole('button', { name: 'after-verify', exact: true }).click();
    await page.getByLabel('下一步如何判断').fill('复测成功且证据明确时结束，否则回流修复');
    await page.getByRole('button', { name: '关闭画布' }).click();
    await page.getByRole('button', { name: '新建计划' }).click();
    assert.equal(await page.getByLabel('下一步如何判断').inputValue(), '复测成功且证据明确时结束，否则回流修复');
    await page.screenshot({ path: path.join(artifacts, 'editor.png') });
    await page.getByRole('button', { name: '提交计划' }).click();
    await page.getByLabel('计划名称').fill('浏览器 v2 修复验证');
    await page.getByLabel('一句话概述').fill('固定图版本、命名输出及局部路由');
    const saved = page.waitForResponse((r) => r.url().endsWith('/api/brain/plan-defs') && r.request().method() === 'POST');
    await page.getByRole('button', { name: '确认提交' }).click();
    const savedResponse = await saved;
    assert.equal(savedResponse.status(), 200, await savedResponse.text());
    const version = (await savedResponse.json()).version;
    assert.equal(version.plan.schema_version, 2);
    await page.getByRole('button', { name: '执行', exact: true }).click();
    await page.getByLabel('目标和交付物').fill('浏览器运行：提交经过验证的修复结果');
    await page.getByLabel('大脑所在节点').click();
    await page.locator('.ant-select-item-option').filter({ hasText: 'test-node' }).first().click();
    await page.getByLabel('文档名称', { exact: true }).fill('回归需求');
    await page.getByLabel('Markdown 正文').fill('# 问题\n修复后提交测试依据。');
    const created = page.waitForResponse((r) => r.url().endsWith('/api/brain/runs') && r.request().method() === 'POST');
    await page.getByRole('button', { name: '执行指定版本' }).click();
    const createdResponse = await created;
    assert.equal(createdResponse.status(), 202, await createdResponse.text());
    const { id } = await createdResponse.json();
    await page.locator('.brain-run-header').getByText('已完成', { exact: true }).waitFor({ timeout: 60000 });
    const snapshot = await (await page.request.get(`${base}/api/brain/runs/${id}`)).json();
    assert.equal(snapshot.phase, 'completed');
    assert.equal(Object.keys(snapshot.graph.outputs).length, 2);
    assert.equal(Object.keys(snapshot.graph.routes).length, 2);
    for (const receipt of Object.values(snapshot.graph.routes)) {
      assert.equal(receipt.context.outputs.length, 1);
      assert.ok(receipt.decision.reason);
      assert.equal(receipt.context.plan, undefined);
    }
    const first = Object.values(snapshot.graph.visits).find((v) => !v.parents.length);
    const visit = await (await page.request.get(`${base}/api/brain/runs/${id}/instances/${encodeURIComponent(first.visit)}`)).json();
    assert.equal(visit.inputs.document.name, '回归需求');
    assert.equal(visit.inputs.document.markdown, '# 问题\n修复后提交测试依据。');
    await page.locator('.react-flow__node').filter({ hasText: 'after-verify' }).click();
    await page.locator('.brain-inspector').getByText('Fixture verifies connected output').waitFor();
    await page.screenshot({ path: path.join(artifacts, 'run.png') });
    fs.writeFileSync(path.join(artifacts, 'snapshot.json'), JSON.stringify(snapshot, null, 2));
    assert.deepEqual(errors, []);
    console.log(JSON.stringify({ result: 'PASS', id, artifacts }));
  } catch (error) {
    fs.writeFileSync(path.join(artifacts, 'page-errors.json'), JSON.stringify(errors, null, 2));
    console.error('Page errors:', errors);
    await page.screenshot({ path: path.join(artifacts, 'failure.png') }).catch(() => {});
    console.error(`Browser artifacts: ${artifacts}`);
    throw error;
  } finally { await browser.close(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
