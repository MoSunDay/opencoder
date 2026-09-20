// Real Server → Worker → Chromium acceptance for dynamic Agent instances.
const assert = require('assert/strict');
const fs = require('fs');
const path = require('path');
const harness = require('./todo_workbench/harness');
let h;
let release;
const held = new Promise((resolve) => { release = resolve; });
async function main() {
  h = await harness.open(async (prompt, request) => {
    if (prompt.includes('DISCOVER_BATCH')) return { items: ['INSTANCE_ZERO', 'INSTANCE_ONE'] };
    const full = JSON.stringify((request.messages || []).filter((m) => m.role === "system"));
    if (full.includes('INSTANCE_ONE') && !prompt.includes('DISCOVER_BATCH')) await held;
    return { instance: full.includes('INSTANCE_ZERO') ? 'zero-result' : 'one-result' };
  }, { dag: true });
  const { page, api, root, until } = h;
  console.log(JSON.stringify({ root }));
  const id = 'dag-dynamic-browser';
  await api('POST', '/api/dag/defs', { spec: { name: 'dynamic-browser', steps: [
    { name: 'discover', kind: { type: 'agent', prompt: 'DISCOVER_BATCH' } },
    { name: 'process', depends_on: ['discover'], kind: { type: 'dynamic', source: { type: 'step_output', step: 'discover', pointer: '/items' }, template: { type: 'agent', prompt: 'Read local how instructions and return a JSON result.' } } },
  ] } });
  await api('POST', '/api/dag/defs/dynamic-browser/dispatch', { id, node_id: h.nodeId });
  await until(async () => (await api('GET', `/api/dag/runs/${id}/steps/process/instances`)).progress?.done === 1, 'first instance complete');
  const requests = [];
  page.on('request', (r) => { if (r.url().includes('/instances/') && r.url().includes('/events')) requests.push(r.url()); });
  await page.getByText('Agent', { exact: true }).first().click();
  await page.getByRole('menuitem', { name: 'DAG 工作流' }).click();
  await page.getByRole('tab', { name: '运行', exact: true }).click();
  await page.getByRole('button', { name: '查看', exact: true }).first().click();
  await page.locator('[data-id="process"]').getByText('1/2 成功').waitFor();
  assert.equal(await page.locator('[data-id="process"]').count(), 1, 'one logical node on canvas');
  await page.locator('[data-id="process"] .dag-node').click();
  const drawer = page.locator('.dag-logs-drawer');
  await drawer.getByText('"INSTANCE_ZERO"', { exact: true }).waitFor();
  await drawer.locator('.execution-logs').getByText(/zero-result/).first().waitFor();
  let disconnected = false;
  await page.route(`**/api/dag/runs/${id}/steps/process/instances/1/events*`, async (route) => {
    if (!disconnected) { disconnected = true; await route.abort('connectionreset'); }
    else await route.continue();
  });
  await drawer.getByRole('combobox', { name: '选择实例' }).click();
  await page.locator('.ant-select-item-option-content').filter({hasText:'实例 1 · running'}).click();
  await drawer.getByText('"INSTANCE_ONE"', { exact: true }).waitFor();
  await until(async () => !(await drawer.innerText()).includes('zero-result'), 'previous instance logs cleared');
  await until(() => requests.filter((url) => url.includes('/instances/1/events')).length >= 2, 'instance stream reconnect');
  release();
  await until(async () => (await api('GET', `/api/dag/runs/${id}/progress`)).done === 2, 'run complete');
  await drawer.locator('.execution-logs').getByText(/one-result/).first().waitFor();
  await drawer.getByText('2/2 成功', { exact: true }).waitFor();
  await until(async () => !(await drawer.innerText()).includes('运行中'), 'terminal instance and template receipts');
  await page.screenshot({ path: path.join(root, 'dynamic-instance.png'), animations: 'disabled' });
  const detail = await api('GET', `/api/dag/runs/${id}/steps/process/instances/1`);
  assert.deepEqual(detail.input, 'INSTANCE_ONE');
  assert.equal(detail.status, 'done');
  assert.equal(detail.output.instance, 'one-result');
  await page.reload({ waitUntil: 'networkidle' });
  await page.getByRole('menuitem', { name: 'DAG 工作流' }).click();
  await page.getByRole('tab', { name: '运行', exact: true }).click();
  await page.getByRole('button', { name: '查看', exact: true }).first().click();
  await page.locator('[data-id="process"] .dag-node').waitFor();
  await page.locator('[data-id="process"] .dag-node').click();
  await page.locator('.dag-logs-drawer').locator('.execution-logs').getByText(/zero-result/).first().waitFor();
  await api('POST', '/api/dag/defs', { spec: { name: 'dynamic-input-browser', steps: [
    { name: 'batch', kind: { type: 'dynamic', source: { type: 'input', pointer: '/items' }, template: { type: 'agent', prompt: 'Read the local how instructions.' } } },
  ] } });
  await page.reload({ waitUntil: 'networkidle' });
  await page.getByRole('menuitem', { name: 'DAG 工作流' }).click();
  await page.getByRole('row').filter({ hasText: 'dynamic-input-browser' }).getByRole('button', { name: '派发', exact: true }).click();
  await page.getByRole('textbox', { name: 'batch 批次' }).fill('["INSTANCE_ZERO"]');
  const dispatched = page.waitForResponse((response) => response.url().endsWith('/dynamic-input-browser/dispatch') && response.request().method() === 'POST');
  await page.getByRole('button', { name: '确认派发', exact: true }).click();
  const dispatchReply = await dispatched;
  assert.equal(dispatchReply.status(), 202);
  const inputRun = (await dispatchReply.json()).run_id;
  await until(async () => (await api('GET', `/api/dag/runs/${inputRun}/steps/batch/instances`)).progress?.done === 1, 'UI text batch dispatch');
  assert.equal((await api('GET', `/api/dag/runs/${inputRun}/steps/batch/instances/0`)).input, 'INSTANCE_ZERO');
  assert.deepEqual(h.errors, []);
  assert(requests.some((url) => url.includes('/instances/0/events')));
  assert(requests.some((url) => url.includes('/instances/1/events')));
  fs.writeFileSync(path.join(root, 'dynamic-result.json'), JSON.stringify({ result: 'PASS', requests, detail }, null, 2));
  console.log(JSON.stringify({ result: 'PASS', root }));
}
main().catch(async (error) => { console.error(error); if (h) { await h.page.screenshot({path:path.join(h.root,'failure.png')}); fs.writeFileSync(path.join(h.root,'failure.txt'),await h.page.locator('body').innerText()); } process.exitCode = 1; }).finally(async () => { release(); await harness.close(); });
