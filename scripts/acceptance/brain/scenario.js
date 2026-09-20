// Shared real API scenario. No fake capabilities or execution records.
const assert = require('node:assert/strict');

async function prepare(api, tag, nodeId) {
  const instruction = 'Read the named Scheduler inputs. Echo the request value exactly. Do not modify files, invoke tools, or access the network.';
  await api('POST', '/api/dag/defs', { spec: { name: tag, description: 'Input reference acceptance', steps: [
    { name: 'analyze', kind: { type: 'agent', prompt: `ANALYZE_INPUT_ECHO. ${instruction} Return JSON {"echo":<request value>}.` } },
  ] } });
  await api('POST', '/api/teams', { name: tag, captain: 'act', members: [{ agent: 'act' }, { agent: 'plan' }] });
  await api('POST', '/api/todo/templates', { name: tag, spec: {
    schema_version: 1, id: tag, name: tag, objective: instruction,
    todos: [{ id: 'echo', title: 'Verify bound input', requirement_background: 'Five capability acceptance',
      instructions: `${instruction} Put the echoed value in candidate.result.`, max_attempts: 1,
      acceptance: { criteria: 'The accepted result equals the bound request input.' } }],
  } });
  const capabilities = [`dag-${tag}`, 'builtin-agent-act', 'builtin-operator', `team-${tag}`, `todos-${tag}-v1`];
  const catalog = (await api('GET', '/api/brain/library')).capabilities;
  for (const id of capabilities) assert(catalog.some((c) => (c.id || c.capability_id) === id), `missing registered capability ${id}`);
  return {
    id: `brain-${tag}`, schema_version: 3, node_id: nodeId, capability_ids: capabilities, max_rounds: 3,
    objective: `Acceptance only. ${instruction} Follow exactly two dispatch rounds: round 1 dispatch dag-${tag}, binding request/repo/commit/branch from root inputs. Its analyze step returns {echo: value}. Round 2 dispatch ALL FOUR remaining allowed capabilities in parallel, binding request from the successful DAG execution at /analyze/echo and repo/commit/branch from root inputs. Agent and Operator final reply must include the request value verbatim. Team must discuss and include that value verbatim in final_summary. TODO must accept an echo candidate with exactly that value. Complete only after all five operations succeeded, citing all five execution IDs. No other task is authorized.`,
    inputs: { request: `echo-${tag}-参数 "quoted"`, repo: { name: 'fixture/repository', readonly: true }, commit: '0123456789abcdef', branch: 'feature/参数 with spaces' },
  };
}

async function collect(api, request, until) {
  const id = request.id;
  const snapshot = await until(async () => {
    const value = await api('GET', `/api/brain/runs/${id}`);
    return ['completed', 'failed', 'blocked', 'cancelled'].includes(value.run.phase) && value;
  }, 'five capability barrier', 300000);
  assert.equal(snapshot.run.phase, 'completed', `${id}: ${snapshot.run.phase}: ${snapshot.run.error}`);
  assert.equal(snapshot.run.round, 2);
  assert.equal(snapshot.operations.length, 5);
  assert.deepEqual(snapshot.operations.map((o) => o.execution_kind).sort(), ['agent', 'dag', 'operator', 'team', 'todos']);
  assert(snapshot.operations.every((o) => o.status === 'done'));
  const view = await api('GET', `/api/brain/runs/${id}/view`);
  assert.deepEqual(view.rounds.map((r) => [r.round, r.operations.length]), [[1, 1], [2, 4]]);
  assert(view.capabilities.every((c) => c.definition === undefined));
  const forbidden = new Set(['messages', 'scheduler_output', 'scheduler_inputs', 'dag_steps', 'definition', 'input', 'output', 'graph']);
  const indexOnly = (value) => {
    if (!value || typeof value !== 'object') return;
    for (const [key, item] of Object.entries(value)) { assert(!forbidden.has(key), `body leaked into scheduler: ${key}`); indexOnly(item); }
  };
  indexOnly(snapshot); indexOnly(view);
  let after = 0; const events = [];
  while (true) {
    const page = await api('GET', `/api/brain/runs/${id}/events-page?after=${after}&limit=3`);
    indexOnly(page);
    for (const event of page.events) { assert(event.seq > after); after = event.seq; events.push(event); }
    if (!page.more) break;
    assert(page.events.length > 0);
  }
  assert.equal(events.filter((e) => e.event_type === 'round_barrier_reached').length, 2);
  assert.equal(events.filter((e) => e.event_type === 'run_completed').length, 1);
  for (const operation of snapshot.operations) {
    const detail = await api('GET', `/api/executions/${operation.execution_id}`);
    assert.equal(detail.execution.status, 'done');
    const input = detail.request.input;
    assert.deepEqual(input.scheduler_inputs, request.inputs);
    const output = detail.result.scheduler_output;
    // DAG/TODO expose structured output paths; conversations expose free text.
    // Keep exact value checks on structured references and exact byte inclusion
    // in conversation results, without rewriting any executor output.
    if (operation.execution_kind === 'dag' || operation.execution_kind === 'todos') {
      const echo = operation.execution_kind === 'dag' ? output.analyze.echo : output.echo;
      assert.equal(echo, request.inputs.request, `${operation.execution_kind} output did not preserve input`);
    } else {
      assert.equal(typeof output, 'string');
      assert(output.includes(request.inputs.request), `${operation.execution_kind} result omitted the exact input value`);
    }
    if (operation.round === 2) assert.equal(input.bindings.request.kind, 'execution');
    const round = await api('GET', `/api/brain/runs/${id}/rounds/${operation.round}`);
    assert(JSON.stringify(round).includes(operation.execution_id));
  }
  const replay = await api('POST', '/api/brain/runs', request);
  assert.equal(replay.run_id, id);
  return { result: 'PASS', run_id: id, round: snapshot.run.round,
    operations: snapshot.operations.map(({ execution_id, execution_kind, round }) => ({ execution_id, execution_kind, round })),
    last_event_seq: after };
}

module.exports = { prepare, collect };
