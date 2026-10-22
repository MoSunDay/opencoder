import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { chromium } from '../../../crates/web/spa/node_modules/playwright-core/index.mjs';
import { checkEntities } from './entities.mjs';

const [base, output] = process.argv.slice(2);
const browser = await chromium.launch({
  executablePath: process.env.CHROME_PATH || chromium.executablePath(),
  args: ['--no-sandbox', '--disable-dev-shm-usage'],
});
const page = await browser.newPage({ viewport: { width: 1920, height: 1000 } });
const failures = [];
const measurements = [];
page.on('pageerror', (error) => failures.push(error.message));
const drawer = page.locator('.ant-drawer-open');
const selectTab = async (name) => {
  await drawer.getByRole('tab', { name, exact: true }).click();
  await drawer.getByRole('tab', { name, exact: true, selected: true }).waitFor();
};
const api = async (path, method = 'GET') => {
  const response = await page.request.fetch(`${base}/api/ontology/envs/debug${path}`, { method });
  assert(response.ok(), `${method} ${path}: ${await response.text()}`);
  return response.json();
};
const measure = async (width, tab) => {
  const dimensions = await page.evaluate(() => {
    const body = document.querySelector('.entity-detail-body');
    return {
      viewport: window.innerWidth,
      page: document.documentElement.scrollWidth,
      content: body.clientWidth,
      scroll: body.scrollWidth,
    };
  });
  assert(dimensions.page <= width + 1, `page overflow at ${width}: ${JSON.stringify(dimensions)}`);
  assert(dimensions.scroll <= dimensions.content + 1, `detail overflow: ${JSON.stringify(dimensions)}`);
  measurements.push({ width, tab, ...dimensions });
  await page.screenshot({ path: `${output}/catalog-${width}-${tab}.png`, animations: 'disabled' });
};
try {
  const entities = await checkEntities(browser, base, output);
  await page.goto(base);
  await page.locator('.fleet-sidebar [role=tablist]').getByRole('tab', { name: 'Ontology', exact: true }).click();
  await page.getByRole('menuitem', { name: /实体$/ }).click();
  await page.locator('.ant-tree').getByText('平台能力', { exact: true }).click();
  await page.getByRole('textbox', { name: '搜索名称或标识' }).fill('受理控制');
  await page.getByRole('button', { name: '受理控制', exact: true }).click();
  await drawer.getByText('此能力由平台同步维护', { exact: true }).waitFor();
  assert.equal(await drawer.getByRole('button', { name: '编辑', exact: true }).count(), 0);
  for (const width of [1920, 1280, 768, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await selectTab('基本信息');
    await drawer.getByText('opencoder-cli drain --help', { exact: true }).waitFor();
    await measure(width, 'basic');
    await selectTab('普通属性');
    await drawer.getByText('能力定义与入口', { exact: true }).waitFor();
    assert.equal(await drawer.getByRole('textbox', { name: '编辑 能力定义与入口', exact: true }).count(), 0);
    assert(await drawer.getByRole('button', { name: '提交', exact: true }).isDisabled());
    await measure(width, 'attributes');
    await selectTab('来源');
    await drawer.getByText('平台能力来源', { exact: true }).waitFor();
    assert.equal(await drawer.getByRole('button', { name: /编辑内容/ }).count(), 0);
    await measure(width, 'source');
    await selectTab('拓展信息');
    await drawer.getByRole('button', { name: /编辑内容/ }).waitFor();
    await measure(width, 'notes');
  }
  await drawer.getByRole('button', { name: /编辑内容/ }).click();
  await drawer.getByRole('textbox', { name: '补充说明正文', exact: true }).fill('# Browser note\n\nRetained after synchronization.');
  await drawer.getByRole('button', { name: /^保\s*存$/ }).click();
  await drawer.getByText('Retained after synchronization.', { exact: true }).waitFor();
  await api('/sync', 'POST');
  const entry = (await api('/capabilities?query=受理控制')).items[0];
  const detail = await api(`/entities/${entry.id}`);
  const notes = detail.text_attributes.find((item) => item.definition.attribute_role === 'ext');
  const text = await api(`/entities/${entry.id}/attributes/${notes.definition.id}/text/${notes.current.revision}`);
  assert.equal(text.content, '# Browser note\n\nRetained after synchronization.');
  assert.deepEqual(failures, []);
  const result = { passed: true, notes_preserved: true, generated_fields_readonly: true, measurements, entities };
  await writeFile(`${output}/browser.json`, JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result));
} catch (error) {
  await page.screenshot({ path: `${output}/catalog-failure.png` });
  console.error(JSON.stringify({ error: error.message, failures }));
  throw error;
} finally {
  await browser.close();
}
