const assert = require('node:assert/strict');
const { KINDS, assertResult, assertTaskOutput, assertBrain } = require('./scenario');
const ui = require('./browser');

const linksPath = (todo) => '/api/project/todos/' + todo.id + '/executions';
async function finished(h, todo, id) {
  const result = await h.until(async () => {
    const index = await h.api('GET', '/api/executions/' + id + '/index');
    const records = (await h.api('GET', linksPath(todo))).assignments;
    const record = records.find((row) => row.execution_id === id);
    const brain = index.kind === 'brain' && ['idle', 'waiting'].includes(index.status)
      ? (await h.api('GET', '/api/brain/runs/' + id + '/layered')).run : undefined;
    return record && (['done', 'error', 'cancelled'].includes(index.status) ||
      (['agent', 'operator'].includes(index.kind) && index.status === 'idle') ||
      (brain && ['blocked', 'paused', 'failed'].includes(brain.phase)))
      ? { index, record, ...(brain ? { brain } : {}) } : false;
  }, 'execution result: ' + id, h.live || h.realModel ? 900000 : 300000);
  const { index, record } = result;
  assert(!result.brain || result.brain.phase === 'completed', 'Brain did not complete without intervention: ' + JSON.stringify(result.brain));
  assert(['done', 'idle'].includes(index.status), 'execution failed: ' + JSON.stringify(result));
  assert.equal(record.kind, index.kind);
  const conclusion = await h.api('GET', '/api/executions/' + id + '/result');
  if (index.kind === 'dag') {
    const step = await h.api('GET', '/api/dag/runs/' + id + '/steps/verify');
    assertTaskOutput(step.output, todo.marker);
    result.step = step;
  } else assertResult(conclusion.summary, todo.marker);
  result.conclusion = conclusion;
  for (const field of ['steps', 'operations', 'messages', 'events', 'plan', 'result_md', 'sync_state']) {
    assert.equal(record[field], undefined, 'TODO must keep execution references only');
  }
  return result;
}

async function progress(h, hierarchy, count) {
  const overview = await h.api('GET', '/api/project/overview');
  const goal = overview.goals.find((row) => row.id === hierarchy.goal.id);
  assert(goal);
  const group = goal.initiatives.find((row) => row.id === hierarchy.initiative.id);
  assert(group);
  assert.deepEqual(goal.progress, { total: 4, done: count });
  assert.deepEqual(group.progress, { total: 4, done: count });
  assert.deepEqual(group.todos.map((row) => row.id).sort(), Object.values(hierarchy.todos).map((row) => row.id).sort());
  await ui.progress(h.page, hierarchy, count, h.root);
}

async function verifyBrain(h, id, marker) {
  const view = await h.api('GET', '/api/brain/runs/' + id + '/layered');
  assertBrain(view, marker);
  const children = [];
  for (const operation of view.operations) {
    const detail = await h.api('GET', '/api/executions/' + operation.execution_id);
    assert.equal(detail.execution.status, 'done');
    const output = detail.result.scheduler_output;
    const result = operation.execution_kind === 'dag' ? output.verify : output;
    assertTaskOutput(result, marker);
    if (operation.execution_kind === 'dag') {
      const bindings = detail.request.input.bindings;
      for (const kind of ['agent', 'operator']) {
        assert.equal(bindings[kind + '_result'].kind, 'execution');
        assert.equal(bindings[kind + '_result'].execution_id, view.operations.find((op) => op.execution_kind === kind).execution_id);
        const input = detail.request.input.layered_inputs[kind + '_result'];
        const upstream = await h.api('GET', '/api/executions/' + bindings[kind + '_result'].execution_id);
        assert.deepEqual(input, upstream.result.scheduler_output, 'bound input differs from the actual upstream output');
        assertTaskOutput(input, marker);
      }
    }
    children.push({ id: operation.execution_id, kind: operation.execution_kind, output });
  }
  return { run: view.run, children, events: view.events };
}

async function rerun(h, state) {
  const todo = state.hierarchy.todos.agent;
  const prior = await finished(h, todo, state.executions.agent);
  const id = await ui.launch(h.page, state.scenario, todo, 'agent');
  assert.notEqual(id, prior.index.id);
  await finished(h, todo, id);
  const records = (await h.api('GET', linksPath(todo))).assignments;
  assert.equal(records.length, 2);
  assert.deepEqual(records.filter((row) => row.execution_id === prior.index.id), [prior.record]);
  await h.api('POST', linksPath(todo), { execution_id: id });
  assert.deepEqual((await h.api('GET', linksPath(todo))).assignments, records,
    'duplicate linking changed saved records');
  return id;
}

async function run(h, state, save) {
  if (!state.hierarchy) { state.hierarchy = await ui.create(h.page, state.scenario); save(); }
  state.executions ||= {};
  state.results ||= {};
  for (const kind of KINDS) {
    const todo = state.hierarchy.todos[kind];
    if (!state.executions[kind]) {
      const fixture = !h.live && !h.realModel;
      state.executions[kind] = await ui.launch(h.page, state.scenario, todo, kind, fixture);
      save();
    }
    console.log(JSON.stringify({ stage: 'dispatched', kind, id: state.executions[kind] }));
    state.results[kind] = await finished(h, todo, state.executions[kind]);
    assert.equal(state.results[kind].index.kind, kind);
    assert.equal(state.results[kind].index.node_id, h.nodeId);
    save();
    await h.page.reload({ waitUntil: 'networkidle' });
    const inspected = await ui.inspect(h.page, todo, state.executions[kind], h.root, state.results[kind].index.status);
    if (kind !== 'dag') assertResult(inspected.result, todo.marker);
    state.uiFeatures = { todoExecutionStatusColumn: inspected.statusColumn };
    if (kind === 'brain') {
      state.brain = await verifyBrain(h, state.executions.brain, todo.marker);
      assert.deepEqual((await h.api('GET', linksPath(todo))).assignments.map((row) => row.execution_id),
        [state.executions.brain], 'TODO must link the root Brain, not its child operations');
    }
    console.log(JSON.stringify({ stage: 'verified', kind, id: state.executions[kind] }));
    save();
  }
  if (!state.rerun) { state.rerun = await rerun(h, state); save(); }
  if (!state.progressDone) {
    const count = state.completedTodos || 0;
    await progress(h, state.hierarchy, count);
    for (let index = count; index < KINDS.length; index++) {
      await ui.markDone(h.page, state.hierarchy.todos[KINDS[index]]);
      await progress(h, state.hierarchy, index + 1);
      state.completedTodos = index + 1; save();
    }
    state.progressDone = true; save();
  }
}
module.exports = { run, finished, linksPath };
