// Real Server/Worker/browser, scripted model only. Run with PLATFORM_BIN_DIR
// pointing at the reviewed bundle and CHROME_PATH pointing at Chromium.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const harness = require('../todo_workbench/harness');
const { prepare, collect } = require('./scenario');
const launch = require('./launch');
let h, request, finishAnalysis;
const analysis = new Promise((resolve) => { finishAnalysis = resolve; });
const planning = [];

async function answer(prompt) {
  let context;
  try { context = JSON.parse(prompt); } catch {}
  if (context?.schema_version === 3 && context?.run_id) {
    planning.push(context.round);
    const ids = context.request.capability_ids;
    if (context.round < 2) {
      const capabilities = (context.round === 0 ? ids.slice(0, 1) : ids.slice(1)).map((capability_id) => ({
        capability_id, inputs: Object.fromEntries(Object.keys(context.request.inputs).map((name) => [name,
          context.round === 1 && name === 'request'
            ? { kind: 'execution', execution_id: context.operations[0].execution_id, path: '/analyze/echo' }
            : { kind: 'root', name },
        ])),
      }));
      return { decision: 'dispatch', capabilities, reason: 'Verify references across real executors', evidence_execution_ids: context.operations.map((o) => o.execution_id) };
    }
    assert(context.operations.every((o) => o.status === 'done'));
    return { decision: 'complete', reason: 'All five executions supplied matching input evidence', evidence_execution_ids: context.operations.map((o) => o.execution_id) };
  }
  const echo = request.inputs.request;
  // TODO/Team nest the bound JSON inside another prompt JSON string. The
  // unique marker proves delivery here; collect() checks every value exactly.
  assert(prompt.includes(echo.split('-参数')[0]), 'executor did not receive bound input');
  if (prompt.startsWith('Decide the next workflow operation.')) {
    return prompt.includes('RUNNABLE=[]') ? { operation: 'complete', reason: 'echo accepted' }
      : { operation: 'dispatch', todos: [{ todo_id: 'echo', context_mode: 'new' }], reason: 'verify input' };
  }
  if (prompt.startsWith('Accept or reject one TODO candidate.')) return { operation: 'accept', reason: 'matching input', mark_milestone: false };
  if (prompt.startsWith('Complete exactly one focused TODO.')) return { status: 'candidate', summary: echo, result: echo, verification: 'matching input', evidence_refs: [], recovery_context: { summary: echo, refs: [] } };
  if (prompt.startsWith('你是团队队长。请根据')) return { question: 'echo the input', participants: ['plan'], rationale: 'input verification' };
  if (prompt.startsWith('你是团队队长。请汇总')) return { summary: echo, aligned: true, ambiguities: [] };
  if (prompt.startsWith('你是团队队长。请基于')) return { complete: true, final_summary: echo };
  if (prompt.includes('ANALYZE_INPUT_ECHO')) { await analysis; return { echo }; }
  return echo;
}

