// Agent resource versions, managed Harness settings and node scheduling through the UI.
const assert = require('assert/strict');
const path = require('path');

module.exports = async ({ page, api, until, root, envs }) => {
  await page.locator('.ant-layout-sider').getByText('节点', { exact: true }).click();
  await page.getByRole('menuitem', { name: '节点列表' }).click();
  await page.getByRole('button', { name: '调度配置', exact: true }).click();
  await page.getByLabel('node-max-runs').fill('2');
  await page.getByLabel('node-queue-order').click();
  await page.locator('.ant-select-item-option-content').getByText('后入先出 LIFO', { exact: true }).click();
  await page.getByRole('button', { name: '保存调度配置', exact: true }).click();
  await until(async () => (await api('GET', '/api/nodes')).nodes.some((node) => node.snapshot.max_runs === 2 && node.snapshot.queue_order === 'lifo'), 'saved node scheduling');
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  await page.screenshot({ path: path.join(root, 'node-scheduling.png'), animations: 'disabled' });

  const file = (name, content) => ({ path: name, content_b64: Buffer.from(content).toString('base64') });
  await api('POST', '/api/agents/resources/prompts', { name: 'browser-prompt', files: [file('soul.md', 'Review files.')] });
  await api('PUT', '/api/agents/resources/prompts/browser-prompt', { name: 'browser-prompt', files: [file('soul.md', 'Review files.'), file('how.md', 'Inspect the task.')] });
  await api('POST', '/api/agents/resources/skills', { name: 'browser-skills', files: [file('review/SKILL.md', '# Review\nInspect files.')] });
  await api('POST', '/api/agents/resources/tools', { name: 'browser-tools', files: [file('probe.sh', '#!/bin/sh\nprintf probe')] });
  await api('POST', '/api/agents', { name: 'browser-agent', current: { prompt: 'browser-prompt', skills: 'browser-skills', tools: 'browser-tools' } });
  await page.locator('.ant-layout-sider').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: 'Agent 配置' }).click();
  await page.getByRole('tab', { name: 'Agent 列表', exact: true }).waitFor();
  const row = page.getByRole('row').filter({ hasText: 'browser-agent' });
  await row.getByText('prompts/browser-prompt/v2/', { exact: true }).waitFor();
  await row.getByText('skills/browser-skills/v1/', { exact: true }).waitFor();
  await row.getByText('tools/browser-tools/v1/', { exact: true }).waitFor();
  assert((await row.innerText()).includes('soul、how'));
  await page.screenshot({ path: path.join(root, 'agent-resources.png'), animations: 'disabled' });
  await page.getByRole('tab', { name: 'Agent Harness', exact: true }).click();
  await page.getByLabel('harness-act').waitFor();
  assert.equal(await page.locator('[aria-label^="harness-"]').count(), 8);
  await page.getByLabel('harness-browser-agent').click();
  await page.locator('.ant-select-item-option-content').getByText('Codex', { exact: true }).click();
  await until(async () => (await api('GET', '/api/agents/browser-agent/meta')).meta.harness === 'codex', 'saved agent Harness');
  await page.getByRole('tab', { name: 'Harness 管理', exact: true }).click();
  await page.getByLabel('codex-managed-envs').fill('INVALID');
  await page.getByRole('button', { name: '保存 Codex 配置', exact: true }).click();
  await page.getByText('环境变量必须为 KEY=VALUE，每行一个', { exact: true }).waitFor();
  assert.equal((await api('GET', '/api/executions')).executions.length, 0, 'invalid input must not dispatch');
  await page.screenshot({ path: path.join(root, 'invalid-environment.png'), animations: 'disabled' });
  await page.getByLabel('codex-managed-envs').fill(envs.join('\n'));
  await page.getByRole('button', { name: '保存 Codex 配置', exact: true }).click();
  await page.getByText('配置 v1', { exact: true }).waitFor();
  await page.getByRole('tab', { name: 'Harness 管理', exact: true }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: path.join(root, 'harness-management.png'), animations: 'disabled' });
};
