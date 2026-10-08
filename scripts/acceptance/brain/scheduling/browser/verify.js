// Inspect the completed real run; do not intercept or replace API/model responses.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { chromium } = require('../../../../../crates/web/spa/node_modules/playwright-core');

async function main() {
  const [base, tokenFile, evidence, output] = process.argv.slice(2);
  assert(base && tokenFile && evidence, 'usage: node verify.js BASE TOKEN_FILE CASE_EVIDENCE_DIR [NEW_OUTPUT_DIR]');
  const report = JSON.parse(fs.readFileSync(path.join(evidence, 'result.json')));
  const record = report.cases.find(row => row.case === 'closed-loop' && row.result === 'PASS');
  assert(record, 'a passing real closed-loop run is required');
  const id = record.evidence.run_id;
  const view = JSON.parse(fs.readFileSync(path.join(evidence, 'runs', `${id}.json`)));
  const directory = output || path.join(evidence, 'browser');
  assert(!fs.existsSync(directory), 'browser evidence is immutable; choose a new output directory');
  fs.mkdirSync(directory, { mode: 0o700 });
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(),
    args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const page = await browser.newPage({ viewport: { width: 1920, height: 1100 } });
  const errors = []; const fetched = new Map(); const checked = [];
  page.on('pageerror', error => errors.push(error.message));
  page.on('response', async response => {
    if (/\/api\/executions\/[^/?]+$/.test(response.url()) && response.ok()) {
      try {
        const value = await response.json();
        if (value.execution) fetched.set(value.execution.id, value);
      } catch (error) { errors.push(`execution response: ${error.message}`); }
    }
  });
  try {
    await page.addInitScript(token => localStorage.setItem('oc_token', token), fs.readFileSync(tokenFile, 'utf8').trim());
    await page.goto(`${base}/?brain_run=${id}`, { waitUntil: 'domcontentloaded' });
    await page.getByRole('tablist', { name: '导航分类', exact: true }).getByRole('tab', { name: 'Agent', exact: true }).click();
    await page.getByRole('menuitem', { name: '大脑调度' }).click();
    await page.locator('.brain-run').first().getByText('已完成', { exact: true }).waitFor();
    await page.getByRole('button', { name: '查看详情' }).click();
    for (const op of view.operations) {
      const round = page.locator('.brain-run-details > .ant-collapse > .ant-collapse-item')
        .filter({ hasText: `第 ${op.round} 轮 ·` }).first();
      const header = round.locator('.ant-collapse-header').first();
      if (await header.getAttribute('aria-expanded') !== 'true') await header.click();
      await round.getByRole('button', { name: op.execution_id, exact: true }).click();
      const drawer = page.getByRole('dialog', { name: '能力执行明细', exact: true });
      await drawer.locator('.execution-view-full').waitFor();
      await drawer.getByTestId('rf__node-check').click();
      const logs = page.locator('.dag-logs-drawer');
      await logs.getByText('revision', { exact: false }).first().waitFor();
      await logs.getByText('已结束', { exact: true }).waitFor();
      const detail = fetched.get(op.execution_id);
      assert.equal(detail?.execution.kind, 'dag');
      const output = detail.result.scheduler_output.check;
      assert.equal(output.revision, op.round === 1 ? record.evidence.baseline_revision : record.evidence.repaired_revision);
      if (op.node_id === 'edge') assert.equal(output.passed, op.round === 2);
      await page.screenshot({ path: path.join(directory, `${op.round}-${op.node_id}.png`), animations: 'disabled' });
      checked.push(op.execution_id);
      await logs.getByRole('button', { name: '关闭', exact: true }).click();
      await drawer.getByRole('button', { name: '返回轮次列表' }).click();
    }
    for (const width of [390, 768, 1280, 1920]) {
      await page.setViewportSize({ width, height: 1100 });
      await page.screenshot({ path: path.join(directory, `history-${width}.png`), animations: 'disabled' });
    }
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(directory, 'result.json'), JSON.stringify({ result: 'PASS', id, checked }, null, 2));
  } catch (error) {
    await page.screenshot({ path: path.join(directory, 'failure.png') });
    fs.writeFileSync(path.join(directory, 'result.json'), JSON.stringify({ result: 'FAIL', error: error.message, checked, errors }, null, 2));
    throw error;
  } finally { await browser.close(); }
}
main().catch(error => { console.error(error.message); process.exitCode = 1; });
