const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const harness = require('../todo_workbench/harness');
const rounds = [];
async function answer(prompt) {
  let context;
  try { context = JSON.parse(prompt); } catch {}
  if (context?.schema_version === 3 && context.run_id) {
    rounds.push(context.round);
    if (context.round === 3) return {
      decision: 'complete', reason: 'Retest passed after the repair',
      evidence_execution_ids: context.operations.map(o => o.execution_id),
    };
    const previous = context.operations.find(o => o.round === context.round);
    return { decision: 'dispatch', capabilities: [{
      capability_id: context.round === 1 ? 'builtin-agent-act' : 'builtin-operator',
      inputs: { request: previous ? { kind: 'execution', execution_id: previous.execution_id, path: '' } : { kind: 'root', name: 'request' } },
    }], reason: ['Run test', 'Repair the reported issue', 'Retest the repaired result'][context.round],
    evidence_execution_ids: previous ? [previous.execution_id] : [] };
  }
  const input = JSON.parse(prompt.slice(prompt.lastIndexOf('Scheduler inputs:\n') + 'Scheduler inputs:\n'.length));
  if (prompt.includes('Capability: builtin-agent-act')) {
    assert.equal(input.request, 'needs repair');
    return 'repaired';
  }
  assert(['needs repair', 'repaired'].includes(input.request));
  return input.request === 'repaired' ? 'passed' : 'needs repair';
}
async function main() {
  const h = await harness.open(answer, { withBrowser: false });
  const id = `brain-repair-${Date.now()}`;
  await h.api('POST', '/api/brain/runs', { id, schema_version: 3, node_id: h.nodeId,
    objective: 'Test, repair the reported issue, retest, then complete with evidence.',
    inputs: { request: 'needs repair' }, capability_ids: ['builtin-agent-act', 'builtin-operator'], max_rounds: 3 });
  const snapshot = await h.until(async () => {
    const s = await h.api('GET', `/api/brain/runs/${id}`);
    return ['completed', 'failed', 'blocked', 'cancelled'].includes(s.run.phase) && s;
  }, 'repair and retest', 120000);
  assert.equal(snapshot.run.phase, 'completed', snapshot.run.error);
  assert.equal(snapshot.run.round, 3);
  assert.deepEqual(rounds, [0, 1, 2, 3]);
  const operations = snapshot.operations.sort((a, b) => a.round - b.round);
  assert.equal(new Set(operations.map(o => o.execution_id)).size, 3);
  assert.deepEqual(operations.map(o => [o.execution_kind, o.status]), [['operator', 'done'], ['agent', 'done'], ['operator', 'done']]);
  for (let i = 0; i < operations.length; i++) {
    const detail = await h.api('GET', `/api/executions/${operations[i].execution_id}`);
    assert.equal(detail.request.input.scheduler_inputs.request, ['needs repair', 'needs repair', 'repaired'][i]);
    assert.equal(detail.result.scheduler_output, ['needs repair', 'repaired', 'passed'][i]);
  }
  const events = await h.api('GET', `/api/brain/runs/${id}/events-page?limit=100`);
  assert.equal(events.events.filter(e => e.event_type === 'round_barrier_reached').length, 3);
  const receipt = { result: 'PASS', run_id: id, rounds, operations: operations.map(({ execution_id, execution_kind, round }) => ({ execution_id, execution_kind, round })), model_transport: 'scripted', evidence: h.root };
  fs.writeFileSync(path.join(h.root, 'receipt.json'), JSON.stringify(receipt, null, 2));
  console.log(JSON.stringify(receipt));
}
main().catch(error => { console.error(error); process.exitCode = 1; }).finally(() => harness.close());
