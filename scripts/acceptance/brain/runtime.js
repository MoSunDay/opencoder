// Real browser + control/node acceptance; only the model transport is scripted.
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const base = process.argv[2];
const artifacts = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-brain-layered-browser-'));
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
    await page.getByRole('tab', { name: '计划库' }).click();
    await page.getByRole('button', { name: '新建计划', exact: true }).click();
    for (let index = 1; index <= 2; index++) {
      await page.getByRole('button', { name: index === 1 ? '添加第一个里程碑' : '＋ 新增层级', exact: true }).click();
      await page.getByLabel('里程碑名称', { exact: true }).fill(index === 1 ? '收集证据' : '验证结果');
      await page.getByLabel('里程碑目标', { exact: true }).fill('返回一次明确的能力执行结果');
      await page.getByLabel('达成标准', { exact: true }).fill('结果包含 node-owned child result');
      const capability = page.getByRole('combobox', { name: '挂载能力', exact: true });
      await capability.fill('Execute an explicit host operation');
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option').first().click();
      await page.locator('.brain-milestone-inspector h3').click();
    }
    await page.getByRole('button', { name: '关闭画布', exact: true }).click();
    await page.getByRole('button', { name: '新建计划', exact: true }).click();
    await page.locator('.brain-milestone-node strong').getByText('验证结果', { exact: true }).click();
    assert.equal(await page.getByLabel('里程碑名称', { exact: true }).inputValue(), '验证结果');
    await page.getByRole('button', { name: '下一步：计划信息', exact: true }).click();
    await page.getByLabel('计划名称', { exact: true }).fill('里程碑计划浏览器验收');
    await page.getByLabel('整体目标与交付物', { exact: true }).fill('执行两个里程碑并验证明细');
    const saved = page.waitForResponse((r) => r.url().endsWith('/api/brain/plan-defs') && r.request().method() === 'POST');
    await page.getByRole('button', { name: '保存计划版本', exact: true }).click();
    const response = await saved;
    assert.equal(response.status(), 200, await response.text());
    const { version } = await response.json();
    assert.equal(version.plan.schema_version, 6); assert.equal(version.plan.edges.length, 0);
    assert.deepEqual(version.plan.nodes.map((node) => node.layer), [1, 2]);
    assert.equal(version.plan.max_rounds, 5);
    await page.getByRole('button', { name: /^执\s*行$/ }).click();
    await page.getByLabel('大脑所在节点', { exact: true }).click();
    await page.locator('.ant-select-dropdown:visible .ant-select-item-option').filter({ hasText: 'test-node' }).first().click();
    const created = page.waitForResponse((r) => r.url().endsWith('/api/brain/runs') && r.request().method() === 'POST');
    await page.getByRole('button', { name: '开始执行', exact: true }).click();
    const receipt = await created; assert.equal(receipt.status(), 202, await receipt.text());
    await page.getByText('有效通过 2 / 2 层', { exact: true }).waitFor();
    await page.locator('.brain-milestone-node').filter({ hasText: '验证结果' }).click();
    await page.getByText('能力执行明细', { exact: true }).waitFor();
    await page.waitForFunction(() => document.querySelector('.ant-drawer-body')?.textContent.includes('node-owned child result'));
    assert.ok(details.length > 0, 'existing execution detail API must be used');
    await page.screenshot({ path: path.join(artifacts, 'capability-detail.png'), animations: 'disabled' });
    assert.deepEqual(errors, []); console.log(JSON.stringify({ result: 'passed', artifacts }));
  } catch (error) {
    await page.screenshot({ path: path.join(artifacts, 'failure.png') });
    fs.writeFileSync(path.join(artifacts, 'failure.html'), await page.content());
    console.error(JSON.stringify({ artifacts, errors, text: (await page.locator('body').innerText()).slice(-6000) }));
    throw error;
  } finally { await browser.close(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
