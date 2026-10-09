// Isolated browser checks for the startup-script editor against a built SPA.
// node scripts/acceptance/harness/startup.js OUTPUT_DIR SPA_DIST
const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');

async function check(browser, origin, output, width) {
  const page = await browser.newPage({ viewport: { width, height: 1000 } });
  page.setDefaultTimeout(30000);
  const errors = []; const writes = []; const profiles = [];
  let defaults = { revision: 1, settings: { model: 'sample', startup_script: [] } };
  page.on('pageerror', (error) => errors.push(error.message));
  await page.addInitScript(() => {
    localStorage.setItem('oc_token', 'isolated-startup-check');
    localStorage.setItem('oc_nav_page', JSON.stringify('agents'));
  });
  await page.route('**/api/**', async (route) => {
    const request = route.request(); const key = new URL(request.url()).pathname;
    let response;
    if (request.method() === 'PUT' && key.startsWith('/api/harnesses/codex')) {
      const settings = request.postDataJSON(); writes.push({ key, settings });
      if (key === '/api/harnesses/codex') defaults = { revision: defaults.revision + 1, settings };
      else profiles.push({ name: decodeURIComponent(key.split('/').pop()), revision: 1, settings });
      response = { revision: key === '/api/harnesses/codex' ? defaults.revision : 1 };
    } else response = {
      '/api/me': { name: 'startup-test', role: 'admin' },
      '/api/agents': { agents: [] },
      '/api/nodes': { nodes: [] },
      '/api/harnesses': { harnesses: [{ name: 'codex', ...defaults }], profiles },
    }[key];
    if (!response) errors.push(`Unexpected request: ${request.method()} ${key}`);
    await route.fulfill({ status: response ? 200 : 500, json: response || { error: key } });
  });
  try {
    await page.goto(origin);
    await page.getByRole('tab', { name: 'Harness 管理', exact: true }).click();
    const editor = page.getByLabel('codex-startup-script', { exact: true });
    await editor.fill('/bin/sh\n/opt/scripts/script with 空格.sh\nliteral $(argument)');
    await page.locator('button[type=submit]').click();
    await page.getByText('配置 v2', { exact: true }).waitFor();
    assert.deepEqual(writes[0].settings, { model: 'sample', startup_script: ['/bin/sh', '/opt/scripts/script with 空格.sh', 'literal $(argument)'] });
    await page.getByRole('button', { name: '刷新配置', exact: true }).click();
    await page.waitForFunction(() => document.querySelector('[aria-label="codex-startup-script"]')?.value.includes('literal $(argument)'));
    await page.screenshot({ path: path.join(output, `editor-${width}.png`), animations: 'disabled' });
    assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `page overflow at ${width}`);
    await editor.fill('');
    await page.locator('button[type=submit]').click();
    await page.getByText('配置 v3', { exact: true }).waitFor();
    assert.deepEqual(writes[1].settings.startup_script, []);
    await page.getByRole('button', { name: '新建配置档案', exact: true }).click();
    const dialog = page.getByRole('dialog', { name: '新建配置档案', exact: true });
    await dialog.getByLabel('new-codex-profile', { exact: true }).fill('script-profile');
    await dialog.getByLabel('new-codex-profile-startup-script').fill('/opt/scripts/start.sh');
    await page.screenshot({ path: path.join(output, `create-${width}.png`), animations: 'disabled' });
    await dialog.getByRole('button', { name: /^创\s*建$/ }).click();
    await page.getByText('配置档案 script-profile 已创建，新任务将使用此版本').waitFor();
    assert.deepEqual(writes[2], { key: '/api/harnesses/codex/profiles/script-profile', settings: { model: null, startup_script: ['/opt/scripts/start.sh'] } });
    assert.equal(await editor.inputValue(), '/opt/scripts/start.sh');
    assert.deepEqual(errors, []);
    return { width, passed: true, operations: ['save', 'reload', 'clear', 'create'] };
  } catch (error) {
    await page.screenshot({ path: path.join(output, `failure-${width}.png`), animations: 'disabled', timeout: 10000 }).catch(() => {});
    fs.writeFileSync(path.join(output, `failure-${width}.json`), JSON.stringify({ errors, writes, body: await page.locator('body').innerText() }, null, 2));
    throw error;
  } finally { await page.close(); }
}

async function main() {
  const output = process.argv[2]; const dist = process.argv[3];
  assert(output && dist, 'OUTPUT_DIR and SPA_DIST are required');
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
  try {
    browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    const results = [];
    for (const width of [1920, 1280, 768, 390]) results.push(await check(browser, `http://127.0.0.1:${server.address().port}`, output, width));
    fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify({ passed: true, results }, null, 2));
    console.log(JSON.stringify({ passed: true, widths: results.length, operations: results.length * 4 }));
  } finally { await browser?.close(); await new Promise((resolve) => server.close(resolve)); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
