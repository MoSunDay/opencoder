const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

async function schedules({ page, api, root, until }) {
  const definition = await api('POST', '/api/dag/defs', { spec: { name: 'ui-scheduled-native', steps: [{ name: 'output', kind: { type: 'binary', resource: 'stdout' } }] } });
  await page.getByRole('menuitem', { name: '定时任务' }).click();
  await page.getByRole('button', { name: '新建任务', exact: true }).click();
  const modal = page.locator('.ant-modal');
  await modal.getByLabel('cron 表达式').fill('0 3 * * *');
  await modal.getByLabel('类型', { exact: true }).click();
  await page.locator('.ant-select-item-option-content').getByText('DAG', { exact: true }).click();
  await modal.getByLabel('schedule_target').fill(definition.id);
  await modal.getByLabel('schedule_params').fill('["schedule-input"]');
  await modal.getByRole('switch').click();
  const created = page.waitForResponse((response) => response.url().endsWith('/api/schedules') && response.request().method() === 'POST');
  await modal.getByRole('button', { name: /^保\s*存$/ }).click();
  const receipt = await (await created).json();
  assert(receipt.id);
  const row = page.getByRole('row').filter({ hasText: receipt.id });
  await row.getByRole('button', { name: /^编\s*辑$/ }).click();
  assert.equal(await modal.getByLabel('schedule_target').inputValue(), definition.id);
  assert.deepEqual(JSON.parse(await modal.getByLabel('schedule_params').inputValue()), ['schedule-input']);
  assert.equal(await modal.getByLabel('时区').inputValue(), '+08:00');
  await modal.getByLabel('schedule_params').fill('["edited-input"]');
  const saved = page.waitForResponse((response) => response.url().endsWith(`/api/schedules/${receipt.id}`) && response.request().method() === 'PUT');
  await modal.getByRole('button', { name: /^保\s*存$/ }).click();
  assert((await saved).ok());
  await row.getByRole('button', { name: /^启\s*用$/ }).click();
  await until(async () => (await api('GET', '/api/schedules')).schedules.find((item) => item.id === receipt.id).enabled, 'schedule enabled');
  await row.getByRole('button', { name: /^停\s*用$/ }).click();
  await until(async () => !(await api('GET', '/api/schedules')).schedules.find((item) => item.id === receipt.id).enabled, 'schedule disabled');
  // The write can reach the server before the browser replaces the old row.
  // Open the confirmation only after the refreshed disabled state is visible.
  await row.getByRole('button', { name: /^启\s*用$/ }).waitFor({ state: 'visible' });
  await row.getByRole('button', { name: '立即触发', exact: true }).click();
  await page.getByRole('button', { name: '确认触发', exact: true }).click();
  let run;
  await until(async () => {
    run = (await api('GET', `/api/schedules/${receipt.id}/runs`)).runs.find((item) => item.execution_id);
    return run;
  }, 'manual schedule recorded');
  await until(async () => (await api('GET', `/api/executions/${run.execution_id}`)).execution.status === 'done', 'manual native schedule completes', 120000);
  await row.getByRole('button', { name: '触发历史', exact: true }).click();
  await page.getByRole('button', { name: run.execution_id, exact: true }).click();
  const executionDetail = page.locator('.oc-execution-detail');
  const canvas = executionDetail.locator('.dag-detail-graph');
  const table = executionDetail.locator('[aria-label="DAG 运行环境"] .ant-table-wrapper');
  await canvas.waitFor();
  await table.waitFor();
  assert.equal(await executionDetail.getByText('全部步骤共享本次 DAG 容器', { exact: true }).count(), 0);
  const canvasBounds = await canvas.boundingBox();
  const tableBounds = await table.boundingBox();
  assert(canvasBounds && tableBounds && canvasBounds.y + canvasBounds.height <= tableBounds.y,
    'execution canvas appears above the pinned resource table');
  await page.screenshot({ path: path.join(root, 'schedule-execution-detail.png'), animations: 'disabled' });
  await page.locator('.oc-execution-detail .ant-drawer-close').click();
  await page.locator('.ant-drawer-open .ant-drawer-close').click();
  await row.getByRole('button', { name: /^删\s*除$/ }).click();
  await page.getByRole('button', { name: '确认删除', exact: true }).click();
  await until(async () => !(await api('GET', '/api/schedules')).schedules.some((item) => item.id === receipt.id), 'schedule removed');
  assert((await api('GET', `/api/schedules/${receipt.id}/runs`)).runs.some((item) => item.execution_id === run.execution_id));
  fs.writeFileSync(path.join(root, 'schedule-ui.json'), JSON.stringify({ schedule_id: receipt.id, run, history_retained: true }, null, 2));
}

module.exports = { schedules };
