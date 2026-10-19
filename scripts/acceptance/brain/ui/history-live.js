// Read-only verification against real APIs and execution-owned results.
// node history-live.js BASE OUTPUT TOKEN_FILE_OR_DASH RUN_ID_OR_DASH COMMIT_OR_DASH [PROXY]
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { chromium } = require('../../../../crates/web/spa/node_modules/playwright-core');

function nextDownload(page) {
  const context = page.context(), pages = new Set(context.pages());
  let timer, receive;
  const promise = new Promise((resolve, reject) => {
    timer = setTimeout(() => reject(new Error('Artifact download timed out')), 40000);
    receive = (download) => { clearTimeout(timer); resolve(download); };
  });
  const opened = (child) => { pages.add(child); child.on('download', receive); };
  for (const child of pages) child.on('download', receive);
  context.on('page', opened);
  return { promise, close: () => {
    clearTimeout(timer); context.off('page', opened);
    for (const child of pages) child.off('download', receive);
  } };
}

async function main() {
  const [base, output, tokenFile, requested, commit, proxy] = process.argv.slice(2);
  assert(base && output && tokenFile && requested && commit, 'BASE OUTPUT TOKEN RUN COMMIT required');
  fs.mkdirSync(output, { recursive: true, mode: 0o700 });
  const token = tokenFile === '-' ? '' : fs.readFileSync(tokenFile, 'utf8').trim();
  const prefix = new URL(base).pathname.replace(/\/$/, '');
  const browser = await chromium.launch({ args: ['--no-sandbox', '--disable-dev-shm-usage'],
    ...(proxy ? { proxy: { server: proxy, bypass: '127.0.0.1,localhost' } } : {}) });
  const errors = [], checks = [], downloads = [];
  let selected;
  const capture = async (page, name) => {
    const geometry = await page.evaluate(() => ({ width: innerWidth, outer: document.documentElement.scrollWidth,
      panes: [...document.querySelectorAll('.fleet-content,.brain-detail-scroll')].filter((e) => e.clientWidth)
        .map((e) => ({ width: e.clientWidth, scroll: e.scrollWidth })) }));
    assert(geometry.outer <= geometry.width + 1, `${name}: outer overflow`);
    assert(geometry.panes.every((e) => e.scroll <= e.width + 1), `${name}: pane overflow`);
    await page.screenshot({ path: path.join(output, `${name}.png`), animations: 'disabled' });
    checks.push({ name, ...geometry });
  };
  try {
    for (const width of [1920, 1280, 768, 390]) {
      const page = await browser.newPage({ viewport: { width, height: 1000 }, acceptDownloads: true });
      page.setDefaultTimeout(40000);
      page.on('pageerror', (e) => errors.push(e.message));
      await page.addInitScript(({ token, prefix }) => {
        localStorage.setItem('oc_base', prefix); localStorage.setItem('oc_nav_page', JSON.stringify('brain'));
        if (token) localStorage.setItem('oc_token', token);
      }, { token, prefix });
      await page.route('**/api/**', (route) => {
        if (['GET', 'HEAD'].includes(route.request().method())) return route.continue();
        errors.push(`Unexpected write ${new URL(route.request().url()).pathname}`); return route.abort();
      });
      try {
        await page.goto(`${base.replace(/\/$/, '')}/?base=${encodeURIComponent(prefix)}`, { waitUntil: 'domcontentloaded' });
        const get = (route) => page.evaluate(async ({ route, prefix }) => {
          const headers = localStorage.oc_token ? { Authorization: `Bearer ${localStorage.oc_token}` } : {};
          const response = await fetch(prefix + route, { headers });
          if (!response.ok) throw new Error(`${route}: HTTP ${response.status}`);
          return response.json();
        }, { route, prefix });
        if (commit !== '-') {
          assert.equal((await get('/api/admin/release')).instance_release, `rel-${commit}`);
          for (const file of ['app.js', 'app.css', 'download-sw.js']) {
            const response = await page.request.get(`${base.replace(/\/$/, '')}/static/${file}`);
            assert.equal(response.status(), 200);
            const digest = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
            const expected = path.resolve(__dirname, '../../../../crates/web/spa/dist/static', file);
            assert.equal(digest(await response.body()), digest(fs.readFileSync(expected)), `deployed ${file}`);
          }
        }
        const history = await get('/api/brain/runs?limit=20');
        const small = await get('/api/brain/runs?limit=2');
        assert(small.runs.length <= 2, 'server must honor limit');
        if (small.next_cursor) {
          const query = new URLSearchParams({ limit: '2', cursor_id: small.next_cursor.id, cursor_created_at: small.next_cursor.created_at });
          const next = await get(`/api/brain/runs?${query}`);
          assert(next.runs.every((row) => !small.runs.some((prior) => prior.id === row.id)), 'cursor repeated records');
        }
        await page.locator('.brain-history tr[data-row-key]').first().waitFor();
        assert.equal(await page.locator('.brain-history tr[data-row-key]').count(), history.runs.length);
        for (const row of history.runs) {
          const element = page.locator('.brain-history tr').filter({ hasText: row.id });
          assert.equal(await element.locator('.brain-history-name').innerText(), row.name || '未记录计划名称');
        }
        await capture(page, `history-${width}`);
        if (!selected) {
          for (const row of requested === '-' ? history.runs : [{ id: requested }]) {
            const view = await get(`/api/brain/runs/${encodeURIComponent(row.id)}/layered`);
            if (view.schema_version === 7 && view.operations.some((op) => op.execution_id) && view.run.phase === 'completed') {
              selected = { id: row.id, view }; break;
            }
          }
        }
        assert(selected, 'a terminal schema 7 run with child executions is required');
        const { id, view } = selected;
        const listing = history.runs.find((row) => row.id === id);
        if (listing) assert.equal(listing.name, view.plan.title, 'name must match frozen plan');
        await page.goto(`${base.replace(/\/$/, '')}/?base=${encodeURIComponent(prefix)}&brain_run=${encodeURIComponent(id)}`);
        await page.getByRole('button', { name: '查看详情', exact: true }).click();
        const drawer = page.getByRole('dialog');
        await drawer.getByText('交付摘要', { exact: true }).waitFor();
        await capture(page, `overview-${width}`);
        await drawer.getByRole('tab', { name: '执行记录', exact: true }).click();
        await capture(page, `rounds-${width}`);
        if (view.run.round > 1 && view.events.some((event) => event.event_type === 'layer_started' && event.round > 1 && event.layer === 1)) {
          await drawer.getByRole('button', { name: /^第 1 轮 ·/ }).click();
          await drawer.getByText('历史达标，返工后已失效', { exact: true }).first().waitFor();
          await capture(page, `prior-round-${width}`);
          await drawer.getByRole('button', { name: new RegExp(`^第 ${view.run.round} 轮 ·`) }).click();
        }
        const operations = view.operations.filter((op) => op.execution_id && op.round === view.run.round);
        for (const op of operations.slice(0, 3)) {
          const row = drawer.locator('tr[data-row-key]').filter({ hasText: op.execution_id });
          await row.getByRole('button', { name: '查看执行详情', exact: true }).click();
          await drawer.getByRole('tab', { name: '结果', exact: true }).waitFor();
          const detail = await get(`/api/executions/${encodeURIComponent(op.execution_id)}`);
          assert.equal(detail.execution.id, op.execution_id);
          await capture(page, `child-${op.node_id}-${width}`);
          await drawer.getByRole('tab', { name: '执行过程', exact: true }).click();
          if (op.execution_kind === 'dag') await drawer.locator('.dag-detail-graph .react-flow__node').first().waitFor();
          await drawer.getByRole('tab', { name: '输入与产物', exact: true }).click();
          await drawer.getByText('本次实际输入', { exact: true }).waitFor();
          const artifacts = detail.result?.scheduler_artifacts;
          if (width === 1280 && Array.isArray(artifacts) && artifacts.length && await drawer.getByRole('button', { name: '下载文件', exact: true }).count()) {
            const artifact = artifacts[0];
            const pending = nextDownload(page);
            let download;
            try {
              [download] = await Promise.all([pending.promise,
                drawer.getByRole('button', { name: '下载文件', exact: true }).first().click()]);
            } finally { pending.close(); }
            const file = path.join(output, `artifact-${downloads.length}.bin`);
            await download.saveAs(file); const bytes = fs.readFileSync(file);
            assert.equal(bytes.length, artifact.bytes);
            assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'), artifact.sha256);
            downloads.push({ execution: artifact.execution, step: artifact.step, index: artifact.index, bytes: bytes.length, sha256: artifact.sha256 });
          }
          await drawer.getByRole('button', { name: '返回运行详情', exact: true }).click();
          assert.equal(await drawer.getByRole('tab', { name: '执行记录', exact: true }).getAttribute('aria-selected'), 'true');
        }
        await drawer.getByRole('tab', { name: '调度记录', exact: true }).click();
        await drawer.locator('.brain-event-card').first().waitFor();
        await capture(page, `events-${width}`);
        await drawer.getByRole('tab', { name: '输入与产物', exact: true }).click();
        await drawer.getByText('本次实际输入', { exact: true }).waitFor();
        const frozen = await get(`/api/executions/${encodeURIComponent(id)}`);
        const actual = frozen.request?.input?.layered_request?.inputs;
        if (actual && typeof actual.request === 'string' && actual.request.length < 200) {
          await drawer.getByText(actual.request, { exact: true }).first().waitFor();
        }
        await capture(page, `inputs-${width}`);
        await drawer.locator('.ant-drawer-close').click();
        await page.getByRole('button', { name: '返回运行历史', exact: true }).click();
        await page.getByText('第 1 页 · 每页 20 条', { exact: true }).waitFor();
      } catch (error) {
        await page.screenshot({ path: path.join(output, `failure-${width}.png`) }).catch(() => {});
        const diagnostic = {
          error: String(error),
          worker: await page.evaluate(() => navigator.serviceWorker?.controller?.scriptURL).catch(() => null),
          popups: await Promise.all(page.context().pages().filter((child) => child !== page).map(async (child) => ({
            url: child.url(), text: (await child.locator('body').innerText().catch(() => '')).slice(0, 2000),
          }))),
        };
        fs.writeFileSync(path.join(output, `failure-${width}.json`), JSON.stringify(diagnostic, null, 2));
        throw error;
      } finally { await page.unrouteAll({ behavior: 'ignoreErrors' }); await page.close(); }
    }
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify({ result: 'PASS', run_id: selected.id, commit, checks, downloads, errors }, null, 2));
    console.log(JSON.stringify({ result: 'PASS', checks: checks.length, downloads: downloads.length }));
  } finally { await browser.close(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
