// Real isolated Server: summary history entry points, initiative ordering and column layout.
// node scripts/acceptance/project_order_ui.js SERVER_BINARY SPA_DIST OUTPUT_DIR
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, execFileSync } = require('node:child_process');
const { chromium } = require('../../crates/web/spa/node_modules/playwright-core');
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function main() {
  const [binary, dist, output] = process.argv.slice(2);
  assert(binary && dist && output, 'SERVER_BINARY SPA_DIST OUTPUT_DIR required');
  fs.mkdirSync(output, { recursive: true });
  const hash = crypto.createHash('sha256');
  for (const file of fs.readdirSync(dist, { recursive: true }).filter((name) => fs.statSync(path.join(dist, name)).isFile()).sort()) hash.update(fs.readFileSync(path.join(dist, file)));
  const build = JSON.parse(execFileSync(binary, ['--build-info'], { encoding: 'utf8' }));
  assert.equal(build.spa_sha256, hash.digest('hex'), 'Server must embed the tested SPA');
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-project-order-'));
  fs.mkdirSync(path.join(root, '.opencoder'));
  fs.writeFileSync(path.join(root, 'opencoder.json'), JSON.stringify({ agent: { agents_dir: path.join(root, 'agents'), share_dir: path.join(root, 'share') } }));
  fs.writeFileSync(path.join(root, '.opencoder/schedules.json'), JSON.stringify({ schedules: [] }));
  const token = crypto.randomBytes(24).toString('hex');
  const credential = path.join(root, 'credential'); fs.writeFileSync(credential, token, { mode: 0o600 });
  const log = path.join(output, 'server.log'); const fd = fs.openSync(log, 'w');
  const server = spawn(binary, ['--web', '--workdir', root, '--data-dir', path.join(root, 'data'), '--port', '0', '--token-file', credential], { cwd: root, stdio: ['ignore', fd, fd] });
  fs.closeSync(fd);
  let browser; let page;
  try {
    let base;
    const deadline = Date.now() + 45000;
    while (!base && Date.now() < deadline) {
      assert.equal(server.exitCode, null, 'Server exited; see server.log');
      base = fs.readFileSync(log, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/)?.[1];
      if (!base) await pause(100);
    }
    assert(base, 'Server did not start');
    async function api(method, route, body) {
      const response = await fetch(base + route, { method, headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: body == null ? undefined : JSON.stringify(body) });
      const data = await response.json(); assert(response.ok, `${method} ${route}: ${JSON.stringify(data)}`); return data;
    }
    const project = await api('POST', '/api/project/goals', { title: '项目排序验收', current_summary: '项目初版' });
    await api('PATCH', `/api/project/goals/${project.id}`, { current_summary: '项目当前概述' });
    const names = ['可见专项 A：名称需要完整展示，包含关键进展、核心工作以及剩余卡点的长期维护任务', '隐藏专项', '可见专项 B'];
    const initiatives = [];
    for (const [index, title] of names.entries()) initiatives.push(await api('POST', '/api/project/initiatives', { title, goal_id: project.id, sort: index, current_summary: '专项初版' }));
    await api('PATCH', `/api/project/initiatives/${initiatives[0].id}`, { current_summary: '专项当前概述' });
    const ids = initiatives.map((row) => row.id);
    const orderPath = `/api/project/goals/${project.id}/initiatives/order`;
    const savedOrder = async () => (await api('GET', '/api/project/overview')).goals[0].initiatives.map((row) => row.id);
    browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    page = await browser.newPage({ viewport: { width: 1920, height: 1000 } });
    const errors = []; const measurements = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
    await page.goto(base, { waitUntil: 'networkidle' });
    await page.locator('.fleet-nav-category').getByText('项目', { exact: true }).click();
    const tab = (name) => page.locator('.fleet-content .ant-tabs').getByRole('tab', { name, exact: true });
    const table = (name) => page.getByLabel(name, { exact: true });
    const resetScroll = (element) => element.locator('.ant-table-content').evaluate((el) => { el.scrollLeft = 0; });
    async function history(button, expected, screenshot) {
      await button.click();
      const panel = page.getByRole('dialog', { name: '概述修改记录', exact: true });
      await panel.getByText(expected, { exact: true }).waitFor();
      assert(await panel.evaluate((el) => el.scrollWidth <= el.clientWidth + 1), 'history overflow');
      if (screenshot) await page.screenshot({ path: path.join(output, screenshot), animations: 'disabled' });
      await panel.locator('.ant-drawer-close').click(); await panel.waitFor({ state: 'hidden' });
    }
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await tab('项目').click();
      await history(table('项目表格').getByRole('button', { name: '概述历史' }), '项目初版');
      assert.equal(await page.getByRole('dialog', { name: '项目 · 项目排序验收' }).count(), 0, 'history must not open project detail');
      await resetScroll(table('项目表格'));
      await page.screenshot({ path: path.join(output, `projects-${width}.png`), animations: 'disabled' });
      await tab('专项').click();
      const list = table('专项表格');
      await resetScroll(list);
      const bounds = await list.evaluate((element) => {
        const headers = [...element.querySelectorAll('thead th')];
        const title = headers.find((el) => el.textContent === '专项').getBoundingClientRect().width;
        const project = headers.find((el) => el.textContent === '所属项目').getBoundingClientRect().width;
        const name = element.querySelector('.project-name-full');
        const actions = element.querySelector('thead .ant-table-cell-fix-end');
        return { title, project, whiteSpace: getComputedStyle(name).whiteSpace, nameFits: name.scrollWidth <= name.clientWidth + 1,
          actionsPosition: getComputedStyle(actions).position, outerFits: element.scrollWidth <= element.clientWidth + 1 };
      });
      assert(Math.abs(bounds.title - bounds.project) <= 1 && bounds.title >= 299, JSON.stringify(bounds));
      assert(bounds.nameFits && bounds.outerFits && bounds.whiteSpace === 'normal', JSON.stringify(bounds));
      if (width <= 800) assert.equal(bounds.actionsPosition, 'static', 'mobile actions must not cover the name');
      assert(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth + 1), `page overflow ${width}`);
      measurements.push({ width, ...bounds });
      await page.screenshot({ path: path.join(output, `initiatives-${width}.png`), animations: 'disabled' });
      await history(list.locator(`tr[data-row-key="${ids[0]}"]`).getByRole('button', { name: '概述历史' }), '专项初版', `history-${width}.png`);
      assert.equal(await page.getByRole('dialog', { name: `专项 · ${names[0]}` }).count(), 0);
    }
    console.log('PASS history buttons and responsive name columns');
    await page.setViewportSize({ width: 1280, height: 1000 });
    await tab('项目').click();
    await page.getByRole('button', { name: '项目排序验收', exact: true }).click();
    let drawer = page.getByRole('dialog', { name: '项目 · 项目排序验收' });
    await history(drawer.getByLabel('当前概述', { exact: true }).getByRole('button', { name: '修改记录' }), '项目初版');
    await drawer.getByRole('tab', { name: '专项', exact: true }).click();
    let inner = table('项目内专项表格');
    await history(inner.locator(`tr[data-row-key="${ids[0]}"]`).getByRole('button', { name: '概述历史' }), '专项初版');
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await resetScroll(inner);
      assert(await drawer.evaluate((el) => el.scrollWidth <= el.clientWidth + 1), `project drawer overflow ${width}`);
      await page.screenshot({ path: path.join(output, `project-detail-${width}.png`), animations: 'disabled' });
    }
    await page.setViewportSize({ width: 1280, height: 1000 });
    await inner.locator('thead').getByLabel('筛选专项', { exact: true }).click();
    await page.getByRole('textbox', { name: '搜索专项' }).fill('可见');
    await page.getByRole('button', { name: /^搜\s*索$/ }).click();
    async function drag(fromId, toId) {
      const source = inner.locator(`tr[data-row-key="${fromId}"] .project-row-drag`);
      const destination = inner.locator(`tr[data-row-key="${toId}"] .project-row-drag`);
      await source.scrollIntoViewIfNeeded();
      const start = await source.boundingBox(); const end = await destination.boundingBox();
      const response = page.waitForResponse((r) => r.url().endsWith(orderPath));
      await page.mouse.move(start.x + start.width / 2, start.y + start.height / 2); await page.mouse.down();
      await page.mouse.move(start.x + start.width / 2 + 10, start.y + start.height / 2, { steps: 3 });
      await page.mouse.move(end.x + end.width / 2, end.y + end.height / 2, { steps: 16 }); await page.mouse.up();
      const result = await response;
      await drawer.getByText('正在保存专项顺序…', { exact: true }).waitFor({ state: 'hidden' });
      return result;
    }
    assert.equal((await drag(ids[0], ids[2])).status(), 200);
    const expected = [ids[1], ids[2], ids[0]];
    assert.deepEqual(await savedOrder(), expected);
    await page.route(`**${orderPath}`, (route) => route.fulfill({ status: 500, json: { error: '排序写入失败验收' } }));
    assert.equal((await drag(ids[0], ids[2])).status(), 500);
    assert.deepEqual(await inner.locator('tr[data-row-key]').evaluateAll((rows) => rows.map((row) => row.dataset.rowKey)), [ids[2], ids[0]]);
    assert.deepEqual(await savedOrder(), expected);
    await page.unroute(`**${orderPath}`);
    console.log('PASS filtered mouse drag and failed-save rollback');
    await drawer.locator('.ant-drawer-close').click(); await drawer.waitFor({ state: 'hidden' });
    await page.getByRole('button', { name: '项目排序验收', exact: true }).click();
    await drawer.getByRole('tab', { name: '专项', exact: true }).click();
    await inner.getByRole('button', { name: '清除全部列筛选' }).click();
    assert.deepEqual(await inner.locator('tr[data-row-key]').evaluateAll((rows) => rows.map((row) => row.dataset.rowKey)), expected);
    await page.reload({ waitUntil: 'networkidle' });
    await page.getByRole('button', { name: '项目排序验收', exact: true }).click();
    await drawer.getByRole('tab', { name: '专项', exact: true }).click();
    await inner.locator('tr[data-row-key]').first().waitFor();
    assert.deepEqual(await inner.locator('tr[data-row-key]').evaluateAll((rows) => rows.map((row) => row.dataset.rowKey)), expected);
    const handle = inner.locator(`tr[data-row-key="${ids[0]}"] .project-row-drag`);
    console.log('PASS persisted order after reload');
    await handle.scrollIntoViewIfNeeded(); await handle.focus(); await handle.press('Space');
    await inner.locator('.project-row-drag[aria-pressed="true"]').waitFor();
    await pause(100);
    await handle.press('ArrowUp');
    await page.waitForFunction((id) => [...document.querySelectorAll('[id^="DndLiveRegion"]')].some((el) => el.textContent.includes(`over droppable area ${id}`)), ids[2]);
    const keyboardSave = page.waitForResponse((r) => r.url().endsWith(orderPath)); await handle.press('Space');
    assert.equal((await keyboardSave).status(), 200);
    await drawer.getByText('正在保存专项顺序…', { exact: true }).waitFor({ state: 'hidden' });
    assert.deepEqual(await savedOrder(), [ids[1], ids[0], ids[2]]);
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'receipt.json'), JSON.stringify({ result: 'PASS', server_spa_sha256: build.spa_sha256, measurements, history_entry_points: 4, persisted_drag: true, filtered_hidden_order: true, failed_save_rollback: true, refresh_persistence: true, keyboard_drag: true, page_errors: errors }, null, 2));
    console.log(`PASS: project history, initiative drag and responsive layout; ${output}`);
  } catch (error) {
    if (page) { await page.screenshot({ path: path.join(output, 'failure.png'), animations: 'disabled' }).catch(() => {}); fs.writeFileSync(path.join(output, 'failure.html'), await page.content()); }
    throw error;
  } finally {
    await browser?.close();
    if (server.exitCode === null && !server.signalCode) {
      const stopped = new Promise((resolve) => server.once('exit', resolve)); server.kill('SIGTERM');
      const force = setTimeout(() => server.kill('SIGKILL'), 10000); await stopped; clearTimeout(force);
    }
    fs.rmSync(credential, { force: true });
  }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
