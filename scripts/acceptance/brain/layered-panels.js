const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

async function inspectPanels({ base, token, id, view, operations, marker, evidence }) {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const page = await browser.newPage({ viewport: { width: 1650, height: 1100 } });
  const errors = []; const fetched = new Map(); const panels = [];
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('response', async (response) => {
    if (!/\/api\/executions\/[^/?]+$/.test(response.url()) || !response.ok()) return;
    try { const value = await response.json(); fetched.set(value.execution.id, value.execution); } catch (error) { errors.push(error.message); }
  });
  try {
    await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
    await page.goto(`${base}/?brain_run=${id}`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('radiogroup').getByText('Agent', { exact: true }).click();
    await page.getByRole('menuitem', { name: '大脑调度' }).click();
    await page.locator('.brain-run').first().getByText('已完成', { exact: true }).waitFor();
    await page.locator('.react-flow__controls-fitview').click();
    await page.screenshot({ path: path.join(evidence, 'overview.png'), animations: 'disabled' });
    for (const op of operations) {
      await page.getByLabel('选择历史层激活', { exact: true }).click();
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option').filter({ hasText: `第 ${op.round} 轮 · 第 ${op.layer} 层 ·` }).click();
      await page.getByTestId(`rf__node-${op.node_id}`).locator('.brain-milestone-node strong').click();
      const drawer = page.getByRole('dialog', { name: '能力执行明细', exact: true });
      await drawer.waitFor();
      await drawer.locator('.execution-view-full').waitFor();
      if (op.execution_kind === 'dag') {
        await drawer.getByTestId('rf__node-analyze').click();
        const logs = page.locator('.dag-logs-drawer');
        await logs.getByText(marker, { exact: false }).first().waitFor();
        await logs.getByText('已结束', { exact: true }).waitFor();
        await page.screenshot({ path: path.join(evidence, 'dag-step.png'), animations: 'disabled' });
        await logs.getByRole('button', { name: '关闭', exact: true }).click();
      } else if (op.execution_kind === 'todos') {
        await drawer.locator('.todo-parent-heading').waitFor();
        await drawer.getByText('1/1 已通过', { exact: true }).waitFor();
      } else if (op.execution_kind === 'team') {
        await drawer.locator('.execution-team-turn').first().waitFor();
        await drawer.getByText(marker, { exact: false }).last().waitFor();
      } else if (op.execution_kind === 'brain') {
        await drawer.locator('.brain-milestone-node').waitFor();
        await drawer.locator('.brain-run').getByText('已完成', { exact: true }).waitFor();
      } else {
        await drawer.getByRole('img', { name: 'Agent', exact: true }).first().waitFor();
        await drawer.getByText(marker, { exact: false }).last().waitFor();
      }
      assert.equal(fetched.get(op.execution_id)?.kind, op.execution_kind, 'detail must come from its real execution ID');
      await page.screenshot({ path: path.join(evidence, `${op.execution_kind}.png`), animations: 'disabled' });
      panels.push({ kind: op.execution_kind, id: op.execution_id, content: 'PASS' });
      await drawer.locator('button.ant-drawer-close').click(); await drawer.waitFor({ state: 'hidden' });
    }
    const firstLayer = page.locator('.brain-run > .ant-collapse .ant-collapse-header').filter({ hasText: '第 1 轮 · 第 1 层 ·' });
    if (await firstLayer.getAttribute('aria-expanded') !== 'true') await firstLayer.click();
    const reason = view.events.find((event) => event.layer === 1 && event.event_type === 'layer_started').reason_summary;
    await page.getByText(reason, { exact: true }).waitFor();
    assert.equal(await page.getByText('本层决策：判断中', { exact: true }).count(), 0);
    await page.screenshot({ path: path.join(evidence, 'history.png'), animations: 'disabled' });
    await page.setViewportSize({ width: 390, height: 844 });
    await page.screenshot({ path: path.join(evidence, 'narrow.png'), animations: 'disabled' });
    assert.deepEqual(errors, []); return panels;
  } catch (error) {
    await page.screenshot({ path: path.join(evidence, 'browser-failure.png') });
    fs.writeFileSync(path.join(evidence, 'browser-failure.html'), await page.content());
    throw error;
  } finally { await browser.close(); }
}
module.exports = { inspectPanels };
