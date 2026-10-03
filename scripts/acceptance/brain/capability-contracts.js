const assert = require('node:assert/strict');
const path = require('node:path');

async function readTarget(page, id) {
  return page.evaluate(async (id) => {
    const response = await fetch(`/api/brain/capabilities/${id}/target`, {
      headers: { Authorization: `Bearer ${localStorage.getItem('oc_token')}` },
    });
    if (!response.ok) throw new Error(`read capability target: ${response.status}`);
    return (await response.json()).target;
  }, id);
}

async function addField(page, label, value) {
  const input = page.getByRole('combobox', { name: label, exact: true });
  await input.fill(value);
  await page.locator('.ant-select-dropdown:visible .ant-select-item-option').filter({ hasText: value }).first().waitFor();
  await input.press('Enter');
  const tag = page.locator('.ant-form-item').filter({ has: input }).locator('.ant-select-selection-item').filter({ hasText: value });
  await tag.waitFor();
  // This Select treats Tab as another selection, which would remove the tag.
  if (await input.getAttribute('aria-expanded') === 'true') await input.press('Escape');
  await tag.waitFor();
}

module.exports = async function capabilityContracts(page, artifacts) {
  await page.getByRole('tab', { name: '能力库', exact: true }).click();
  await page.getByRole('button', { name: '新建能力', exact: true }).click();
  await page.getByLabel('一句话描述', { exact: true }).fill('验证代码版本与测试结果');
  await page.getByLabel('输入描述', { exact: true }).fill('待测试的代码版本');
  await page.getByLabel('输出描述', { exact: true }).fill('是否通过和具体失败原因');
  const created = page.waitForResponse((r) => r.url().endsWith('/api/brain/capabilities') && r.request().method() === 'POST');
  await page.getByRole('button', { name: '创建能力', exact: true }).click();
  const response = await created;
  assert.equal(response.status(), 201, await response.text());
  const id = (await response.json()).capability.id;
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  assert.deepEqual(await readTarget(page, id), { kind: 'agent', target: 'act' }, 'the default execution target must be saved');

  await page.getByText('验证代码版本与测试结果', { exact: true }).click();
  await addField(page, '必填输入字段', 'revision');
  await addField(page, '必填输出字段', 'passed');
  await addField(page, '必填输出字段', 'failures');
  await page.getByRole('button', { name: /保存修改$/ }).click();
  await page.getByRole('dialog').waitFor({ state: 'hidden' });
  const expected = { kind: 'agent', target: 'act', required_inputs: ['revision'], required_outputs: ['passed', 'failures'] };
  assert.deepEqual(await readTarget(page, id), expected);

  await page.getByText('验证代码版本与测试结果', { exact: true }).click();
  const drawer = page.getByRole('dialog', { name: '编辑能力', exact: true });
  for (const field of ['revision', 'passed', 'failures']) await drawer.locator('.ant-select-selection-item').filter({ hasText: field }).waitFor();
  for (const width of [1920, 1280, 768, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await drawer.getByText('必填输出字段', { exact: true }).scrollIntoViewIfNeeded();
    await page.waitForFunction(() => [...document.querySelectorAll('.ant-drawer-open')].every((element) =>
      element.getAnimations({ subtree: true }).every((animation) => animation.playState !== 'running')));
    const bounds = await drawer.boundingBox();
    assert(bounds && bounds.x >= -1 && bounds.x + bounds.width <= width + 1, `capability drawer outside ${width}px viewport: ${JSON.stringify(bounds)}`);
    assert(await drawer.evaluate((element) => element.scrollWidth <= element.clientWidth + 1), 'capability form must not overflow');
    await page.screenshot({ path: path.join(artifacts, `capability-contract-${width}.png`), animations: 'disabled' });
  }
  await page.setViewportSize({ width: 1650, height: 1100 });
  const invalid = 'x'.repeat(129);
  await addField(page, '必填输出字段', invalid);
  await page.getByRole('button', { name: /保存修改$/ }).click();
  await drawer.getByText(/执行目标未保存，请重试/).waitFor();
  assert.deepEqual(await readTarget(page, id), expected, 'invalid fields must not overwrite the saved contract');
  const tag = drawer.locator('.ant-select-selection-item').filter({ hasText: invalid });
  await tag.locator('.ant-select-selection-item-remove').click();
  await page.getByRole('button', { name: /保存修改$/ }).click();
  await drawer.waitFor({ state: 'hidden' });
  assert.deepEqual(await readTarget(page, id), expected);
};
