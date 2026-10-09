const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

async function conversation({ page, api, root, until }, mode) {
  await page.getByRole('menuitem', { name: /全部执行$/ }).click();
  await page.getByRole('menuitem', { name: /Agent$/ }).click();
  await page.getByText(mode === 'agent' ? 'Agent 模式' : 'Operator 模式', { exact: true }).click();
  const sender = page.getByPlaceholder('输入提示词，Enter 发送，Shift+Enter 换行');
  const transcript = page.locator('.ant-bubble-list');
  assert(await sender.isDisabled(), 'chat cannot send without a selected node');
  await page.getByRole('combobox', { name: '执行节点', exact: true }).click();
  await page.locator('.ant-select-item-option-content').filter({ hasText: 'node-a' }).click();
  if (mode === 'agent') {
    await page.getByRole('combobox', { name: '执行 Agent', exact: true }).click();
    await page.locator('.ant-select-item-option-content').filter({ hasText: 'ui-chat-agent' }).click();
  }
  const replies = page.locator('.ant-bubble-start').getByText('browser node-owned answer', { exact: true });
  const before = (await api('GET', '/api/executions')).executions.length;
  await sender.fill(`ui-${mode}-first-prompt`);
  const accepted = page.waitForResponse((response) => response.url().endsWith('/api/sessions') && response.request().method() === 'POST');
  await page.locator('.ant-sender button:has(.anticon-arrow-up)').click();
  const response = await accepted;
  assert(response.ok(), `${mode} session must be accepted`);
  const request = response.request().postDataJSON();
  assert.equal(request.kind || 'operator', mode);
  if (mode === 'agent') assert.equal(request.agent, 'ui-chat-agent');
  const run = await response.json();
  assert(run.id);
  await replies.first().waitFor();
  await until(async () => ['idle', 'done'].includes((await api('GET', `/api/executions/${run.id}`)).execution.status), 'chat first reply completes');
  await page.locator('.ant-sender-actions-btn-loading-button').waitFor({ state: 'hidden' });
  const count = (await api('GET', '/api/executions')).executions.length;
  assert.equal(count, before + 1, `${mode} creates exactly one execution`);
  await sender.fill(`ui-${mode}-follow-up`);
  const continued = page.waitForResponse((response) => response.url().endsWith(`/api/sessions/${run.id}/prompt`) && response.request().method() === 'POST');
  await page.locator('.ant-sender button:has(.anticon-arrow-up)').click();
  assert((await continued).ok());
  await transcript.getByText(`ui-${mode}-follow-up`, { exact: true }).first().waitFor();
  await until(async () => ['idle', 'done'].includes((await api('GET', `/api/executions/${run.id}`)).execution.status), 'chat continuation completes');
  await page.locator('.ant-sender-actions-btn-loading-button').waitFor({ state: 'hidden' });
  await until(async () => await replies.count() === 2, `${mode} continuation displays its reply`);
  assert.equal((await api('GET', '/api/executions')).executions.length, count);
  await page.screenshot({ path: path.join(root, `chat-${mode}-continuation.png`), animations: 'disabled' });
  // Exercise transport completion without a runner terminal event. The real
  // node still executes and saves its reply; only the browser's event body is
  // reduced to the fleet end marker, as with an empty completed replay.
  const eventRoute = `**/api/sessions/${run.id}/events?*`;
  await page.route(eventRoute, async (route) => {
    const response = await route.fetch();
    assert(response.ok(), 'session event stream must succeed');
    await route.fulfill({ response, body: 'event: stream_end\ndata: {"finished":true}\n\n' });
  });
  try {
    await sender.fill(`ui-${mode}-recover-saved-answer`);
    const continued = page.waitForResponse((response) => response.url().endsWith(`/api/sessions/${run.id}/prompt`) && response.request().method() === 'POST');
    await page.locator('.ant-sender button:has(.anticon-arrow-up)').click();
    assert((await continued).ok());
    await until(async () => await replies.count() === 3, `${mode} empty stream restores the third saved reply`);
    assert.equal(await page.locator('.ant-spin-spinning').count(), 0, 'chat must leave the waiting state');
    await page.locator('.ant-sender-actions-btn-loading-button').waitFor({ state: 'hidden' });
    assert(await sender.isEnabled(), `${mode} input must recover after stream completion`);
    assert.equal((await api('GET', '/api/executions')).executions.length, count);
    await page.screenshot({ path: path.join(root, `chat-${mode}-empty-stream.png`), animations: 'disabled' });
  } finally { await page.unroute(eventRoute); }
  return { mode, id: run.id, continuation_reused_execution: true, empty_stream_restores_answer: true };
}

async function chat(context) {
  await context.api('POST', '/api/agents/resources/prompts', { name: 'ui-chat-agent',
    files: [{ path: 'soul.md', content_b64: Buffer.from('Reply directly without tools, files or network.').toString('base64') }] });
  await context.api('POST', '/api/agents', { name: 'ui-chat-agent', current: { prompt: 'ui-chat-agent' } });
  const conversations = [];
  for (const mode of ['operator', 'agent']) conversations.push(await conversation(context, mode));
  fs.writeFileSync(path.join(context.root, 'chat-ui.json'), JSON.stringify({ conversations }, null, 2));
}

module.exports = { chat };
