// Real Brain → dynamic DAG → shared execution panel. Only LLM replies are scripted.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const harness = require('../todo_workbench/harness');
let h, release;
const held = new Promise((resolve) => { release = resolve; });
const rounds = [];
const inputs = { request: 'Root input 参数 "quoted"', items: ['INSTANCE_ZERO', 'INSTANCE_ONE'] };
async function answer(prompt, request) {
  let context;
  try { context = JSON.parse(prompt); } catch {}
  if (context?.schema_version === 3 && context.run_id) {
    rounds.push(context.round);
    if (context.round === 0) return {
      decision: 'dispatch', reason: 'Process the named root batch', evidence_execution_ids: [],
      capabilities: [{ capability_id: 'dag-brain-dynamic', inputs: {
        request: { kind: 'root', name: 'request' }, items: { kind: 'root', name: 'items' },
      } }],
    };
    return { decision: 'complete', reason: 'Every dynamic instance succeeded',
      evidence_execution_ids: context.operations.map((o) => o.execution_id) };
  }
  const text = JSON.stringify(request.messages || []);
  const marker = 'Scheduler inputs:\n';
  assert(prompt.includes(marker), 'dynamic template omitted scheduler inputs');
  assert.deepEqual(JSON.parse(prompt.slice(prompt.lastIndexOf(marker) + marker.length).split('\n')[0]), inputs);
  const one = text.includes('INSTANCE_ONE') && JSON.stringify((request.messages || []).filter((m) => m.role === 'system')).includes('INSTANCE_ONE');
  if (one) await held;
  return { instance: one ? 'one-result' : 'zero-result' };
}
async function main() {
  h = await harness.open(answer, { dag: true });
  const { api, page, until } = h;
  await api('POST', '/api/dag/defs', { spec: { name: 'brain-dynamic', description: 'Named batch input', steps: [
    { name: 'batch', kind: { type: 'dynamic', source: { type: 'input', pointer: '/items' }, template: { type: 'agent', prompt: 'Read the local instance instructions and return its result.' } } },
  ] } });
  const id = `brain-dynamic-${Date.now()}`;
  await api('POST', '/api/brain/runs', { id, schema_version: 3, node_id: h.nodeId,
    objective: 'Process two instances, then complete with their execution evidence.', inputs,
    capability_ids: ['dag-brain-dynamic'], max_rounds: 1 });
  const operation = await until(async () => {
    const snapshot = await api('GET', `/api/brain/runs/${id}`);
    assert(!['blocked', 'failed'].includes(snapshot.run.phase), snapshot.run.error);
    return snapshot.operations[0];
  }, 'dynamic dispatch');
  const execution = operation.execution_id;
  await until(async () => (await api('GET', `/api/dag/runs/${execution}/steps/batch/instances`)).progress?.done === 1, 'first instance');
  assert.deepEqual(rounds, [0]);
  await page.goto(`${new URL(page.url()).origin}/?brain_run=${id}`, { waitUntil: 'domcontentloaded' });
  await page.getByRole('radiogroup').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '大脑调度' }).click();
  await page.getByLabel('大脑调度总览画布').waitFor();
  assert.equal(await page.locator('.brain-summary-node').count(), 4);
  await page.getByRole('button', { name: `查看执行 ${execution}` }).click();
  await page.locator('[data-id="batch"]').getByText('1/2 成功').waitFor();
  await page.locator('[data-id="batch"] .dag-node').click();
  const drawer = page.locator('.dag-logs-drawer');
  await drawer.locator('.execution-logs').getByText(/zero-result/).first().waitFor();
  await drawer.getByRole('combobox', { name: '选择实例' }).click();
  await page.locator('.ant-select-item-option-content').filter({ hasText: '实例 1 · running' }).click();
  await drawer.getByText('"INSTANCE_ONE"', { exact: true }).waitFor();
  await until(async () => !(await drawer.innerText()).includes('zero-result'), 'previous logs cleared');
  assert.deepEqual(rounds, [0], 'partial group must not wake the Brain');
  release();
  await drawer.locator('.execution-logs').getByText(/one-result/).first().waitFor();
  const done = await until(async () => {
    const s = await api('GET', `/api/brain/runs/${id}`);
    return ['completed', 'failed', 'blocked'].includes(s.run.phase) && s;
  }, 'dynamic barrier');
  assert.equal(done.run.phase, 'completed', done.run.error);
  assert.deepEqual(rounds, [0, 1]);
  const detail = await api('GET', `/api/executions/${execution}`);
  assert.deepEqual(detail.request.input.scheduler_inputs, inputs);
  assert.deepEqual(detail.result.scheduler_output.batch, [{ instance: 'zero-result' }, { instance: 'one-result' }]);
  await page.screenshot({ path: path.join(h.root, 'brain-dynamic.png'), animations: 'disabled' });
  assert.deepEqual(h.errors, []);
  const receipt = { result: 'PASS', run_id: id, execution_id: execution, rounds, evidence: h.root };
  fs.writeFileSync(path.join(h.root, 'receipt.json'), JSON.stringify(receipt, null, 2));
  console.log(JSON.stringify(receipt));
}
main().catch(async (error) => {
  console.error(error);
  if (h) await h.page.screenshot({ path: path.join(h.root, 'failure.png') }).catch(() => {});
  process.exitCode = 1;
}).finally(async () => { release(); await harness.close(); });
