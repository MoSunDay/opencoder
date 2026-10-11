// Real Server + browser + ctl acceptance; every write uses disposable data.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, spawnSync } = require('node:child_process');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const [bin, output] = process.argv.slice(2).map(p => path.resolve(p));
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function main() {
  assert(bin && output && !fs.existsSync(output), 'use a binary directory and a new evidence directory');
  fs.mkdirSync(output, { recursive: true });
  const work = path.join(output, 'work'); fs.mkdirSync(work);
  const token = crypto.randomBytes(24).toString('hex');
  const tokenFile = path.join(output, 'token'); fs.writeFileSync(tokenFile, token, { mode: 0o600 });
  fs.writeFileSync(path.join(work, 'opencoder.json'), JSON.stringify({
    agent: { agents_dir: path.join(output, 'agents'), nfs: { enabled: false } },
    dag: { binary_dir: path.join(output, 'binary'), workspace_dir: path.join(output, 'workspace'), nfs: { enabled: false }, workspace_nfs: { enabled: false } },
    ontology: { files_dir: path.join(output, 'ontology'), nfs: { enabled: false } },
  }));
  const net = require('node:net');
  let port;
  for (let i = 0; i < 100; i++) {
    const listener = net.createServer();
    const candidate = crypto.randomInt(5000, 9999);
    try { await new Promise((ok, fail) => listener.once('error', fail).listen(candidate, '127.0.0.1', ok));
      await new Promise(ok => listener.close(ok)); port = candidate; break;
    } catch { listener.close(); }
  }
  assert(port, 'free listener');
  const base = `http://127.0.0.1:${port}`;
  const log = fs.openSync(path.join(output, 'server.log'), 'w');
  const server = spawn(path.join(bin, 'opencoder-server'), ['--workdir', work, '--data-dir', path.join(output, 'data'), '--port', String(port), '--token', token, '--web'], { stdio: ['ignore', log, log] });
  let browser;
  const receipt = { passed: false, widths: [], errors: [] };
  async function api(method, endpoint, body) {
    const reply = await fetch(base + endpoint, { method, headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' }, body: body === undefined ? undefined : JSON.stringify(body) });
    assert(reply.ok, `${method} ${endpoint}: ${reply.status} ${await reply.clone().text()}`);
    return reply.json();
  }
  function ctl(args, code = 0) {
    const result = spawnSync(path.join(bin, 'opencoder-cli'), ['--server', base, '--token-file', tokenFile, ...args], { encoding: 'utf8', timeout: 30000 });
    assert.equal(result.status, code, result.stderr + result.stdout);
    return JSON.parse(code === 0 ? result.stdout : result.stderr);
  }
  try {
    let ready = false;
    for (let i = 0; i < 300; i++) {
      try { await api('GET', '/api/health'); ready = true; break; } catch {
        if (server.exitCode !== null) throw Error('isolated Server exited');
        await pause(200);
      }
    }
    assert(ready, 'isolated Server ready');
    const goal = await api('POST', '/api/project/goals', { title: '验收项目' });
    const initial = await api('POST', '/api/project/initiatives', { title: '验收专项', goal_id: goal.id, current_summary: '保留概述' });
    assert(!Object.hasOwn(initial, 'status'));
    const todo = await api('POST', '/api/project/todos', { title: '状态只在 TODO', draft: '验收工作', initiative_id: initial.id, board_status: 'todo' });
    ctl(['project', 'initiatives', 'patch', initial.id, '--json', JSON.stringify({ title: '验收专项', sort: 2 })]);
    ctl(['project', 'initiatives', 'patch', initial.id, '--json', '{"status":"done","title":"不可写入"}'], 4);
    const listed = ctl(['project', 'initiatives', 'list']);
    assert(!JSON.stringify(listed).includes('"status"'));
    browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    const page = await browser.newPage(); page.setDefaultTimeout(30000);
    page.on('pageerror', error => receipt.errors.push(error.message));
    await page.addInitScript(value => { if (!location.protocol.startsWith('http')) return; localStorage.setItem('oc_token', value); localStorage.setItem('oc_nav_page', JSON.stringify('project')); }, token);
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1080 });
      await page.goto(base, { waitUntil: 'networkidle' });
      await page.getByRole('tab', { name: '专项', exact: true }).click();
      const table = page.getByLabel('专项表格', { exact: true });
      await table.getByRole('button', { name: '验收专项', exact: true }).waitFor();
      assert.equal(await table.getByRole('columnheader', { name: /^状态/ }).count(), 0);
      await page.getByRole('button', { name: '新建专项', exact: true }).click();
      let drawer = page.locator('.ant-drawer:visible').last();
      assert.equal(await drawer.getByLabel('状态', { exact: true }).count(), 0);
      await drawer.getByLabel('标题', { exact: true }).fill(`新专项${width}`);
      await page.screenshot({ animations: 'disabled', path: path.join(output, `create-${width}.png`) });
      const saved = page.waitForResponse(r => r.url().endsWith('/api/project/initiatives') && r.request().method() === 'POST');
      await drawer.getByRole('button', { name: /保\s*存/ }).click();
      const reply = await saved; assert(reply.ok());
      assert(!Object.hasOwn(reply.request().postDataJSON(), 'status'));
      const created = await reply.json(); assert(!Object.hasOwn(created, 'status'));
      await page.getByPlaceholder('一句话标题').waitFor({ state: 'hidden' });
      await table.getByRole('button', { name: '验收专项', exact: true }).click();
      drawer = page.locator('.ant-drawer:visible').last();
      await drawer.getByText('状态只在 TODO', { exact: true }).waitFor();
      assert(await drawer.locator('section.project-board-lane[aria-label$="待办"]').count());
      assert(await drawer.locator('section.project-board-lane[aria-label$="已完成"]').count());
      await drawer.getByRole('button', { name: '编辑专项', exact: true }).click();
      const edit = page.getByRole('dialog', { name: '编辑专项', exact: true });
      assert.equal(await edit.getByLabel('状态', { exact: true }).count(), 0);
      await page.screenshot({ animations: 'disabled', path: path.join(output, `edit-${width}.png`) });
      const patched = page.waitForResponse(r => r.url().endsWith(`/api/project/initiatives/${initial.id}`) && r.request().method() === 'PATCH');
      await edit.getByRole('button', { name: /保\s*存/ }).click();
      const patch = await patched; assert(patch.ok()); assert(!Object.hasOwn(patch.request().postDataJSON(), 'status'));
      await edit.waitFor({ state: 'hidden' });
      const layout = await page.evaluate(() => ({ viewport: innerWidth, document: document.documentElement.scrollWidth }));
      assert(layout.document <= width + 1, JSON.stringify(layout));
      receipt.widths.push(layout);
      await page.screenshot({ animations: 'disabled', path: path.join(output, `detail-${width}.png`) });
      await api('DELETE', `/api/project/initiatives/${created.id}`);
    }
    await api('PATCH', `/api/project/todos/${todo.id}`, { board_status: 'done' });
    const overview = await api('GET', '/api/project/overview');
    assert.equal(overview.goals[0].initiatives[0].progress.done, 1);
    assert.equal(overview.goals[0].progress.done, 1);
    assert.equal(overview.goals[0].initiatives[0].current_summary, '保留概述');
    assert(!Object.hasOwn(overview.goals[0].initiatives[0], 'status'));
    assert.deepEqual(receipt.errors, []);
    receipt.passed = true;
  } finally {
    if (browser) await browser.close();
    server.kill('SIGTERM');
    await new Promise(resolve => server.exitCode !== null ? resolve() : server.once('exit', resolve));
    fs.closeSync(log); fs.unlinkSync(tokenFile);
    fs.writeFileSync(path.join(output, 'receipt.json'), JSON.stringify(receipt, null, 2));
  }
  console.log(JSON.stringify(receipt));
}
main().catch(error => { console.error(error); process.exitCode = 1; });
