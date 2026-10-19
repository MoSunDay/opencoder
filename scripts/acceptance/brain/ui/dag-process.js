// Display regression: DAG measurements must survive polling and hidden tabs.
// node scripts/acceptance/brain/ui/dag-process.js OUTPUT_DIR
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { chromium } = require('../../../../crates/web/spa/node_modules/playwright-core');
const { response, plan } = require('./history-fixtures');
const output = process.argv[2];
assert(output, 'OUTPUT_DIR required');
const dist = path.resolve('crates/web/spa/dist');
const spec = { name: 'display-check', steps: [
  { name: 'prepare', kind: { type: 'binary', resource: 'tool' } },
  { name: 'verify', kind: { type: 'binary', resource: 'tool' }, depends_on: ['prepare'] },
] };
const failure = '步骤返回了需要完整阅读的错误信息，卡片必须随内容增高。'.repeat(5);

async function main() {
  fs.mkdirSync(output, { recursive: true });
  const errors = [], checks = [];
  let progressReads = 0;
  const server = http.createServer((request, reply) => {
    const url = new URL(request.url, 'http://localhost');
    if (url.pathname.startsWith('/api/')) {
      if (request.method !== 'GET') { errors.push(`Unexpected write ${url.pathname}`); reply.writeHead(405).end(); return; }
      let body = response(url);
      if (url.pathname.endsWith('/layered')) {
        body = structuredClone(body);
        Object.assign(body.operations[2], { execution_id: 'dag-check', execution_kind: 'dag' });
      }
      if (url.pathname === '/api/executions/dag-check') body = {
        execution: { id: 'dag-check', kind: 'dag', status: 'error', created_at: 1 }, definition: { spec },
        request: { kind: 'dag', input: {} }, result: { scheduler_output: { summary: '检查结果' } },
      };
      if (url.pathname === '/api/dag/runs/dag-check/progress') {
        progressReads++;
        body = { execution_status: 'error', head_seq: 2, steps: [
          { name: 'prepare', status: 'done' }, { name: 'verify', status: 'error', error: failure },
        ] };
      }
      if (body === undefined) { errors.push(`Unexpected read ${url.pathname}`); reply.writeHead(404).end(); return; }
      reply.writeHead(200, { 'content-type': 'application/json' }).end(JSON.stringify(body)); return;
    }
    const file = path.join(dist, url.pathname === '/' ? 'index.html' : url.pathname);
    if (!file.startsWith(`${dist}/`) || !fs.existsSync(file)) { reply.writeHead(404).end(); return; }
    const mime = file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html';
    reply.writeHead(200, { 'content-type': mime }); fs.createReadStream(file).pipe(reply);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const waitForRefresh = async (page) => {
    const before = progressReads, deadline = Date.now() + 15000;
    while (progressReads < before + 2 && Date.now() < deadline) await page.waitForTimeout(100);
    assert(progressReads >= before + 2, 'progress polling stopped');
  };
  try {
    for (const width of [1920, 1280, 768, 390]) {
      const page = await browser.newPage({ viewport: { width, height: 1000 } });
      page.setDefaultTimeout(10000);
      page.on('pageerror', (error) => errors.push(error.message));
      try {
        await page.addInitScript(() => localStorage.setItem('oc_nav_page', JSON.stringify('brain')));
        await page.goto(`http://127.0.0.1:${server.address().port}/`);
        await page.getByRole('button', { name: plan.title, exact: true }).first().click();
        await page.getByRole('button', { name: '查看详情', exact: true }).click();
        const drawer = page.getByRole('dialog');
        await drawer.getByRole('tab', { name: '执行记录', exact: true }).click();
        await drawer.locator('tr[data-row-key="op-3"]').getByRole('button', { name: '查看执行详情', exact: true }).click();
        await drawer.getByRole('tab', { name: '执行过程', exact: true }).click();
        const graph = drawer.locator('.dag-detail-graph');
        const inspect = async (stage) => {
          await drawer.getByRole('tabpanel', { name: '执行过程', exact: true }).waitFor();
          assert.equal(await drawer.getByRole('tab', { name: '执行过程', exact: true }).getAttribute('aria-selected'), 'true');
          await page.waitForFunction(() => [...document.querySelectorAll('.ant-drawer .ant-tabs-nav')]
            .every((element) => element.getAnimations({ subtree: true }).every((animation) => animation.playState !== 'running')));
          const nodes = graph.locator('.react-flow__node');
          assert.equal(await nodes.count(), 2);
          for (const node of await nodes.all()) await node.waitFor({ state: 'visible' });
          await graph.locator('.react-flow__edge').waitFor();
          const geometry = await graph.evaluate((element) => ({
            width: innerWidth, outer: document.documentElement.scrollWidth,
            nodes: [...element.querySelectorAll('.react-flow__node')].map((node) => ({
              id: node.dataset.id, width: node.offsetWidth, height: node.offsetHeight,
              cardHeight: node.querySelector('.dag-node').offsetHeight,
            })),
            edges: [...element.querySelectorAll('.react-flow__edge')].map((edge) => {
              const line = edge.querySelector('.react-flow__edge-path');
              const endpoint = line.getPointAtLength(line.getTotalLength()).matrixTransform(line.getScreenCTM());
              const handle = element.querySelector('[data-id="verify"] .target').getBoundingClientRect();
              return { targetY: endpoint.y, handleY: handle.y + handle.height / 2 };
            }),
          }));
          assert(geometry.outer <= width + 1, 'page overflow');
          const failed = geometry.nodes.find((node) => node.id === 'verify');
          assert(failed.height > 52 && Math.abs(failed.height - failed.cardHeight) <= 1, 'error card height clamped');
          assert(geometry.edges.every((edge) => Math.abs(edge.targetY - edge.handleY) <= 2), 'edge detached from resized card');
          await page.screenshot({ path: path.join(output, `${stage}-${width}.png`) });
          checks.push({ stage, ...geometry });
        };
        await graph.locator('.dag-node--error').waitFor();
        await inspect('initial');
        await waitForRefresh(page);
        await inspect('refreshed');
        await drawer.getByRole('tab', { name: '结果', exact: true }).click();
        await waitForRefresh(page);
        await drawer.getByRole('tab', { name: '执行过程', exact: true }).click();
        await inspect('reopened');
      } catch (error) {
        await page.screenshot({ path: path.join(output, `failure-${width}.png`) }).catch(() => {});
        fs.writeFileSync(path.join(output, 'failure.json'), JSON.stringify({ error: String(error), checks, errors }, null, 2));
        throw error;
      } finally { await page.close(); }
    }
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify({ result: 'PASS', scope: 'synthetic DAG display', checks, errors }, null, 2));
    console.log(JSON.stringify({ result: 'PASS', checks: checks.length }));
  } finally { await browser.close(); server.closeAllConnections(); server.close(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
