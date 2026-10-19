// Display-only acceptance with synthetic records. No existing services or data.
// node scripts/acceptance/brain/ui/history.js OUTPUT_DIR
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { chromium } = require('../../../../crates/web/spa/node_modules/playwright-core');
const { response, plan } = require('./history-fixtures');
const output = process.argv[2];
assert(output, 'OUTPUT_DIR required');
const dist = path.resolve('crates/web/spa/dist');

async function main() {
  fs.mkdirSync(output, { recursive: true });
  const errors = [], checks = [];
  const server = http.createServer((request, reply) => {
    const url = new URL(request.url, 'http://localhost');
    if (url.pathname.startsWith('/api/')) {
      if (request.method !== 'GET') { errors.push(`Unexpected write ${url.pathname}`); reply.writeHead(405).end(); return; }
      if (/^\/api\/executions\/[^/]+\/events$/.test(url.pathname)) {
        reply.writeHead(200, { 'content-type': 'text/event-stream' }).end('event: stream_end\ndata: {"finished":true}\n\n'); return;
      }
      const body = response(url);
      if (body === undefined) { errors.push(`Unexpected read ${url.pathname}`); reply.writeHead(404).end(); return; }
      reply.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify(body)); return;
    }
    const file = path.join(dist, url.pathname === '/' ? 'index.html' : url.pathname);
    if (!file.startsWith(`${dist}/`) || !fs.existsSync(file)) { reply.writeHead(404).end(); return; }
    const mime = file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : file.endsWith('.html') ? 'text/html' : 'application/octet-stream';
    reply.writeHead(200, { 'content-type': mime }); fs.createReadStream(file).pipe(reply);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const capture = async (page, name) => {
    await page.screenshot({ path: path.join(output, `${name}.png`), animations: 'disabled' });
    const geometry = await page.evaluate(() => ({ width: innerWidth, document: document.documentElement.scrollWidth,
      panes: [...document.querySelectorAll('.fleet-content, .brain-detail-scroll')].filter((e) => e.getBoundingClientRect().width > 0)
        .map((e) => ({ width: e.clientWidth, scroll: e.scrollWidth })),
      dialogs: [...document.querySelectorAll('.ant-drawer-open .ant-drawer-content-wrapper')].map((e) => ({ left: e.getBoundingClientRect().left, right: e.getBoundingClientRect().right })) }));
    assert(geometry.document <= geometry.width + 1, `${name}: document overflow`);
    assert(geometry.panes.every((pane) => pane.scroll <= pane.width + 1), `${name}: pane overflow`);
    assert(geometry.dialogs.every((box) => box.left >= -1 && box.right <= geometry.width + 1), `${name}: drawer outside viewport`);
    checks.push({ name, ...geometry });
  };
  try {
    for (const width of [1920, 1280, 768, 390]) {
      const page = await browser.newPage({ viewport: { width, height: 1000 } });
      page.on('pageerror', (error) => errors.push(error.message));
      await page.addInitScript(() => localStorage.setItem('oc_nav_page', JSON.stringify('brain')));
      await page.goto(`http://127.0.0.1:${server.address().port}/`);
      await page.getByRole('button', { name: plan.title, exact: true }).first().waitFor();
      await capture(page, `history-${width}`);
      await page.getByRole('button', { name: '下一页', exact: true }).click();
      await page.getByText('brain-old', { exact: true }).waitFor();
      await page.getByRole('button', { name: '查看运行', exact: true }).click();
      await page.getByRole('button', { name: '返回运行历史' }).click();
      await page.getByText('第 2 页 · 每页 20 条').waitFor();
      await page.getByRole('button', { name: '上一页', exact: true }).click();
      await page.getByRole('button', { name: plan.title, exact: true }).first().click();
      await page.getByRole('button', { name: '查看详情', exact: true }).click();
      const drawer = page.getByRole('dialog');
      await drawer.getByRole('heading', { name: '已完成', exact: true }).waitFor();
      await capture(page, `overview-${width}`);
      await drawer.getByRole('tab', { name: '执行记录', exact: true }).click();
      await drawer.getByRole('button', { name: '第 1 轮 · 1 层 · 1 项执行' }).click();
      await drawer.getByText('operator-1', { exact: true }).waitFor();
      await capture(page, `rounds-${width}`);
      await drawer.getByRole('button', { name: '查看执行详情', exact: true }).click();
      await drawer.getByText('执行输出已生成', { exact: true }).waitFor();
      await drawer.getByText('false', { exact: true }).waitFor();
      await capture(page, `child-result-${width}`);
      await drawer.getByRole('tab', { name: '执行过程', exact: true }).click();
      await drawer.getByRole('heading', { name: '会话消息' }).waitFor();
      await drawer.getByRole('tab', { name: '输入与产物', exact: true }).click();
      await drawer.getByText('report.txt', { exact: true }).waitFor();
      await capture(page, `child-inputs-${width}`);
      await drawer.getByRole('button', { name: '返回运行详情', exact: true }).click();
      assert.equal(await drawer.getByRole('button', { name: '第 1 轮 · 1 层 · 1 项执行' }).getAttribute('aria-pressed'), 'true');
      await drawer.getByRole('tab', { name: '调度记录', exact: true }).click();
      await drawer.getByRole('button', { name: '查看依据', exact: true }).first().click();
      await capture(page, `events-${width}`);
      const taskLink = drawer.getByRole('button', { name: '查看任务：operator-1', exact: true }).last();
      await taskLink.scrollIntoViewIfNeeded();
      const position = await drawer.locator('.brain-detail-scroll').evaluate((element) => element.scrollTop);
      assert(position > 100, 'Scheduling history should have a nonzero reading position');
      await taskLink.click();
      await drawer.getByText('执行输出已生成', { exact: true }).waitFor();
      await drawer.getByRole('button', { name: '返回运行详情', exact: true }).click();
      assert.equal(await drawer.locator('.brain-detail-scroll').evaluate((element) => element.scrollTop), position, 'Returning from a child should restore the scheduling history position');
      await drawer.getByRole('tab', { name: '输入与产物', exact: true }).click();
      await drawer.getByText('实际输入', { exact: true }).waitFor();
      assert.equal(await drawer.getByText('默认输入', { exact: true }).count(), 0);
      await capture(page, `inputs-${width}`);
      await page.close();
    }
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify({ verified: true, scope: 'synthetic display and navigation', checks, errors }, null, 2));
    console.log(JSON.stringify({ verified: true, screenshots: checks.length, widths: [1920, 1280, 768, 390] }));
  } finally { await browser.close(); server.closeAllConnections(); server.close(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
