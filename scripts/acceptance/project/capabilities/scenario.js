const assert = require('node:assert/strict');

const KINDS = ['agent', 'operator', 'dag', 'brain'];
const LABELS = { agent: 'Agent', operator: 'Operator', dag: 'DAG 工作流', brain: '大脑调度' };
function instruction(marker) {
  return 'Integration acceptance. Do not use tools, files or network. Add 2, 3, and 5. ' +
    'Return a JSON object with marker="' + marker + '", sum=10, passed=true. ' +
    'Preserve the marker from the task input. For verification, compare the supplied upstream results.';
}

async function prepare(api, tag) {
  const agent = tag + '-executor';
  await api('POST', '/api/agents/resources/prompts', { name: agent,
    files: [{ path: 'soul.md', content_b64: Buffer.from(
      'Follow the bounded integration task. Return its requested JSON without using tools, files or network.'
    ).toString('base64') }] });
  await api('POST', '/api/agents', { name: agent, current: { prompt: agent } });
  const dag = tag + '-dag';
  await api('POST', '/api/dag/defs', { spec: { name: dag,
    description: 'Project TODO capability integration acceptance',
    steps: [{ name: 'verify', kind: { type: 'agent', agent: 'plan',
      prompt: 'Read the supplied execution input. Add 2, 3, and 5, and verify any supplied upstream results. ' +
        'Return JSON with the exact marker from input, sum=10, passed=true. Do not use tools, files or network.' } }],
  } });
  const marker = tag + '-brain';
  const task = instruction(marker);
  const plan = {
    schema_version: 7, title: tag + '-plan', objective: task +
      ' First dispatch both compute nodes, then the DAG verification node. Bind todo from root inputs ' +
      'to every node. Bind the complete compute results as agent_result and operator_result into verify ' +
      'using execution bindings with path="" (the JSON Pointer root, never path="/"). ' +
      'Complete only after both layers pass. The final summary must contain the marker and sum 10.',
    inputs: {}, max_rounds: 1,
    layers: [
      { layer_id: 'compute', title: '并行计算', task, objective: 'Compute independently',
        success_criteria: 'Both results preserve the marker and have sum=10 and passed=true.' },
      { layer_id: 'verify', title: 'DAG 验证', task, objective: 'Verify both upstream results',
        success_criteria: 'DAG verify output preserves the marker and has sum=10 and passed=true.' },
    ],
    nodes: [
      { node_id: 'agent', layer_id: 'compute', capability_id: 'agent-' + agent, title: 'Compute 2+3+5; preserve the input marker in JSON', objective: task },
      { node_id: 'operator', layer_id: 'compute', capability_id: 'builtin-operator', title: 'Independently compute 2+3+5; preserve the input marker in JSON', objective: task },
      { node_id: 'verify', layer_id: 'verify', capability_id: 'dag-' + dag, title: 'Verify both bound results; return marker, sum and passed', objective: task },
    ],
  };
  const planId = tag + '-plan';
  await api('POST', '/api/brain/plan-defs', { id: planId, version: 1, created_at: Date.now(),
    changelog: 'Project TODO E2E fixture', plan });
  const library = (await api('GET', '/api/brain/library')).capabilities;
  for (const id of ['agent-' + agent, 'builtin-operator', 'dag-' + dag]) {
    assert(library.some((entry) => entry.id === id), 'shared capability missing: ' + id);
  }
  const ids = { agent: 'agent-' + agent, operator: 'builtin-operator', dag: 'dag-' + dag, brain: 'plan-' + planId + '@1' };
  const capabilities = Object.fromEntries(Object.entries(ids).map(([kind, id]) => {
    const capability = library.find((entry) => entry.id === id);
    assert(capability?.definition, 'capability unavailable: ' + id);
    return [kind, { id, label: `${capability.summary || capability.target} · ${capability.target}` }];
  }));
  return { tag, agent, dag, planId, plan, capabilities, kinds: KINDS };
}

function jsonObjects(text) {
  const values = [];
  let start = -1, depth = 0, quoted = false, escaped = false;
  for (let i = 0; i < text.length; i++) {
    const char = text[i];
    if (start < 0) {
      if (char === '{') { start = i; depth = 1; }
      continue;
    }
    if (quoted) {
      if (escaped) escaped = false;
      else if (char === '\\') escaped = true;
      else if (char === '"') quoted = false;
    } else if (char === '"') quoted = true;
    else if (char === '{') depth++;
    else if (char === '}' && --depth === 0) {
      try { values.push(JSON.parse(text.slice(start, i + 1))); } catch { /* Prose is not JSON. */ }
      start = -1;
    }
  }
  return values;
}

function taskOutputs(value, marker) {
  if (typeof value === 'string') {
    try { return taskOutputs(JSON.parse(value), marker); }
    catch { return jsonObjects(value).flatMap((entry) => taskOutputs(entry, marker)); }
  }
  if (!value || typeof value !== 'object') return [];
  if (Object.hasOwn(value, 'marker')) return [value];
  return ['output_text', 'output_json', 'scheduler_output', 'verify']
    .filter((key) => Object.hasOwn(value, key))
    .flatMap((key) => taskOutputs(value[key], marker));
}

function assertTaskOutput(value, marker) {
  const outputs = taskOutputs(value, marker);
  assert(outputs.length, 'structured task output is missing');
  for (const output of outputs) {
    assert.equal(output.marker, marker, 'task returned another input marker');
    assert.equal(output.sum, 10, 'task returned an incorrect sum');
    assert.equal(output.passed, true, 'task verification did not pass');
  }
}

function assertResult(text, marker) {
  assert.equal(typeof text, 'string', 'an execution result is required');
  assert(text.includes(marker), 'result belongs to another input: ' + text.slice(0, 300));
  assert.match(text, /\b10\b/, 'expected computed sum');
  // Brain summaries may be prose; every child output is checked separately.
  if (!marker.endsWith('-brain')) assertTaskOutput(text, marker);
}

function assertBrain(view, marker) {
  assert.equal(view.run.phase, 'completed', view.run.error);
  const visits = view.events.filter((event) => event.event_type === 'layer_started');
  assert.deepEqual(visits.map((event) => [event.round, event.layer]), [[1, 1], [1, 2]]);
  assert.equal(view.operations.length, 3);
  assert.deepEqual(view.operations.map((op) => op.execution_kind).sort(), ['agent', 'dag', 'operator']);
  assert.equal(new Set(view.operations.map((op) => op.execution_id)).size, 3);
  assert(view.operations.every((op) => op.status === 'done'));
  const barrier = view.events.find((event) => event.event_type === 'layer_barrier_reached' && event.activation === visits[0].activation);
  assert(barrier && barrier.seq < visits[1].seq, 'DAG started before parallel work finished');
  for (const op of view.operations.filter((op) => op.activation === visits[0].activation)) {
    assert(view.events.some((event) => event.event_type === 'operation_terminal' &&
      event.execution_id === op.execution_id && event.seq < barrier.seq));
  }
  assert(JSON.stringify(view).includes(marker));
}

module.exports = { KINDS, LABELS, instruction, prepare, assertResult, assertTaskOutput, assertBrain };
