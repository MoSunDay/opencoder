// Read-only browser acceptance for milestone transition labels and routes.
// Usage: node label-boundary.js CONFIG PLAN_ID VERSION PLAN_TITLE EVIDENCE [SPA_DIST]
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

const [configPath, planId, version, title, evidence, dist] = process.argv.slice(2);
assert(configPath && planId && version && title && evidence, 'CONFIG PLAN_ID VERSION PLAN_TITLE EVIDENCE required');
const settings = JSON.parse(fs.readFileSync(configPath)).deployment;
const token = fs.readFileSync(settings.token_file, 'utf8').trim();
fs.mkdirSync(evidence, { recursive: true, mode: 0o700 });
const longForward = '前进之前必须检查完整证据、输入输出和每个能力的执行结果。'.repeat(50).slice(0, 1024);
const longReturn = '若第二阶段存在任何失败或证据缺口，应回退到第一阶段重新处理并记录复盘原因。'.repeat(50).slice(0, 1024);
const intersects = (a, b) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;

async function openPlan(browser, includeRetry) {
  const page = await browser.newPage({ viewport: { width: 1650, height: 1100 } });
  await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
  if (dist) for (const name of ['app.js', 'app.css']) {
    await page.route(`**/static/${name}`, (route) => route.fulfill({ path: path.join(dist, 'static', name),
      contentType: name.endsWith('.js') ? 'text/javascript' : 'text/css' }));
  }
  const routePath = `/api/brain/plan-defs/${encodeURIComponent(planId)}/versions/${version}`;
  await page.route(`**${routePath}`, async (route) => {
    const response = await route.fetch(); assert(response.ok(), `${routePath}: ${response.status()}`);
    const data = await response.json(); assert(data.plan.layers.length >= 2);
    const [first, second] = data.plan.layers;
    const forward = data.plan.transitions.find((edge) => edge.from === first.layer_id && edge.to === second.layer_id);
    const backward = data.plan.transitions.find((edge) => edge.from === second.layer_id && edge.to === first.layer_id);
    assert(forward && backward, 'reference plan needs both directions');
    forward.condition = longForward; backward.condition = longReturn;
    if (includeRetry) data.plan.transitions.push({ from: second.layer_id, to: second.layer_id, condition: '本层重新验证' });
    await route.fulfill({ json: data });
  });
  await page.goto(settings.public_url, { waitUntil: 'domcontentloaded' });
  await page.getByRole('radiogroup').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: '大脑调度' }).click();
  await page.getByRole('tab', { name: '计划库' }).click();
  await page.getByPlaceholder('搜索计划名称').fill(title);
  await page.getByText(title, { exact: true }).click();
  await page.locator('.brain-milestone-preview .react-flow__edge-text').first().waitFor();
  return page;
}

async function measure(page, label, count) {
  const labels = await page.locator('.react-flow__edge-text').evaluateAll((nodes) => nodes.map((node) => ({
    text: node.textContent, ...({ left: node.getBoundingClientRect().left, right: node.getBoundingClientRect().right,
      top: node.getBoundingClientRect().top, bottom: node.getBoundingClientRect().bottom }) })));
  const nodes = await page.locator('.react-flow__node-layer').evaluateAll((items) => items.map((node) => {
    const box = node.getBoundingClientRect(); return { left: box.left, right: box.right, top: box.top, bottom: box.bottom };
  }));
  const canvas = await page.locator('.brain-milestone-preview .react-flow, .brain-method-editor .react-flow').first().boundingBox();
  await page.screenshot({ path: path.join(evidence, `${label}.png`), animations: 'disabled' });
  assert.equal(labels.length, count, `${label}: edge count`);
  assert(labels.every((item) => item.text.length <= 15), `${label}: label not shortened`);
  for (let i = 0; i < labels.length; i++) {
    const item = labels[i];
    assert(item.left >= canvas.x - 1 && item.right <= canvas.x + canvas.width + 1, `${label}: label clipped`);
    // The short retry badge sits on the left border; other labels stay clear of cards.
    assert(nodes.every((node) => !intersects(item, node) ||
      (item.text === '重试' && item.right <= node.left + 8)), `${label}: label covers a milestone`);
    for (let j = i + 1; j < labels.length; j++) assert(!intersects(item, labels[j]), `${label}: labels overlap`);
  }
  return { label, labels, canvas };
}

async function inspectPreview(browser, includeRetry, samples) {
  const page = await openPlan(browser, includeRetry);
  try {
    const count = includeRetry ? 3 : 2;
    for (const width of [1650, 390, 320]) {
      await page.setViewportSize({ width, height: width === 1650 ? 1100 : 844 });
      await page.waitForTimeout(600);
      await page.locator('.brain-milestone-preview .react-flow__controls-fitview').click();
      await page.waitForTimeout(300);
      samples.push(await measure(page, `${includeRetry ? 'retry' : 'return'}-${width}`, count));
    }
    if (includeRetry) {
      const length = await page.locator('.brain-milestone-preview .react-flow__edge-path').last()
        .evaluate((edge) => edge.getTotalLength());
      assert(length > 100, 'self-retry edge collapsed to a stub');
    }
  } finally { await page.close(); }
}

async function inspectEditor(browser, samples) {
  const page = await openPlan(browser, false);
  try {
    await page.getByRole('button', { name: '关闭', exact: true }).first().click();
    await page.getByRole('row').filter({ hasText: title }).getByRole('button', { name: '创建下一版本' }).click();
    await page.locator('.brain-method-editor .react-flow__edge').last().waitFor();
    samples.push(await measure(page, 'editor-before-drag', 2));
    const edge = page.locator('.brain-method-editor .react-flow__edge').last();
    const point = await edge.locator('.react-flow__edge-path').evaluate((element) => {
      const at = element.getPointAtLength(element.getTotalLength() * 0.25).matrixTransform(element.getScreenCTM());
      return { x: at.x, y: at.y };
    });
    await page.mouse.click(point.x, point.y);
    assert.equal((await page.getByLabel('扭转条件', { exact: true }).inputValue()).length, 1024);
    assert.equal((await edge.getAttribute('title')).length, 1024);
    const header = page.locator('.brain-method-editor .react-flow__node-layer').nth(1).locator('.brain-layer-box header strong');
    const box = await header.boundingBox();
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.mouse.down();
    await page.mouse.move(box.x + box.width / 2 + 210, box.y + box.height / 2 + 35, { steps: 18 });
    await page.mouse.up();
    await page.waitForTimeout(400);
    samples.push(await measure(page, 'editor-after-drag', 2));
  } finally { await page.close(); }
}

(async () => {
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(),
    args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  try {
    const samples = [];
    await inspectPreview(browser, false, samples);
    await inspectPreview(browser, true, samples);
    await inspectEditor(browser, samples);
    const result = { result: 'PASS', plan_id: planId, version, samples };
    fs.writeFileSync(path.join(evidence, 'result.json'), JSON.stringify(result, null, 2), { mode: 0o600 });
    console.log(JSON.stringify({ result: 'PASS', evidence, samples: samples.length }));
  } finally { await browser.close(); }
})().catch((error) => { console.error(error); process.exitCode = 1; });
