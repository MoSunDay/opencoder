// Built server + two real nodes + browser, isolated durable storage only.
// TMPDIR must reside on a volume meeting the production node storage gate.
// PLATFORM_BIN_DIR=... node scripts/acceptance/project/main.js
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const { harness, until, pause } = require('./harness');
const { openBrowser, createHierarchy, projectPage } = require('./browser');
const { audit } = require('./audit');
let h, browser;
const errors = [];
async function main() {
  h = await harness();
  console.log(JSON.stringify({ stage: 'ready', evidence: h.root }));
  const { api, field, mode } = h;
  const node = (await api('GET', '/api/nodes')).nodes.find((node) => node.name === 'node-a');
  const opened = await openBrowser(h, errors); browser = opened.browser;
  const browserPage = opened.page;
  const { goal, milestone, todo, backlog } = await createHierarchy(browserPage);
  assert.equal(milestone.goal_id, goal.id);
  assert.equal(todo.milestone_id, milestone.id); assert.equal(backlog.milestone_id, null);
  const root = `project-${todo.id}`;
  const runs = [];
  async function run(action, target = todo.id) {
    const run_id = `prun-${crypto.randomUUID()}`;
    const receipt = await api('POST', `/api/project/todos/${target}/${action}`, { run_id, node_id: node.id });
    assert.equal(receipt.run_id, run_id); assert.equal(receipt.node_id, node.id);
    await until(async () => {
      const detail = await api('GET', `/api/executions/project-${target}`);
      assert.notEqual(detail.execution.status, 'error', detail.error || 'unexpected execution failure');
      return detail.execution.status === 'idle';
    }, `${action} completion`, 120000);
    await until(async () => (await api('GET', '/api/executions?kind=project')).executions.some((index) => index.id === run_id), 'run index');
    const detail = await api('GET', `/api/executions/${run_id}`);
    assert.equal(detail.run.status, 'done'); assert.equal(detail.retention, 'complete');
    const output = typeof detail.run.output_md === 'string' ? detail.run.output_md : await field(run_id, `project.run.${run_id}.output_md`);
    assert.equal(output, mode.text);
    await until(async () => (await api('GET', '/api/executions?kind=project')).executions.find((index) => index.id === run_id)?.status === 'done', 'terminal index convergence');
    assert(Date.now() - detail.run.finished_at <= 10000, 'terminal status must converge within ten seconds');
    const calls = mode.requests;
    const retry = await api('POST', `/api/project/todos/${target}/${action}`, { run_id, node_id: node.id });
    assert.equal(retry.run_id, run_id); assert.equal(mode.requests, calls);
    const input = JSON.parse(typeof detail.run.input_snapshot === 'string' ? detail.run.input_snapshot : await field(run_id, `project.run.${run_id}.input_snapshot`));
    runs.push({ id: run_id, target, trace: detail.replay, input, summary: detail.run }); return runs.at(-1);
  }
  mode.text = '# Large plan\n' + 'output 界 '.repeat(10000) + 'END';
  const first = await run('plan');
  mode.text = 'completed execution';
  for (let i = 0; i < 24; i++) await run('execute');
  await run('plan', backlog.id); await run('execute', backlog.id);
  console.log(JSON.stringify({ stage: '27-runs', model_requests: mode.requests }));
  const firstOutput = await field(first.id, `project.run.${first.id}.output_md`);
  assert.equal(firstOutput, '# Large plan\n' + 'output 界 '.repeat(10000) + 'END');
  assert.equal(first.input.todo.draft, todo.draft);
  const page1 = await api('GET', `/api/project/todos/${todo.id}/runs`);
  assert.equal(page1.runs.length, 20); assert.equal(page1.more, true);
  const page2 = await api('GET', `/api/project/todos/${todo.id}/runs?before_version=${page1.next_version}`);
  assert.equal(page2.runs.length, 5); assert.equal(page2.more, false);
  // Custom resource versions are published through the actual API.
  const files = (text) => [{ path: 'soul.md', content_b64: Buffer.from(text).toString('base64') }];
  await api('POST', '/api/agents/resources/prompts', { name: 'acceptance-pack', files: files('Agent version one') });
  await api('POST', '/api/agents', { name: 'acceptance-agent', current: { prompt: 'acceptance-pack' } });
  await api('PATCH', `/api/project/todos/${todo.id}`, { agent: 'acceptance-agent' });
  const agent1 = await run('execute');
  assert.notEqual(agent1.trace.session_id, runs[24].trace.session_id);
  assert.equal(agent1.input.agent.resources.prompts.version, 1);
  await api('PUT', '/api/agents/resources/prompts/acceptance-pack', { name: 'acceptance-pack', files: files('Agent version two') });
  const agent2 = await run('execute');
  assert.notEqual(agent2.trace.session_id, agent1.trace.session_id);
  assert.equal(agent2.input.agent.resources.prompts.version, 2);
  assert.notEqual(agent1.input.agent.digest, agent2.input.agent.digest);
  mode.kind = 'artifact';
  const artifactRun = await run('execute'); mode.kind = 'normal';
  assert.equal(artifactRun.trace.artifacts.length, 1);
  const artifact = artifactRun.trace.artifacts[0];
  fs.writeFileSync(path.join(h.dirs[1], 'report.txt'), 'mutated later');
  const response = await fetch(`${h.base}/api/executions/${artifactRun.id}/artifact?step=${artifact.id}&file=${encodeURIComponent(artifact.name)}`, { headers: { authorization: `Bearer ${h.token}` } });
  assert.equal(response.status, 200);
  const bytes = Buffer.from(await response.arrayBuffer());
  assert.equal(bytes.toString(), 'original immutable artifact 界\n');
  assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'), artifact.sha256);
  // Failed model calls retain both their accepted input and actual request.
  mode.kind = 'failure';
  const failed = `prun-${crypto.randomUUID()}`;
  await api('POST', `/api/project/todos/${todo.id}/execute`, { run_id: failed });
  await until(async () => (await api('GET', `/api/executions/${root}`)).execution.status === 'error', 'failed model');
  mode.kind = 'normal';
  await until(async () => (await api('GET', '/api/executions?kind=project')).executions.some((index) => index.id === failed), 'failed index');
  const failure = await api('GET', `/api/executions/${failed}`);
  assert.equal(failure.run.status, 'failed'); assert(failure.run.input_snapshot);
  assert((await field(failed, 'archive.response-1.jsonl')).includes('injected model failure'));
  // Crash a node with an accepted model request in flight, then resume explicitly.
  mode.kind = 'hang'; const before = mode.requests;
  const interrupted = `prun-${crypto.randomUUID()}`;
  await api('POST', `/api/project/todos/${todo.id}/execute`, { run_id: interrupted });
  await until(async () => mode.requests > before, 'in-flight request');
  await h.stop(h.nodes[0], 'SIGKILL');
  await until(async () => !(await api('GET', '/api/nodes')).nodes.find((n) => n.id === node.id).online, 'owner offline');
  await api('GET', `/api/executions/${first.id}`, undefined, 503);
  h.start('opencoder-agent', h.nodes[0].spawnargs.slice(1), h.dirs[1]);
  await until(async () => (await api('GET', '/api/nodes')).nodes.find((n) => n.id === node.id)?.snapshot?.ready && (await api('GET', '/api/nodes')).nodes.find((n) => n.id === node.id)?.online, 'owner restart', 30000);
  mode.kind = 'normal';
  await run('execute');
  const partial = await api('GET', `/api/executions/${interrupted}`);
  assert.equal(partial.run.status, 'cancelled'); assert.equal(partial.retention, 'partial');
  assert(Number.isInteger(partial.replay.messages_through));
  assert((await field(interrupted, 'archive.request-1.json')).includes('messages'));
  // Cancel a separate TODO so the completed project remains reusable.
  mode.kind = 'partial-hang'; const cancelCalls = mode.requests;
  const cancelled = `prun-${crypto.randomUUID()}`;
  await api('POST', `/api/project/todos/${backlog.id}/execute`, { run_id: cancelled });
  await until(async () => mode.requests >= cancelCalls + 2, 'cancel after tool output');
  await api('POST', `/api/executions/project-${backlog.id}/commands`, { action: 'cancel', input: {} });
  await until(async () => (await api('GET', `/api/executions/project-${backlog.id}`)).execution.status === 'cancelled', 'cancel convergence');
  mode.kind = 'normal';
  const cancellation = await api('GET', `/api/executions/${cancelled}`);
  assert.equal(cancellation.run.status, 'cancelled'); assert(cancellation.run.input_snapshot);
  assert.equal(cancellation.run.output_md, 'partial output before cancellation');
  await audit(h, runs.map((run) => run.id).concat(failed, interrupted, cancelled));
  // Browser exercises the built SPA and exact run replay entry.
  await browserPage.goto(h.base, { waitUntil: 'networkidle' });
  await projectPage(browserPage);
  await browserPage.getByRole('tab', { name: 'TODO', exact: true }).click();
  await browserPage.getByText('Acceptance TODO', { exact: true }).click();
  await browserPage.getByText('加载更早记录', { exact: true }).click();
  const earliest = browserPage.locator('.ant-timeline-item').filter({ has: browserPage.getByText('v1', { exact: true }) });
  await earliest.getByText('查看本次输入与过程', { exact: true }).click();
  await browserPage.getByText('本次输入与 Agent 版本', { exact: true }).waitFor();
  await browserPage.screenshot({ path: path.join(h.root, 'project-replay.png'), fullPage: true });
  console.log(JSON.stringify({ stage: 'observation', attempts: runs.length + 3, started_at: new Date().toISOString() }));
  const samples = [];
  const started = Date.now();
  const duration = 1800000;
  do {
    const indexes = (await api('GET', '/api/executions?kind=project')).executions;
    assert.equal(new Set(indexes.map((index) => index.id)).size, indexes.length);
    for (const record of runs) {
      const index = indexes.find((index) => index.id === record.id); assert(index, record.id);
      assert.equal(index.node_id, node.id); assert.equal(index.status, 'done');
      assert.deepEqual(Object.keys(index).sort(), ['created_at', 'id', 'kind', 'node_id', 'status']);
      const detail = await api('GET', `/api/executions/${record.id}`);
      assert.deepEqual(detail.replay, record.trace);
      assert.deepEqual(detail.run, record.summary);
    }
    assert.deepEqual(errors, []);
    samples.push({ at: new Date().toISOString(), indexes: indexes.length, replayed: runs.length });
    fs.writeFileSync(path.join(h.root, 'observation.json'), JSON.stringify(samples, null, 2));
    if (Date.now() - started >= duration) break;
    await pause(Math.max(0, 10000 - (Date.now() - started) % 10000));
  } while (true);
  const report = { attempts: runs.length + 3, successful: runs.length, duration_ms: Date.now() - started, samples: samples.length, browser_errors: errors, first_run: first.id };
  fs.writeFileSync(path.join(h.root, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify({ stage: 'complete', ...report }));
}
main().catch(async (error) => {
  console.error(error); process.exitCode = 1;
  const page = browser?.contexts()[0]?.pages()[0];
  if (page && h) {
    fs.writeFileSync(path.join(h.root, 'browser-failure.html'), await page.content());
    await page.screenshot({ path: path.join(h.root, 'browser-failure.png'), fullPage: true });
  }
}).finally(async () => { await browser?.close(); await h?.close(); if (!h) process.exit(1); });
