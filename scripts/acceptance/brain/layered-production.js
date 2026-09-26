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
  const node = (id, capability, layer = 1) => ({ node_id: id, layer_id: `layer-${layer}`, objective: instruction,
    title: `${id}: return ${marker} exactly; no tools, files or network.`, capability_id: capability });
  const plan = (title, nodes) => {
    const count = Math.max(...nodes.map((item) => Number(item.layer_id.slice(6))));
    const layers = Array.from({ length: count }, (_, index) => ({ layer_id: `layer-${index + 1}`, title: `Milestone ${index + 1}`,
      objective: instruction, success_criteria: `The output includes ${marker}.` }));
    const transitions = layers.slice(1).map((layer, index) => ({ from: layers[index].layer_id, to: layer.layer_id,
      condition: 'The prior milestone is met.' }));
    return { schema_version: 7, title, objective: instruction, inputs: { request: marker }, layers, nodes, transitions, max_rounds: 5 };
  };
  const child = `${tag}-child`;
  await api('POST', '/api/brain/plan-defs', { id: child, version: 1, created_at: Date.now(),
    changelog: 'Fixed child version acceptance', plan: plan('Acceptance child', [node('child', 'builtin-agent-act')]) });
  const nodes = [node('agent', 'builtin-agent-act'), node('dag', `dag-${tag}`), node('team', `team-${tag}`, 2),
    node('operator', 'builtin-operator', 2), node('todos', `todos-${tag}-v1`, 2), node('nested', `plan-${child}@1`, 3)];
  const rootPlan = plan('Six capability milestone reflection acceptance', nodes);
  rootPlan.layers[2].success_criteria = `The child output includes ${marker} AND this ROOT run is in round 2. In round 1 assess this milestone as not met and reflect back to layer 1.`;
  rootPlan.transitions.push({ from: 'layer-3', to: 'layer-1', condition: 'Round 1 needs another pass through the first milestone.' });
  rootPlan.objective += ' Exercise exactly two rounds: dispatch every layer in round 1, then reflect from nested to agent (layer 1), re-execute all layers in round 2, then complete. Every dispatch binds the acceptance marker; final summary must include it. No external actions.';
  const planId = `plan-${tag}`;
  await api('POST', '/api/brain/plan-defs', { id: planId, version: 1, created_at: Date.now(),
    changelog: 'Layer dispatch history and six detail components', plan: rootPlan });
  const ready = await api('GET', '/api/nodes');
  const host = ready.nodes.find((node) => node.online && node.snapshot?.generation?.startsWith('host-'));
  assert(host, 'online Host node required');
  const request = { id: `brain-${tag}`, node_id: host.id, schema_version: 7, plan: { id: planId, version: 1 }, inputs: { request: marker } };
  save('request', request); save('scenario', { marker, rootPlan });
  assert.equal((await api('POST', '/api/brain/runs', request)).run_id, request.id);
  return request.id;
}
function checkBarrier(view) {
  const visits = view.events.filter((event) => event.event_type === 'layer_started');
  assert.equal(view.run.round, 2);
  assert.equal(visits.length, 6);
  assert.equal(new Set(view.operations.map((op) => op.execution_id)).size, view.operations.length);
  assert.equal(view.operations.length, 12);
  for (let i = 1; i < visits.length; i++) {
    const previous = visits[i - 1]; const started = visits[i];
    const barrier = view.events.find((event) => event.activation === previous.activation && event.event_type === 'layer_barrier_reached');
    assert(barrier && barrier.seq < started.seq, 'next visit crossed an incomplete barrier');
    for (const op of view.operations.filter((op) => op.activation === previous.activation)) {
      assert(view.events.some((event) => event.execution_id === op.execution_id && event.event_type === 'operation_terminal'
        && event.seq < barrier.seq), `missing terminal before barrier: ${op.execution_id}`);
    }
  }
  assert(visits[3].reflection && visits[3].decision_summary === 'reflect_and_return');
}
async function main() {
  assertRelease();
  const id = existingRun || await prepare(); console.log(JSON.stringify({ stage: 'created', id, evidence }));
  // Two full six-capability rounds include two Team discussions and nested plans.
  let view; const deadline = Date.now() + 2400000; const states = [];
  while (Date.now() < deadline) {
    view = await api('GET', `/api/brain/runs/${id}/layered`); save('view', view);
    const state = { phase: view.run.phase, round: view.run.round, layer: view.run.layer, operations: view.operations.map((op) => [op.node_id, op.attempt, op.status]) };
    if (JSON.stringify(state) !== JSON.stringify(states.at(-1)?.state)) {
      states.push({ at: Date.now(), state }); save('states', states); console.log(JSON.stringify(state));
    }
    if (['completed', 'failed', 'blocked', 'cancelled'].includes(view.run.phase)) break;
    await sleep(2000);
  }
  assert.equal(view.run.phase, 'completed', view.run.error || 'acceptance did not complete');
  const operations = view.plan.nodes.map((node) => view.operations.filter((op) => op.node_id === node.node_id).sort((a, b) => b.activation - a.activation)[0]);
  assert.deepEqual(operations.map((op) => op.execution_kind).sort(), ['agent', 'brain', 'dag', 'operator', 'team', 'todos']);
  assert(operations.every((op) => op.status === 'done')); checkBarrier(view);
  for (const event of view.events.filter((row) => row.event_type === 'layer_started')) {
    const detail = await api('GET', `/api/brain/runs/${id}/layered/rounds/${event.layer}?activation=${event.activation}`);
    save(`visit-${event.activation}`, detail);
    assert.deepEqual(detail.visit, event);
    assert.equal(detail.nodes.flatMap((node) => node.operations).length, event.assignments.length);
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
