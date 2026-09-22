// Real-model acceptance against an authorized production/local deployment.
// Usage: node layered-production.js CONFIG EVIDENCE EXPECTED_COMMIT [RUN_ID]
// Creates isolated, named acceptance plans/capabilities; never deletes data.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { inspectPanels } = require('./layered-panels.js');
const [configPath, evidence, commit, existingRun] = process.argv.slice(2);
assert(configPath && evidence && /^[a-f0-9]{40}$/.test(commit || ''), 'CONFIG EVIDENCE EXPECTED_COMMIT required');
const settings = JSON.parse(fs.readFileSync(configPath)).deployment;
const token = fs.readFileSync(settings.token_file, 'utf8').trim();
fs.mkdirSync(evidence, { recursive: true, mode: 0o700 });
const save = (name, value) => fs.writeFileSync(path.join(evidence, `${name}.json`), JSON.stringify(value, null, 2), { mode: 0o600 });
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function api(method, route, body) {
  const response = await fetch(settings.public_url + route, { method,
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body), signal: AbortSignal.timeout(120000) });
  const value = await response.json();
  assert(response.ok, `${method} ${route}: ${response.status} ${JSON.stringify(value)}`);
  return value;
}
function assertRelease() {
  const state = JSON.parse(fs.readFileSync(path.join(settings.state_dir, 'release-state.json')));
  assert.equal(state.current, `rel-${commit}`); assert.equal(state.phase, 'complete');
}
async function prepare() {
  const tag = `layered-${Date.now()}`; const marker = `BRAIN-${tag}`;
  const instruction = `Acceptance only. Do not read or modify files, use tools or network. Reply with ${marker} and finish this task.`;
  await api('POST', '/api/dag/defs', { spec: { name: tag, description: 'Layered capability acceptance',
    steps: [{ name: 'analyze', kind: { type: 'agent', prompt: instruction } }] } });
  await api('POST', '/api/teams', { name: tag, captain: 'act', members: [{ agent: 'act' }, { agent: 'plan' }] });
  await api('POST', '/api/todo/templates', { name: tag, spec: { schema_version: 1, id: tag, name: tag,
    objective: instruction, todos: [{ id: 'echo', title: 'Verify acceptance marker',
      requirement_background: 'Layered capability integration acceptance', instructions: `${instruction} Put the marker in candidate.result.`,
      max_attempts: 1, acceptance: { criteria: `The result contains ${marker}.` } }] } });
  const node = (id, capability) => ({ node_id: id, title: `${id}: ${instruction}`, capability_id: capability, retry: { max_attempts: 2 } });
  const plan = (title, nodes, edges = []) => ({ schema_version: 4, title, objective: instruction, inputs: { request: marker }, nodes, edges, max_rounds: 8 });
  const child = `${tag}-child`;
  await api('POST', '/api/brain/plan-defs', { id: child, version: 1, created_at: Date.now(),
    changelog: 'Fixed child version acceptance', plan: plan('Acceptance child', [node('child', 'builtin-agent-act')]) });
  const nodes = [node('agent', 'builtin-agent-act'), node('dag', `dag-${tag}`), node('team', `team-${tag}`),
    node('operator', 'builtin-operator'), node('todos', `todos-${tag}-v1`), node('nested', `plan-${child}@1`)];
  const edges = ['team', 'operator', 'todos'].flatMap((to) => ['agent', 'dag'].map((from) => ({ from, to })));
  edges.push(...['team', 'operator', 'todos'].map((from) => ({ from, to: 'nested' })));
  const rootPlan = plan('Six capability layer acceptance', nodes, edges);
  const planId = `plan-${tag}`;
  await api('POST', '/api/brain/plan-defs', { id: planId, version: 1, created_at: Date.now(),
    changelog: 'Layer dispatch history and six detail components', plan: rootPlan });
  const ready = await api('GET', '/api/nodes');
  const host = ready.nodes.find((node) => node.online && node.snapshot?.generation?.startsWith('host-'));
  assert(host, 'online Host node required');
  const request = { id: `brain-${tag}`, node_id: host.id, schema_version: 4, plan: { id: planId, version: 1 }, inputs: { request: marker } };
  save('request', request); save('scenario', { marker, rootPlan });
  assert.equal((await api('POST', '/api/brain/runs', request)).run_id, request.id);
  return request.id;
}
function checkBarrier(view) {
  const events = view.events;
  for (let layer = 2; layer <= view.layers.length; layer++) {
    const started = events.find((event) => event.layer === layer && event.event_type === 'layer_started');
    const barrier = events.find((event) => event.layer === layer - 1 && event.event_type === 'layer_barrier_reached');
    assert(barrier && started && barrier.seq < started.seq, 'next layer crossed an incomplete barrier');
    for (const node of view.layers[layer - 2]) {
      assert(events.some((event) => event.node_id === node && event.event_type === 'operation_terminal'
        && event.decision_summary === 'done' && event.seq < barrier.seq), `missing successful terminal before barrier: ${node}`);
    }
  }
}
async function main() {
  assertRelease();
  const id = existingRun || await prepare(); console.log(JSON.stringify({ stage: 'created', id, evidence }));
  let view; const deadline = Date.now() + 1200000; const states = [];
  while (Date.now() < deadline) {
    view = await api('GET', `/api/brain/runs/${id}/layered`); save('view', view);
    const state = { phase: view.run.phase, layer: view.run.layer, operations: view.operations.map((op) => [op.node_id, op.attempt, op.status]) };
    if (JSON.stringify(state) !== JSON.stringify(states.at(-1)?.state)) {
      states.push({ at: Date.now(), state }); save('states', states); console.log(JSON.stringify(state));
    }
    if (['completed', 'failed', 'blocked', 'cancelled'].includes(view.run.phase)) break;
    await sleep(2000);
  }
  assert.equal(view.run.phase, 'completed', view.run.error || 'acceptance did not complete');
  const operations = view.plan.nodes.map((node) => view.operations.filter((op) => op.node_id === node.node_id).sort((a, b) => b.attempt - a.attempt)[0]);
  assert.deepEqual(operations.map((op) => op.execution_kind).sort(), ['agent', 'brain', 'dag', 'operator', 'team', 'todos']);
  assert(operations.every((op) => op.status === 'done')); checkBarrier(view);
  for (let layer = 1; layer <= view.layers.length; layer++) {
    const detail = await api('GET', `/api/brain/runs/${id}/layered/rounds/${layer}`); save(`round-${layer}`, detail);
    const event = view.events.find((row) => row.layer === layer && row.event_type === 'layer_started');
    assert.equal(detail.decision, 'dispatch_layer'); assert.equal(detail.reason, event.reason_summary);
    assert.deepEqual(detail.evidence_execution_ids, event.evidence_execution_ids);
  }
  const marker = view.plan.inputs.request;
  for (const op of operations) {
    const detail = await api('GET', `/api/executions/${op.execution_id}`); save(`detail-${op.execution_kind}`, detail);
    assert.equal(detail.execution.id, op.execution_id); assert.equal(detail.execution.kind, op.execution_kind); assert.equal(detail.execution.status, 'done');
    if (op.execution_kind === 'brain') {
      const nested = await api('GET', `/api/brain/runs/${op.execution_id}/layered`); save('nested', nested);
      assert.equal(nested.run.phase, 'completed'); assert.equal(nested.run.parent.run_id, id);
    } else assert(JSON.stringify(detail.result.scheduler_output).includes(marker), `${op.execution_kind} output missing marker`);
  }
  const panels = await inspectPanels({ base: settings.public_url, token, id, view, operations, marker, evidence });
  assertRelease();
  const receipt = { result: 'PASS', commit, run_id: id, evidence, operations, panels };
  save('result', receipt); console.log(JSON.stringify(receipt));
}
main().catch((error) => { save('failure', { error: error.stack }); console.error(error); process.exitCode = 1; });
