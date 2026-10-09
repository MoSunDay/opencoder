// Deterministic display checks against a built SPA; no production services or data.
// Usage: node scripts/acceptance/brain/ui/layout.js OUTPUT_DIR [SPA_DIST]
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { chromium } = require('../../../../crates/web/spa/node_modules/playwright-core');

const output = process.argv[2];
assert(output, 'OUTPUT_DIR required');
const dist = path.resolve(process.argv[3] || 'crates/web/spa/dist');
const summary = '分析工程中的依赖关系并生成完整的构建验证结果。'.repeat(30);
const entry = { capability: { id: 'coding', capability_type: 'agent', summary,
  input_desc: '待分析的代码版本', output_desc: '结构化验证结果', updated_at: Date.now() }, eng_inputs: [{ content: 'revision=main' }] };
const capability = { id: 'coding', kind: 'agent', target: 'Coder', summary, input_desc: '任务', output_desc: '结果', definition: {}, version: '1' };
const version = { id: 'delivery', version: 2, changelog: '调整计划', plan: {
  schema_version: 7, title: '交付计划', objective: '交付经过验证的变更', inputs: { revision: 'main' }, max_rounds: 5,
  layers: [{ layer_id: 'code', title: '开发', task: '实现需求', objective: '完成变更', success_criteria: '测试通过' }],
  nodes: [{ node_id: 'coding', layer_id: 'code', title: '编码', objective: '实现需求', capability_id: 'coding' }],
} };
const responses = {
  '/api/me': { name: 'layout-test', role: 'admin' },
  '/api/brain/runs': { runs: [] },
  '/api/brain/plan-defs': { plans: [{ id: version.id, title: version.plan.title, latest_version: 2, schema_version: 7 }] },
  '/api/brain/plan-defs/delivery/versions/2': version,
  '/api/brain/plan-defs/delivery/versions': { versions: [version] },
  '/api/brain/library': { capabilities: [capability] },
  '/api/brain/capabilities': { capabilities: [entry] },
  '/api/brain/capabilities/coding': entry,
  '/api/brain/capabilities/coding/target': { target: { kind: 'agent', target: 'Coder' } },
  '/api/agents': { agents: [{ name: 'Coder' }] },
  '/api/brain/search': { hits: [{ capability: entry.capability, distance: 0.125 }] },
};

async function measure(page, name, results) {
  await page.waitForFunction(() => [...document.querySelectorAll('.ant-drawer-open .ant-drawer-content-wrapper')]
    .every((element) => element.getBoundingClientRect().right <= innerWidth + 1));
  await page.screenshot({ path: path.join(output, `${name}.png`), animations: 'disabled' });
  const geometry = await page.evaluate(() => {
    const pane = document.querySelector('.fleet-content');
    const summaries = [...document.querySelectorAll('.brain-capability-summary')].filter((element) => element.getBoundingClientRect().width > 0).map((element) => {
      const box = element.getBoundingClientRect();
      const cell = element.closest('td').getBoundingClientRect();
      return { width: box.width, scroll: element.scrollWidth, client: element.clientWidth,
        inside: box.left >= cell.left && box.right <= cell.right,
        ellipsis: getComputedStyle(element).textOverflow };
    });
    return { width: innerWidth, documentWidth: document.documentElement.scrollWidth,
      paneWidth: pane.clientWidth, paneScroll: pane.scrollWidth, summaries };
  });
  assert(geometry.documentWidth <= geometry.width + 1, `${name}: page overflow`);
  assert(geometry.paneScroll <= geometry.paneWidth + 1, `${name}: pane overflow`);
  for (const text of geometry.summaries) {
    assert(text.inside && text.width <= 512, `${name}: summary escaped its cell or maximum width`);
    assert(text.scroll > text.client && text.ellipsis === 'ellipsis', `${name}: long text must be truncated`);
  }
  results.push({ name, ...geometry });
  console.log(`PASS: ${name}`);
}

