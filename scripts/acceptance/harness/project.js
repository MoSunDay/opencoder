// Real Codex Project Plan -> Execute -> immutable delivery, through Server/Node.
const assert = require('assert/strict');
const fs = require('fs');
const path = require('path');

module.exports = async function project({ api, until, nodeDir, root }) {
  await api('PUT', '/api/agents/plan', { harness: 'codex' });
  await api('PUT', '/api/agents/act', { harness: 'codex' });
  const todo = await api('POST', '/api/project/todos', {
    title: 'Codex binary project delivery', agent: 'act',
    draft: 'Create only codex-project-delivery.txt in the working directory with the exact text WRAP_PROJECT_ARTIFACT. Register that file using the supplied deliverable manifest. Do not edit any other existing file, contact anyone, or perform unrelated work.',
  });
  const id = `project-${todo.id}`;
  let detail;
  for (const action of ['plan', 'execute']) {
    const runId = `prun-browser-${action}`;
    await api('POST', `/api/project/todos/${todo.id}/${action}`, { run_id: runId });
    await until(async () => {
      detail = await api('GET', `/api/executions/${id}`);
      assert(!['error', 'cancelled'].includes(detail.execution.status), JSON.stringify(detail));
      return detail.execution.status === 'idle' && detail.result.run.id === runId;
    }, `real Project ${action}`, 600000);
    if (action === 'plan') {
      const input = JSON.parse(detail.result.run.input_snapshot);
      assert(!input.prompt.includes('OPENCODER_DELIVERABLE_MANIFEST='), 'Plan must not pin a delivery path');
    }
  }
  const trace = JSON.parse(detail.result.run.trace_manifest);
  assert.equal(trace.harness, 'codex');
  assert(trace.thread_id);
  assert.equal(trace.artifacts.length, 1);
  assert.equal(trace.artifacts[0].name, 'codex-project-delivery.txt');
  const saved = path.join(nodeDir, 'state', 'project-runs', 'prun-browser-execute', trace.artifacts[0].file);
  assert.equal(fs.readFileSync(saved, 'utf8').trim(), 'WRAP_PROJECT_ARTIFACT');
  // Changing the working copy must not alter the immutable run's delivery.
  fs.writeFileSync(path.join(nodeDir, 'codex-project-delivery.txt'), 'changed after delivery');
  assert.equal(fs.readFileSync(saved, 'utf8').trim(), 'WRAP_PROJECT_ARTIFACT');
  fs.writeFileSync(path.join(root, 'project-acceptance.json'), JSON.stringify({
    result: 'PASS', execution_id: id, run_id: detail.result.run.id, harness: trace.harness,
    thread_id: trace.thread_id, artifact: trace.artifacts[0],
  }, null, 2));
};
