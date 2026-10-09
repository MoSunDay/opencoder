// Read-only acceptance using an existing Brain execution. Optional upstream
// serves the real API while checking the candidate SPA in a local Vite server.
// node scripts/acceptance/brain/ui/run-canvas.js BASE OUTPUT RUN TOKEN_FILE [UPSTREAM]
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const { chromium } = require('../../../../crates/web/spa/node_modules/playwright-core');

async function candidateProxy(base, upstream) {
  if (!upstream) return { origin: base, close: () => {} };
  const server = http.createServer((request, response) => {
    const target = new URL(request.url, request.url.startsWith('/api/') ? upstream : base);
    const proxy = http.request(target, { method: request.method, headers: { ...request.headers, host: target.host } }, (incoming) => {
      response.writeHead(incoming.statusCode, incoming.headers); incoming.pipe(response);
    });
    proxy.on('error', () => { if (!response.headersSent) response.writeHead(502); response.end(); });
    response.on('close', () => proxy.destroy()); request.pipe(proxy);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return { origin: `http://127.0.0.1:${server.address().port}`, close: () => { server.closeAllConnections(); server.close(); } };
}

async function main() {
  const [base, output, runId, tokenFile, upstream, onlyWidth] = process.argv.slice(2);
  assert(base && output && runId && tokenFile, 'BASE OUTPUT RUN TOKEN_FILE required');
  fs.mkdirSync(output, { recursive: true, mode: 0o700 });
  const token = fs.readFileSync(tokenFile, 'utf8').trim();
  const candidate = await candidateProxy(base, upstream);
  const browser = await chromium.launch({ args: ['--no-sandbox', '--no-proxy-server', '--disable-dev-shm-usage'] });
  const checks = [], errors = [];
  try {
    for (const width of onlyWidth ? [Number(onlyWidth)] : [1920, 1280, 768, 390]) {
      const page = await browser.newPage({ viewport: { width, height: 1000 } });
      page.setDefaultTimeout(30000);
      page.on('pageerror', (error) => errors.push(error.message));
      const detailReads = new Set();
      await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
      await page.route('**/api/**', async (route) => {
        const request = route.request(), url = new URL(request.url());
        if (!['GET', 'HEAD'].includes(request.method())) {
          errors.push(`Unexpected write: ${url.pathname}`); return route.abort();
        }
        if (/^\/api\/executions\/[^/]+$/.test(url.pathname)) detailReads.add(decodeURIComponent(url.pathname.split('/').at(-1)));
        return route.continue();
      });
      try {
        await page.goto(`${candidate.origin}/?brain_run=${encodeURIComponent(runId)}`, { waitUntil: 'domcontentloaded' });
        const workspace = page.getByRole('region', { name: '大脑执行画布' });
        await workspace.locator('.react-flow__node-layer').first().waitFor();
        await workspace.evaluate((element) => {
          window.canvasTrace = [];
          const viewport = element.querySelector('.react-flow__viewport');
          new MutationObserver(() => {
            window.canvasTrace.push({ style: viewport.getAttribute('style'),
              canvas: element.querySelector('.brain-milestone-canvas').getBoundingClientRect().toJSON() });
            if (window.canvasTrace.length > 100) window.canvasTrace.shift();
          }).observe(viewport, { attributes: true, attributeFilter: ['style'] });
        });
        await page.waitForTimeout(700);
        const geometry = await workspace.boundingBox();
        assert(Math.abs(geometry.y + geometry.height - 984) <= 3, `canvas does not fill viewport at ${width}`);
        assert(geometry.width >= width - (width >= 768 ? 260 : 32), `canvas too narrow at ${width}`);
        assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), 'page overflow');
        await page.screenshot({ path: path.join(output, `canvas-${width}.png`) });
        await page.getByRole('button', { name: '全屏画布', exact: true }).click();
        const full = await workspace.boundingBox();
        assert.deepEqual(full, { x: 0, y: 0, width, height: 1000 });
        await page.waitForTimeout(500);
        await page.getByRole('button', { name: '查看全图', exact: true }).click();
        await page.waitForTimeout(350);
        await page.screenshot({ path: path.join(output, `fullscreen-${width}.png`) });
        const snapshot = await page.evaluate(async ({ token, runId }) => {
          const response = await fetch(`/api/brain/runs/${encodeURIComponent(runId)}/layered`, { headers: { Authorization: `Bearer ${token}` } });
          if (!response.ok) throw new Error('Cannot read Brain run');
          return response.json();
        }, { token, runId });
        const opened = [];
        for (const layer of snapshot.plan.layers) {
          const nodeIds = snapshot.plan.nodes.filter((node) => node.layer_id === layer.layer_id).map((node) => node.node_id);
          const latest = snapshot.operations.filter((op) => nodeIds.includes(op.node_id))
            .sort((a, b) => b.activation - a.activation)[0];
          assert(latest?.execution_id, `acceptance requires a dispatched layer: ${layer.layer_id}`);
          const card = workspace.locator(`[data-id="layer:${layer.layer_id}"]`);
          await card.locator('header').click();
          const drawer = page.getByRole('dialog').filter({ has: page.getByRole('combobox', { name: '选择层执行记录' }) });
          await drawer.waitFor();
          await drawer.locator('.execution-view-full').waitFor();
          if (latest.execution_kind === 'dag') await drawer.locator('.dag-detail-graph .react-flow__node').first().waitFor();
          await drawer.getByText(latest.execution_id, { exact: true }).first().waitFor();
          await page.waitForFunction(() => [...document.querySelectorAll('.ant-drawer-open .ant-drawer-content-wrapper')].every((element) => {
            const box = element.getBoundingClientRect();
            return box.left >= -1 && box.right <= innerWidth + 1 && element.getAnimations({ subtree: true }).every((animation) => animation.playState !== 'running');
          }));
          assert(detailReads.has(latest.execution_id), 'drawer did not read the actual child execution');
          if (!opened.length) await page.screenshot({ path: path.join(output, `layer-detail-${width}.png`) });
          opened.push(latest.execution_id);
          await drawer.locator('.ant-drawer-close').click();
          await drawer.waitFor({ state: 'hidden' });
          await page.waitForTimeout(350);
        }
        await page.getByRole('button', { name: '退出全屏', exact: true }).click();
        assert(!await workspace.evaluate((element) => element.classList.contains('is-fullscreen')));
        checks.push({ width, geometry, opened, fullscreen: true });
      } catch (error) {
        const geometry = await page.locator('.brain-milestone-canvas').first().evaluate((element) => ({
          trace: window.canvasTrace,
          viewport: element.querySelector('.react-flow__viewport')?.getAttribute('style'),
          nodes: [...element.querySelectorAll('.react-flow__node')].map((node) => ({
            id: node.dataset.id, style: node.getAttribute('style'), rect: node.getBoundingClientRect().toJSON(),
          })),
        })).catch(() => null);
        fs.writeFileSync(path.join(output, `failure-${width}.json`), JSON.stringify(geometry, null, 2));
        await page.screenshot({ path: path.join(output, `failure-${width}.png`) }).catch(() => {});
        throw error;
      } finally { await page.unrouteAll({ behavior: 'ignoreErrors' }); await page.close(); }
    }
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify({ verified: true, runId, checks, errors }, null, 2));
    console.log(JSON.stringify({ verified: true, widths: checks.map((check) => check.width) }));
  } finally { await browser.close(); candidate.close(); }
}
main().catch((error) => { console.error(String(error.message).replace(/Bearer\s+[^\s'"\\]+/g, 'Bearer [redacted]')); process.exitCode = 1; });