async function checkWidth(browser, origin, width, results, errors) {
  const page = await browser.newPage({ viewport: { width, height: 1000 } });
  page.setDefaultTimeout(15000);
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript(() => {
    localStorage.setItem('oc_token', 'isolated-layout-test');
    localStorage.setItem('oc_nav_page', JSON.stringify('brain'));
  });
  await page.route('**/api/**', async (route) => {
    const key = new URL(route.request().url()).pathname;
    if (!Object.hasOwn(responses, key)) errors.push(`Unexpected request: ${key}`);
    await route.fulfill({ status: Object.hasOwn(responses, key) ? 200 : 500, json: responses[key] || { error: key } });
  });
  try {
    await page.goto(origin);
    await page.getByRole('tab', { name: '能力库', exact: true }).click();
    await page.locator('.brain-capability-summary').waitFor();
    for (const hidden of ['输入描述', '输出描述', '工程输入']) {
      assert.equal(await page.getByRole('columnheader', { name: hidden }).count(), 0);
    }
    await measure(page, `capabilities-${width}`, results);
    await page.getByLabel('搜索能力', { exact: true }).fill('工程');
    await page.getByRole('button', { name: /^搜\s*索$/ }).click();
    await page.getByText('0.1250', { exact: true }).waitFor();
    await measure(page, `search-${width}`, results);
    await page.locator('.brain-capability-summary').click();
    await page.getByLabel('输入描述', { exact: true }).waitFor();
    await page.waitForFunction(() => document.querySelector('textarea[placeholder="一条示例输入"]')?.value === 'revision=main');
    assert.equal(await page.getByLabel('一句话描述', { exact: true }).inputValue(), summary);
    assert.equal(await page.getByLabel('输入描述', { exact: true }).inputValue(), '待分析的代码版本');
    assert.equal(await page.getByLabel('输出描述', { exact: true }).inputValue(), '结构化验证结果');
    await measure(page, `capability-detail-${width}`, results);
    await page.getByRole('button', { name: /^取\s*消$/ }).click();
    await page.getByRole('dialog').waitFor({ state: 'hidden' });
    await page.getByRole('tab', { name: '计划库', exact: true }).click();
    await page.getByRole('button', { name: '交付计划', exact: true }).click();
    await page.getByRole('dialog', { name: '修改计划', exact: true }).waitFor();
    await page.locator('.brain-method-editor .react-flow__node-layer').waitFor();
    assert.equal(await page.locator('.brain-milestone-preview').count(), 0);
    await measure(page, `edit-plan-${width}`, results);
    await page.getByRole('button', { name: '下一步：计划信息' }).click();
    assert.equal(await page.getByLabel('计划名称', { exact: true }).inputValue(), '交付计划');
    await measure(page, `plan-form-${width}`, results);
    await page.getByRole('button', { name: '返回画布', exact: true }).click();
    await page.getByRole('dialog', { name: '计划信息与提交' }).waitFor({ state: 'hidden' });
    await page.getByRole('button', { name: '关闭画布', exact: true }).click();
    await page.getByRole('dialog').waitFor({ state: 'hidden' });
    await page.getByRole('button', { name: '新建计划', exact: true }).click();
    await page.getByRole('dialog', { name: '新建计划', exact: true }).waitFor();
    await measure(page, `create-plan-${width}`, results);
  } catch (error) {
    await page.screenshot({ path: path.join(output, `failure-${width}.png`), animations: 'disabled' }).catch(() => {});
    fs.writeFileSync(path.join(output, `failure-${width}.html`), await page.content());
    throw error;
  } finally { await page.close(); }
}

async function main() {
  fs.mkdirSync(output, { recursive: true });
  const server = http.createServer((request, response) => {
    const files = { '/': ['index.html', 'text/html'], '/static/app.js': ['static/app.js', 'text/javascript'], '/static/app.css': ['static/app.css', 'text/css'] };
    const file = files[new URL(request.url, 'http://localhost').pathname];
    if (!file) { response.writeHead(404); response.end(); return; }
    response.setHeader('Content-Type', file[1]);
    fs.createReadStream(path.join(dist, file[0])).pipe(response);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  let browser;
  const results = []; const errors = [];
  try {
    browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    for (const width of [1920, 1280, 768, 390]) await checkWidth(browser, `http://127.0.0.1:${server.address().port}`, width, results, errors);
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify({ passed: true, results }, null, 2));
    console.log(`PASS: ${results.length} layout checks`);
  } finally {
    await browser?.close();
    await new Promise((resolve) => server.close(resolve));
  }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
