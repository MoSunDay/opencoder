// Submit the same named inputs and capability restrictions through the public UI.
const assert = require('node:assert/strict');
module.exports = async function launch(page, api, request) {
  await page.getByRole('radiogroup').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '大脑调度' }).click();
  await page.getByRole('button', { name: '开始新任务' }).click();
  await page.getByLabel('目标和交付物').fill(request.objective);
  const node = (await api('GET', '/api/nodes')).nodes.find((n) => n.id === request.node_id);
  await page.getByLabel('大脑所在节点').click();
  await page.locator('.ant-select-item-option-content').getByText(node.name, { exact: true }).click();
  const capabilities = (await api('GET', '/api/brain/library')).capabilities;
  for (const id of request.capability_ids) {
    const cap = capabilities.find((c) => (c.capability_id || c.id) === id);
    const label = `${cap.kind} · ${cap.target} · ${cap.summary || cap.id}`;
    await page.getByLabel('可使用的能力（可选）').fill(label);
    await page.locator('.ant-select-item-option-content').getByText(label, { exact: true }).click();
  }
  await page.getByLabel('可使用的能力（可选）').press('Escape');
  await page.getByLabel('最大调度轮次').fill(String(request.max_rounds));
  for (const [i, [name, value]] of Object.entries(request.inputs).entries()) {
    await page.getByRole('button', { name: '添加工程参数' }).click();
    await page.getByLabel('工程参数名').nth(i).fill(name);
    await page.getByLabel('工程参数值').nth(i).fill(JSON.stringify(value));
  }
  const creation = page.waitForResponse((r) => r.url().endsWith('/api/brain/runs') && r.request().method() === 'POST');
  await page.getByRole('button', { name: '开始调度' }).click();
  const response = await creation;
  assert.equal(response.status(), 202, await response.text());
  const submitted = response.request().postDataJSON();
  assert.deepEqual(submitted.inputs, request.inputs);
  assert.deepEqual(submitted.capability_ids, request.capability_ids);
  assert.equal(submitted.schema_version, 3);
  request.id = (await response.json()).run_id;
  assert.equal(submitted.id, request.id);
};