async function main() {
  h = await harness.open(answer, { dag: true });
  const { api, page, until } = h;
  request = await prepare(api, `closure-${Date.now()}`, h.nodeId);
  await launch(page, api, request);
  const waiting = await until(async () => {
    const state = await api('GET', `/api/brain/runs/${request.id}`);
    return (state.operations?.[0]?.status === 'running' || ['blocked', 'failed', 'cancelled'].includes(state.run.phase)) && state;
  }, 'DAG running', 120000);
  assert.equal(waiting.operations?.[0]?.status, 'running', waiting.run.error || 'DAG was not dispatched');
  console.log(JSON.stringify({ stage: 'dag_running', run_id: request.id }));
  assert.deepEqual(planning, [0]);
  await page.goto(`${new URL(page.url()).origin}/?brain_run=${request.id}`, { waitUntil: 'domcontentloaded', timeout: 60000 });
  await page.getByRole('radiogroup').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '大脑调度' }).click();
  await page.getByLabel('大脑调度总览画布').waitFor();
  await page.getByRole('button', { name: `查看执行 ${waiting.operations[0].execution_id}` }).click();
  const dagDetail = await api('GET', `/api/executions/${waiting.operations[0].execution_id}`);
  fs.writeFileSync(path.join(h.root, 'dag-detail.json'), JSON.stringify(dagDetail, null, 2));
  const step = page.getByTestId('rf__node-analyze');
  await step.waitFor();
  await step.click();
  await page.locator('.dag-logs-drawer').waitFor();
  assert.deepEqual(planning, [0], 'brain must wait for terminal DAG event');
  await api('POST', `/api/brain/runs/${request.id}/commands`, { action: 'pause' });
  assert.equal((await api('GET', `/api/brain/runs/${request.id}`)).run.phase, 'paused');
  finishAnalysis();
  await page.locator('.dag-logs-drawer').getByText(request.inputs.request, { exact: false }).first().waitFor();
  await page.screenshot({ path: path.join(h.root, 'brain-dag-step.png'), animations: 'disabled' });
  await page.locator('.dag-logs-drawer').getByRole('button', { name: '关闭', exact: true }).click();
  await page.locator('.ant-drawer:visible .ant-drawer-close').last().click();
  await until(async () => (await api('GET', `/api/brain/runs/${request.id}`)).operations[0].status === 'done', 'paused DAG terminal');
  assert.deepEqual(planning, [0], 'pause must prevent the next activation');
  await h.restart();
  console.log(JSON.stringify({ stage: 'node_restarted', run_id: request.id }));
  const recovered = await api('GET', `/api/brain/runs/${request.id}`);
  assert.equal(recovered.run.phase, 'paused');
  assert.equal(recovered.operations.length, 1);
  assert.equal(recovered.operations[0].execution_id, waiting.operations[0].execution_id);
  await api('POST', `/api/brain/runs/${request.id}/commands`, { action: 'resume' });
  const receipt = await collect(api, request, until);
  assert.deepEqual(planning, [0, 1, 2]);
  await page.locator('.brain-run-header').getByText('已完成', { exact: true }).waitFor();
  const overview = page.getByLabel('大脑调度总览画布');
  assert((await overview.boundingBox()).height < 260, 'long goals must not stretch the overview');
  await overview.scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(h.root, 'brain-overview.png'), animations: 'disabled' });
  for (const operation of receipt.operations.filter((o) => o.round === 2)) {
    await page.getByRole('button', { name: `查看执行 ${operation.execution_id}` }).click();
    await page.getByText('所属节点', { exact: true }).waitFor();
    if (operation.execution_kind === 'todos') await page.locator('.todo-workbench').waitFor();
    else if (operation.execution_kind === 'team') await page.locator('.execution-team-turn').first().waitFor();
    else await page.getByText('会话消息', { exact: true }).waitFor();
    assert.equal(await page.getByRole('button', { name: '在原节点恢复' }).count(), 0);
    await page.screenshot({ path: path.join(h.root, `brain-${operation.execution_kind}.png`), animations: 'disabled' });
    await page.locator('.ant-drawer:visible .ant-drawer-close').last().click();
  }
  assert.deepEqual(h.errors, []);
  fs.writeFileSync(path.join(h.root, 'receipt.json'), JSON.stringify(receipt, null, 2));
  console.log(JSON.stringify({ ...receipt, evidence: h.root }));
}
main().catch(async (error) => {
  console.error(error); process.exitCode = 1;
  if (h) {
    console.error('evidence:', h.root);
    console.error('browser errors:', h.errors);
    await h.page.screenshot({ path: path.join(h.root, 'failure.png') }).catch(() => {});
    fs.writeFileSync(path.join(h.root, 'failure.html'), await h.page.content().catch(() => ''));
  }
})
  .finally(async () => { finishAnalysis(); await harness.close(); });
