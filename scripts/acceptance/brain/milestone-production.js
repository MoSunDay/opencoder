// Production acceptance for capability nodes and milestone transitions.
// Usage: node milestone-production.js CONFIG EVIDENCE EXPECTED_COMMIT
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const { inspectPanels } = require('./layered-panels.js');

const [configPath, evidence, commit] = process.argv.slice(2);
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
  assert.equal(state.current, `rel-${commit}`);
  assert.equal(state.phase, 'complete');
}

async function prepare() {
  const tag = `milestone-${Date.now()}`;
  const marker = `MILESTONE-${tag}`;
  const instruction = `Acceptance only. Do not read or modify files, use tools or network. Reply with ${marker} and finish.`;
  await api('POST', '/api/dag/defs', { spec: { name: tag, description: 'Milestone transition acceptance',
    steps: [{ name: 'analyze', kind: { type: 'agent', prompt: instruction } }] } });
  const node = (id, capability, layer) => ({ node_id: id, layer_id: `layer-${layer}`, capability_id: capability,
    title: `${id}: return ${marker} exactly; no tools, files or network.`, objective: instruction });
  const plan = { schema_version: 7, title: 'Parallel capability and milestone return acceptance',
    objective: `${instruction} Dispatch both layer-1 nodes in round 1, then layer 2. Assess layer 2 as not met in round 1, return to layer 1, dispatch both layers again in round 2, then complete. Every output and final summary must contain ${marker}.`,
    inputs: { request: marker }, max_rounds: 5,
    layers: [
      { layer_id: 'layer-1', title: 'Collect evidence', task: 'Collect two capability outputs in parallel',
        objective: instruction, success_criteria: `Both outputs contain ${marker}.` },
      { layer_id: 'layer-2', title: 'Verify and return', task: 'Verify, return once, then finish',
        objective: instruction, success_criteria: `The output contains ${marker} AND this ROOT run is in round 2. In round 1 assess this milestone as not met and return to layer 1.` },
    ],
    nodes: [node('agent', 'builtin-agent-act', 1), node('dag', `dag-${tag}`, 1), node('operator', 'builtin-operator', 2)],
    transitions: [
      { from: 'layer-1', to: 'layer-2', condition: 'Both parallel outputs are complete.' },
      { from: 'layer-2', to: 'layer-1', condition: 'First round needs another pass.' },
    ], edges: [] };
  assert.deepEqual(await api('POST', '/api/brain/plan-defs/validate', plan), { valid: true });
  const missingTask = structuredClone(plan); missingTask.layers[0].task = '';
  const invalid = await fetch(settings.public_url + '/api/brain/plan-defs/validate', { method: 'POST',
    headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
    body: JSON.stringify(missingTask), signal: AbortSignal.timeout(120000) });
  assert.equal(invalid.status, 400);
  assert.match((await invalid.json()).error, /milestone task required/);
  const planId = `plan-${tag}`;
  await api('POST', '/api/brain/plan-defs', { id: planId, version: 1, created_at: Date.now(),
    changelog: 'Production milestone transition acceptance', plan });
  const ready = await api('GET', '/api/nodes');
  const host = ready.nodes.find((item) => item.online && item.snapshot?.ready && item.snapshot.generation?.startsWith('host-'));
  assert(host, 'ready Host node required');
  const request = { id: `brain-${tag}`, node_id: host.id, schema_version: 7,
    plan: { id: planId, version: 1 }, inputs: { request: marker } };
  save('scenario', { marker, plan, request });
  assert.equal((await api('POST', '/api/brain/runs', request)).run_id, request.id);
  return request.id;
}

function assertBarrier(view) {
  const visits = view.events.filter((event) => event.event_type === 'layer_started');
  assert.equal(view.run.round, 2);
  assert.deepEqual(visits.map((event) => [event.round, event.layer]), [[1, 1], [1, 2], [2, 1], [2, 2]]);
  assert.equal(view.operations.length, 6);
  for (let index = 1; index < visits.length; index++) {
    const prior = visits[index - 1];
    const barrier = view.events.find((event) => event.activation === prior.activation && event.event_type === 'layer_barrier_reached');
    assert(barrier && barrier.seq < visits[index].seq, 'next milestone crossed an unfinished barrier');
    for (const operation of view.operations.filter((item) => item.activation === prior.activation)) {
      assert(view.events.some((event) => event.event_type === 'operation_terminal'
        && event.execution_id === operation.execution_id && event.seq < barrier.seq));
    }
  }
  assert.equal(visits[2].decision_summary, 'reflect_and_return');
  assert(visits[2].reflection, 'backward transition must carry reflection');
}

async function main() {
  assertRelease();
  const id = await prepare(); console.log(JSON.stringify({ stage: 'created', id, evidence }));
  const states = []; const deadline = Date.now() + 2400000; let view;
  while (Date.now() < deadline) {
    view = await api('GET', `/api/brain/runs/${id}/layered`); save('view', view);
    const state = { phase: view.run.phase, round: view.run.round, layer: view.run.layer,
      operations: view.operations.map((operation) => [operation.node_id, operation.attempt, operation.status]) };
    if (JSON.stringify(state) !== JSON.stringify(states.at(-1)?.state)) {
      states.push({ at: Date.now(), state }); save('states', states); console.log(JSON.stringify(state));
    }
    if (['completed', 'failed', 'blocked', 'cancelled'].includes(view.run.phase)) break;
    await sleep(2000);
  }
  assert.equal(view.run.phase, 'completed', view.run.error || 'milestone acceptance did not complete');
  assertBarrier(view);
  const operations = view.plan.nodes.map((node) => view.operations.filter((item) => item.node_id === node.node_id)
    .sort((a, b) => b.activation - a.activation)[0]);
  assert.deepEqual(operations.map((item) => item.execution_kind).sort(), ['agent', 'dag', 'operator']);
  assert(operations.every((item) => item.status === 'done'));
  const marker = view.plan.inputs.request;
  for (const operation of operations) {
    const detail = await api('GET', `/api/executions/${operation.execution_id}`);
    save(`detail-${operation.execution_kind}`, detail);
    assert.equal(detail.execution.status, 'done');
    assert(JSON.stringify(detail.result.scheduler_output).includes(marker));
  }
  const panels = await inspectPanels({ base: settings.public_url, token, id, view, operations, marker, evidence });
  assertRelease();
  const result = { result: 'PASS', commit, run_id: id, visits: 4, operations: 6, panels };
  save('result', result); console.log(JSON.stringify(result));
}

main().catch((error) => { save('failure', { error: error.stack }); console.error(error); process.exitCode = 1; });
