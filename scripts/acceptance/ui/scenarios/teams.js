const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

function teamAnswer(request) {
  const messages = request.messages || [];
  const content = [...messages].reverse().find((message) => message.role === 'user')?.content;
  const text = typeof content === 'string' ? content : Array.isArray(content) ? content.map((part) => part.text || '').join('\n') : '';
  if (text.includes('规划下一轮讨论的核心问题')) {
    const member = text.match(/- node_id: ([^\s（]+)/);
    assert(member, 'Team plan prompt contains no member');
    return JSON.stringify({ question: '确认本次 UI 验收结论', participants: [member[1]], rationale: '核对实际任务结果' });
  }
  if (text.includes('请汇总本轮各成员的回答')) return JSON.stringify({ summary: 'ui-team-aligned', aligned: true, ambiguities: [] });
  if (text.includes('讨论判断话题是否可以收尾')) return JSON.stringify({ complete: true, next_question: null, final_summary: 'ui-team-completed' });
  if (text.includes('请自述你的能力画像')) return JSON.stringify({ capabilities: ['UI acceptance'] });
  return 'browser node-owned answer';
}

async function teams({ page, api, root, until }) {
  const capability = await api('POST', '/api/brain/capabilities', { capability_type: 'tool-usage', summary: 'UI acceptance', input_desc: 'task', output_desc: 'verified result', eng_inputs: ['verify UI'] });
  await api('PUT', `/api/brain/capabilities/${capability.capability.id}/target`, { kind: 'agent', target: 'act' });
  await page.getByRole('menuitem', { name: 'Team 组队' }).click();
  await page.getByRole('button', { name: '创建 Team', exact: true }).click();
  const modal = page.locator('.ant-modal');
  await modal.getByLabel('Team 名称').fill('ui-acceptance-team');
  await modal.getByLabel('队长', { exact: true }).click();
  await page.locator('.ant-select-item-option-content').getByText('act', { exact: true }).click();
  await modal.getByRole('button', { name: '保存 Team', exact: true }).click();
  const row = page.getByRole('row').filter({ hasText: 'ui-acceptance-team' });
  await row.getByRole('button', { name: /^编\s*辑$/ }).click();
  assert.equal(await modal.getByLabel('Team 名称').inputValue(), 'ui-acceptance-team');
  await modal.getByRole('button', { name: '保存 Team', exact: true }).click();
  const saved = (await api('GET', '/api/teams')).teams.find((team) => team.name === 'ui-acceptance-team');
  assert.equal(saved.captain, 'act');
  assert(saved.members.some((member) => member.agent === 'act'));
  await row.getByRole('button', { name: '启动 Team', exact: true }).click();
  await page.locator('.ant-drawer').getByLabel('任务要求').fill('ui-team-real-execution');
  const accepted = page.waitForResponse((response) => response.url().endsWith('/api/executions') && response.request().method() === 'POST');
  await page.locator('.ant-drawer').getByRole('button', { name: /^启\s*动$/ }).click();
  const run = await (await accepted).json();
  assert(run.id);
  await until(async () => {
    const detail = await api('GET', `/api/executions/${run.id}`);
    assert(!['error', 'cancelled', 'interrupted'].includes(detail.execution.status), JSON.stringify(detail));
    return detail.execution.status === 'done';
  }, 'real Team completes', 120000);
  await page.locator('.oc-execution-detail').getByText('ui-team-completed', { exact: true }).first().waitFor();
  await page.screenshot({ path: path.join(root, 'team-completed.png'), animations: 'disabled' });
  const detail = await api('GET', `/api/executions/${run.id}`);
  fs.writeFileSync(path.join(root, 'team-ui.json'), JSON.stringify({ id: run.id, execution: detail.execution, team: saved }, null, 2));
  await page.locator('.oc-execution-detail .ant-drawer-close').click();
}

module.exports = { teamAnswer, teams };
