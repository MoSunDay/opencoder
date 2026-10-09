const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

async function chat({ page, api, root, until }) {
  await page.getByRole('menuitem', { name: /Agent$/ }).click();
  await page.getByText('Operator 模式', { exact: true }).click();
  const sender = page.getByPlaceholder('输入提示词，Enter 发送，Shift+Enter 换行');
  assert(await sender.isDisabled(), 'chat cannot send without a selected node');
  await page.getByRole('combobox', { name: '执行节点', exact: true }).click();
  await page.locator('.ant-select-item-option-content').filter({ hasText: 'node-a' }).click();
  await sender.fill('ui-chat-first-prompt');
  const accepted = page.waitForResponse((response) => response.url().endsWith('/api/sessions') && response.request().method() === 'POST');
  await sender.press('Enter');
  const run = await (await accepted).json();
  assert(run.id);
  await page.getByText('browser node-owned answer', { exact: true }).first().waitFor();
  await until(async () => ['idle', 'done'].includes((await api('GET', `/api/executions/${run.id}`)).execution.status), 'chat first reply completes');
  await page.locator('.ant-sender-actions-btn-loading-button').waitFor({ state: 'hidden' });
  const count = (await api('GET', '/api/executions')).executions.length;
  await sender.fill('ui-chat-follow-up');
  const continued = page.waitForResponse((response) => response.url().endsWith(`/api/sessions/${run.id}/prompt`) && response.request().method() === 'POST');
  await sender.press('Enter');
  assert((await continued).ok());
  await page.getByText('ui-chat-follow-up', { exact: true }).first().waitFor();
  await until(async () => ['idle', 'done'].includes((await api('GET', `/api/executions/${run.id}`)).execution.status), 'chat continuation completes');
  await page.locator('.ant-sender-actions-btn-loading-button').waitFor({ state: 'hidden' });
  assert.equal((await api('GET', '/api/executions')).executions.length, count);
  await page.screenshot({ path: path.join(root, 'chat-continuation.png'), animations: 'disabled' });
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
    await sender.fill('ui-chat-recover-saved-answer');
    const continued = page.waitForResponse((response) => response.url().endsWith(`/api/sessions/${run.id}/prompt`) && response.request().method() === 'POST');
    await sender.press('Enter');
    assert((await continued).ok());
    await until(async () => await page.getByText('browser node-owned answer', { exact: true }).count() === 3, 'empty stream restores the third saved reply');
    assert.equal(await page.locator('.ant-spin-spinning').count(), 0, 'chat must leave the waiting state');
    assert.equal(await page.locator('.ant-sender-actions-btn-loading-button').count(), 0, 'chat composer must allow the next input');
    assert.equal((await api('GET', '/api/executions')).executions.length, count);
    await page.screenshot({ path: path.join(root, 'chat-empty-stream.png'), animations: 'disabled' });
  } finally { await page.unroute(eventRoute); }
  fs.writeFileSync(path.join(root, 'chat-ui.json'), JSON.stringify({ id: run.id, continuation_reused_execution: true, empty_stream_restores_answer: true }, null, 2));
}

module.exports = { chat };
