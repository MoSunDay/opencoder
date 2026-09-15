// Real Server/Agent + browser acceptance, with deterministic agent responses.
const assert = require('assert/strict');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const harness = require('./todo_workbench/harness');
let h;
let finish;
const held = new Promise((resolve) => { finish = resolve; });

async function main() {
  h = await harness.open(async (prompt) => {
    if (prompt.includes('HOLD_REVIEW')) await held;
    return { result: prompt.includes('HOLD_REVIEW') ? 'review-log-result' : 'fetch-log-result' };
  }, { dag: true });
  const { page, api, root, until } = h;
  console.log(JSON.stringify({ root }));
  const served = await fetch(new URL('/static/app.js', page.url())).then((r) => r.arrayBuffer());
  const digest = (bytes) => crypto.createHash('sha256').update(Buffer.from(bytes)).digest('hex');
  assert.equal(digest(served), digest(fs.readFileSync(path.join(__dirname, '../../crates/web/spa/dist/static/app.js'))), 'browser must use the current SPA');
  const requests = [];
  page.on('request', (request) => { if (request.url().includes('/events')) requests.push(request.url()); });
  await api('POST', '/api/dag/defs', { spec: { name: 'dag-results-ui', steps: [
    { name: 'fetch', kind: { type: 'agent', prompt: 'Return a JSON result for FETCH_DATA.' } },
    { name: 'review', depends_on: ['fetch'], kind: { type: 'agent', prompt: 'Return a JSON result for HOLD_REVIEW.' } },
  ] } });
  const id = 'dag-results-browser';
  await api('POST', '/api/dag/defs/dag-results-ui/dispatch', { id, node_id: h.nodeId });
  await until(async () => (await api('GET', `/api/dag/runs/${id}/progress`)).running === 1, 'running step');
  await page.getByText('Agent', { exact: true }).first().click();
  await page.getByRole('menuitem', { name: 'DAG 工作流' }).click();
  await page.getByRole('tab', { name: '运行', exact: true }).click();
  await page.getByRole('button', { name: '查看', exact: true }).first().click();
  await until(async () => await page.locator('.dag-node--running').count() === 1, 'current running graph');
  assert.equal(await page.locator('.dag-detail-side').count(), 0);
  assert.equal(await page.getByRole('log').count(), 0);
  assert(!requests.some((url) => /[?&]after=0(?:&|$)/.test(url)), 'graph must not replay history');
  await page.screenshot({ path: path.join(root, 'dag-current-result.png'), animations: 'disabled', timeout: 60000 });
  await page.locator('[data-id="fetch"] .dag-node').click();
  const drawer = page.locator('.dag-logs-drawer');
  await drawer.getByRole('log').getByText(/fetch-log-result/).first().waitFor();
  let box;
  await until(async () => {
    box = await drawer.locator('.ant-drawer-content-wrapper').boundingBox();
    return box && Math.abs(box.width - 1200) < 2 && Math.abs(box.x - 400) < 2;
  }, 'right drawer settles at 75% width');
  await drawer.getByRole('combobox', { name: '切换步骤日志' }).click();
  await page.locator('.ant-select-item-option-content').getByText('review', { exact: true }).click();
  assert(!(await drawer.getByRole('log').innerText()).includes('fetch-log-result'));
  finish();
  await until(async () => (await api('GET', `/api/dag/runs/${id}/progress`)).done === 2, 'finished DAG');
  await drawer.getByRole('log').getByText(/review-log-result/).first().waitFor();
  await page.screenshot({ path: path.join(root, 'dag-step-logs.png'), animations: 'disabled', timeout: 60000 });
  await drawer.locator('.ant-drawer-close').click();
  await until(async () => await page.locator('.dag-node--done').count() === 2, 'final graph');
  await page.getByRole('button', { name: '执行详情与产物' }).click();
  const detail = page.locator('.oc-execution-detail');
  await detail.locator('[data-id="fetch"] .dag-node').click();
  await page.locator('.dag-logs-drawer').getByRole('log').getByText(/fetch-log-result/).first().waitFor();
  await page.screenshot({ path: path.join(root, 'dag-execution-detail.png'), animations: 'disabled', timeout: 60000 });
  assert.deepEqual(h.errors, []);
  const result = { result: 'PASS', id, cases: ['snapshot-first', 'live-state', 'right-75-percent', 'step-switch', 'live-logs', 'execution-detail'], requests };
  fs.writeFileSync(path.join(root, 'dag-results.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify({ result: 'PASS', root }));
}
main().catch((error) => { console.error(error); process.exitCode = 1; }).finally(async () => { finish(); await harness.close(); });
