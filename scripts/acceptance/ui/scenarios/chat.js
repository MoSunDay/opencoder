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
  const count = (await api('GET', '/api/executions')).executions.length;
  await sender.fill('ui-chat-follow-up');
  const continued = page.waitForResponse((response) => response.url().endsWith(`/api/sessions/${run.id}/prompt`) && response.request().method() === 'POST');
  await sender.press('Enter');
  assert((await continued).ok());
  await page.getByText('ui-chat-follow-up', { exact: true }).first().waitFor();
  await until(async () => ['idle', 'done'].includes((await api('GET', `/api/executions/${run.id}`)).execution.status), 'chat continuation completes');
  assert.equal((await api('GET', '/api/executions')).executions.length, count);
  await page.screenshot({ path: path.join(root, 'chat-continuation.png'), animations: 'disabled' });
  fs.writeFileSync(path.join(root, 'chat-ui.json'), JSON.stringify({ id: run.id, continuation_reused_execution: true }, null, 2));
}

module.exports = { chat };
