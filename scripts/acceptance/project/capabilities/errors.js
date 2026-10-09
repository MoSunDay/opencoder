const assert = require('node:assert/strict');
const { linksPath } = require('./checks');
const { instruction } = require('./scenario');

async function verify(h, state, control) {
  const reports = {};
  const todo = state.hierarchy.todos.agent;
  const path = linksPath(todo);
  const prior = (await h.api('GET', path)).assignments;
  const resultPath = '/api/executions/' + state.executions.agent + '/result';
  const original = await h.api('GET', resultPath);
  await h.stopNode();
  try {
    await h.until(async () => !(await h.api('GET', '/api/nodes')).nodes.find((node) => node.id === h.nodeId).online,
      'node offline', 60000);
    const unavailable = await h.request('GET', resultPath);
    assert.equal(unavailable.response.status, 503, 'offline result must report node unavailability');
    assert.deepEqual((await h.api('GET', path)).assignments, prior, 'offline reads changed execution references');
    reports.offline = { status: unavailable.response.status, references: prior };
  } finally { await h.startNode(); }
  await h.until(async () => (await h.api('GET', resultPath)).summary === original.summary,
    'node result available after restart', 60000);

  async function create(mode) {
    control.mode = mode;
    const marker = state.scenario.tag + '-' + mode + '-agent';
    const todo = await h.api('POST', '/api/project/todos', { title: marker, draft: instruction(marker), board_status: 'todo' });
    const id = 'agent-' + marker;
    await h.api('POST', '/api/executions', { id, kind: 'agent', node_id: h.nodeId, input: { prompt: todo.draft } });
    await h.until(async () => (await h.request('GET', '/api/executions/' + id + '/index')).response.ok,
      'negative execution index');
    await h.api('POST', linksPath(todo), { execution_id: id });
    return { todo, id };
  }
  async function terminal(value, expected) {
    await h.until(async () => (await h.api('GET', '/api/executions/' + value.id + '/index')).status === expected,
      expected + ' execution state', 180000);
    const result = await h.api('GET', '/api/executions/' + value.id + '/result');
    assert.equal(result.summary, null);
    const record = (await h.api('GET', linksPath(value.todo))).assignments[0];
    assert.equal(record.result_md, undefined);
    assert.equal(record.sync_state, undefined);
    const overview = await h.api('GET', '/api/project/overview');
    assert.equal(overview.backlog.find((row) => row.id === value.todo.id).board_status, 'todo');
    return result;
  }

  reports.failed = await terminal(await create('fail'), 'error');
  reports.empty = await terminal(await create('empty'), 'idle');
  const before = control.requests.length;
  const cancelled = await create('hold');
  await h.until(() => control.requests.length > before && control.waiting.length > 0, 'in-flight model call');
  await h.api('POST', '/api/executions/' + cancelled.id + '/commands', { action: 'cancel', input: {} });
  reports.cancelled = await terminal(cancelled, 'cancelled');
  control.mode = 'normal'; for (const release of control.waiting.splice(0)) release();
  return reports;
}
module.exports = { verify };
