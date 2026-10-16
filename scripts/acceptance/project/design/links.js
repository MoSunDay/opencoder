const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

// Only execution reads are fixtures; task fields and their persisted values use
// the isolated Server. New tabs load the real shell and native detail component.
async function verifyExecutionLinks({ page, api, todo, output }) {
  const context = page.context();
  const assignments = ['agent', 'dag'].map((kind) => ({ execution_id: `design-${kind}`, kind, name: `${kind} 原生记录` }));
  const linksPath = `/api/project/todos/${todo.id}/executions`;
  const linksPattern = `**${linksPath}`;
  const detailPattern = /\/api\/executions\/design-(agent|dag)(?:[/?]|$)/;
  const detailHandler = async (route) => {
    const parts = new URL(route.request().url()).pathname.split('/');
    const id = parts[3], kind = id.slice('design-'.length);
    const execution = { id, kind, status: 'done', node_id: 'fixture-node', created_at: Date.now() };
    if (parts[4] === 'events') return route.fulfill({ contentType: 'text/event-stream', body: 'event: stream_end\ndata: {"finished":true}\n\n' });
    let json = parts[4] === 'index' ? execution
      : parts[4] === 'events-page' ? { events: [], head_seq: 0 }
      : parts[4] === 'result' ? { summary: '执行链接显示验收', kind }
      : parts[4] === 'messages' ? { chunks: [], more: false }
      : { execution, request: { kind, target: 'fixture', input: {} }, annotations: {} };
    await route.fulfill({ json });
  };
  await context.route(linksPattern, (route) => route.fulfill({ json: { assignments } }));
  await context.route(detailPattern, detailHandler);
  const checks = [], writes = [], pageErrors = [];
  const listen = (opened) => opened.on('pageerror', (error) => pageErrors.push(error.message));
  context.on('page', listen);
  const observe = (request) => { if (request.method() !== 'GET' && new URL(request.url()).pathname.startsWith('/api/project/')) writes.push(request.method()); };
  page.on('request', observe);
  try {
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: 'TODO', exact: true }).click();
    await page.getByRole('button', { name: todo.title, exact: true }).click();
    const drawer = page.getByRole('dialog', { name: `TODO · ${todo.title}`, exact: true });
    await drawer.getByLabel('TODO 标题', { exact: true }).fill('点击执行记录时应保留的任务草稿');
    await drawer.getByLabel('评论正文', { exact: true }).fill('点击执行记录时应保留的评论草稿');
    await drawer.getByRole('tab', { name: '执行', exact: true }).click();
    assert.equal(await drawer.getByRole('button', { name: '查看', exact: true }).count(), 0);
    for (const record of assignments) {
      const link = drawer.getByRole('link', { name: record.execution_id, exact: true });
      const [detail] = await Promise.all([page.waitForEvent('popup'), link.click()]);
      try {
        await detail.locator('.execution-view-full .ant-descriptions').first().getByText(record.kind === 'agent' ? 'Agent' : 'DAG', { exact: true }).waitFor();
        assert.equal(new URL(detail.url()).searchParams.get('execution'), record.execution_id);
        await detail.reload({ waitUntil: 'networkidle' });
        await detail.locator('.execution-view-full').waitFor();
        assert.equal(await detail.locator('.execution-view-full .ant-alert-error').count(), 0);
        assert.equal(await detail.getByRole('dialog', { name: record.execution_id, exact: true }).count(), 1);
        await detail.screenshot({ path: path.join(output, `execution-link-${record.kind}.png`), animations: 'disabled' });
        checks.push({ kind: record.kind, id: record.execution_id, new_page: true, survives_reload: true });
      } finally { await detail.close(); }
    }
    await drawer.getByRole('tab', { name: '任务', exact: true }).click();
    assert.equal(await drawer.getByLabel('TODO 标题', { exact: true }).inputValue(), '点击执行记录时应保留的任务草稿');
    assert.equal(await drawer.getByLabel('评论正文', { exact: true }).inputValue(), '点击执行记录时应保留的评论草稿');
    assert.equal((await api('GET', `/api/project/todos/${todo.id}`)).title, todo.title);
    assert.deepEqual(writes, []);
    assert.deepEqual(pageErrors, []);
    await drawer.locator('.ant-drawer-close').click();
    await page.getByRole('button', { name: '放弃修改', exact: true }).click();
    await drawer.waitFor({ state: 'hidden' });
    fs.writeFileSync(path.join(output, 'execution-links.json'), JSON.stringify({ result: 'PASS', checks, drafts_preserved: true, writes, pageErrors }, null, 2));
  } finally {
    page.off('request', observe);
    context.off('page', listen);
    await context.unroute(linksPattern); await context.unroute(detailPattern, detailHandler);
  }
}

module.exports = { verifyExecutionLinks };
