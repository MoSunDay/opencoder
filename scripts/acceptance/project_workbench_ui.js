// Isolated browser acceptance over the current SPA bundle and a mutable fixture API.
const { chromium } = require('../../crates/web/spa/node_modules/playwright-core');
const { FIXTURES } = require('./spa_responsive_fixtures');
const { createServer } = require('node:http');
const { readFile, mkdir, writeFile } = require('node:fs/promises');
const path = require('node:path');
const assert = require('node:assert/strict');
const spa = path.resolve(__dirname, '../../crates/web/spa/dist');
const out = process.env.PROJECT_UI_ARTIFACTS || '/tmp/opencoder-project-ui';
const todo = (id, title, status, position, tags = []) => ({ id, title, draft: `${title}说明`, board_status: status, position, initiative_id: 'i', tag_ids: tags, created_at: 1000, updated_at: 2000 });
const overview = { goals: [{ id: 'p', title: '项目浏览器验收', status: 'active', updated_at: 2000, initiatives: [{ id: 'i', title: '专项浏览器验收', goal_id: 'p', status: 'in_progress', todos: [todo('a', '可拖动任务', 'todo', 1000, ['local', 'focus']), todo('hidden', '隐藏任务', 'done', 1000), todo('b', '目标任务', 'done', 2000, ['local'])] }] }], standalone_initiatives: [{ id: 'solo', title: '独立专项验收', status: 'planned', todos: [] }], backlog: [todo('free', '未归属任务', 'backlog', 1000)], tags: [
  { id: 'parent', scope_type: 'project', scope_id: 'p', name: '模块' }, { id: 'local', scope_type: 'initiative', scope_id: 'i', name: '模块' }, { id: 'focus', scope_type: 'project', scope_id: 'p', name: '重点' },
] };
overview.backlog[0].initiative_id = null;
const requests = []; let failOrder = false;
async function main() {
  await mkdir(out, { recursive: true });
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, 'http://localhost');
    const send = (status, data) => { res.writeHead(status, { 'content-type': 'application/json' }); res.end(JSON.stringify(data)); };
    try {
      if (url.pathname.startsWith('/api/')) {
        if (req.method !== 'GET') {
          let raw = ''; for await (const chunk of req) raw += chunk;
          const body = JSON.parse(raw || '{}'); requests.push({ method: req.method, path: url.pathname, body });
          if (url.pathname === '/api/project/todos/order') {
            if (failOrder) return send(500, { error: 'fixture write rejected' });
            for (const [index, id] of body.ids.entries()) {
              const row = overview.goals[0].initiatives[0].todos.find((todo) => todo.id === id);
              row.board_status = body.board_status; row.position = (index + 1) * 1000;
            }
          }
          const id = url.pathname.split('/')[4];
          if (url.pathname.startsWith('/api/project/tags') && req.method !== 'DELETE' && overview.tags.some((tag) => tag.id !== id && tag.scope_type === body.scope_type && tag.scope_id === body.scope_id && tag.name === body.name)) return send(409, { error: '同一归属已有同名 Tag' });
          if (url.pathname === '/api/project/tags') overview.tags.push({ ...body, id: 'created-tag' });
          if (id && url.pathname.startsWith('/api/project/tags/')) {
            if (req.method === 'PATCH') Object.assign(overview.tags.find((tag) => tag.id === id), body);
            if (req.method === 'DELETE') overview.tags = overview.tags.filter((tag) => tag.id !== id);
          }
          if (url.pathname === '/api/project/goals') overview.goals.push({ ...body, id: 'new-project', status: 'active', initiatives: [], updated_at: 2000 });
          if (id && url.pathname.startsWith('/api/project/goals/')) {
            if (req.method === 'PATCH') Object.assign(overview.goals.find((goal) => goal.id === id), body);
            if (req.method === 'DELETE') overview.goals = overview.goals.filter((goal) => goal.id !== id);
          }
          return send(200, { ok: true, id: 'created-tag' });
        }
        if (url.pathname === '/api/project/overview') return send(200, overview);
        if (url.pathname.endsWith('/executions')) return send(200, { assignments: [] });
        return send(200, FIXTURES[url.pathname] || {});
      }
      const file = url.pathname === '/' ? 'index.html' : url.pathname.slice(1);
      const bytes = await readFile(path.join(spa, file));
      res.writeHead(200, { 'content-type': file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html' }); res.end(bytes);
    } catch (error) { send(500, { error: error.message }); }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const page = await browser.newPage({ viewport: { width: 1440, height: 1000 } });
  async function drag(source, destination) {
    const from = await source.boundingBox(); const target = await destination.boundingBox();
    const response = page.waitForResponse((r) => r.url().endsWith('/api/project/todos/order'));
    await page.mouse.move(from.x + from.width / 2, from.y + from.height / 2); await page.mouse.down();
    await page.mouse.move(from.x + from.width / 2 + 12, from.y + from.height / 2, { steps: 3 });
    await page.mouse.move(target.x + target.width / 2, target.y + Math.min(target.height / 2, 70), { steps: 18 }); await page.mouse.up();
    return response;
  }
  const errors = []; const fullscreenDrawers = [];
  page.on('pageerror', (error) => { errors.push(error.message); console.error('PAGE ERROR', error.stack); });
  try {
    await page.addInitScript(() => localStorage.setItem('oc_token', 'fixture-token'));
    await page.goto(`http://127.0.0.1:${server.address().port}`, { waitUntil: 'networkidle' });
    await page.locator('.fleet-nav-category').getByText('项目', { exact: true }).click();
    await page.getByRole('button', { name: '项目浏览器验收', exact: true }).waitFor();
    assert.deepEqual(await page.locator('.fleet-content .ant-tabs').getByRole('tab').allTextContents(), ['项目', '专项', 'TODO', 'Tag']);
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      for (const tab of ['项目', '专项', 'TODO', 'Tag']) {
        await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: tab, exact: true }).click();
        const table = page.getByRole('tabpanel', { name: tab, exact: true }).locator('.project-table');
        await table.waitFor();
        const bounds = await table.evaluate((el) => ({ left: el.getBoundingClientRect().left, right: el.getBoundingClientRect().right, content: el.scrollWidth, width: el.clientWidth }));
        assert(bounds.left >= -1 && bounds.right <= width + 1 && bounds.content <= bounds.width + 1, `${tab} width ${width}: ${JSON.stringify(bounds)}`);
        await page.screenshot({ path: path.join(out, `${tab}-${width}.png`), fullPage: true, animations: 'disabled' });
      }
    }
    console.log('PASS responsive tables');
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: '项目', exact: true }).click();
    await page.getByRole('button', { name: '项目浏览器验收', exact: true }).click();
    const project = page.getByRole('dialog', { name: '项目 · 项目浏览器验收' });
    await project.getByText('专项进度', { exact: true }).waitFor();
    await project.getByRole('button', { name: '专项浏览器验收' }).click();
    const board = page.getByRole('dialog', { name: '专项 · 专项浏览器验收' });
    await board.getByRole('button', { name: '拖动 可拖动任务', exact: true }).waitFor();
    async function verifyFullscreen(drawer, kind, width) {
      await page.waitForFunction((title) => {
        const dialog = [...document.querySelectorAll('[role="dialog"]')].find((item) => item.getAttribute('aria-labelledby') && document.getElementById(item.getAttribute('aria-labelledby'))?.textContent === title);
        if (!dialog) return false;
        const bounds = dialog.getBoundingClientRect();
        return Math.abs(bounds.left) < 1 && Math.abs(bounds.width - innerWidth) < 1;
      }, kind === 'initiative' ? '专项 · 专项浏览器验收' : 'TODO · 可拖动任务');
      const bounds = await drawer.evaluate((el) => {
        const rect = el.getBoundingClientRect();
        return { x: rect.x, y: rect.y, width: rect.width, height: rect.height };
      });
      assert(Math.abs(bounds.x) <= 1 && Math.abs(bounds.y) <= 1 && Math.abs(bounds.width - width) <= 1 && Math.abs(bounds.height - 1000) <= 1, `${kind} fullscreen ${width}: ${JSON.stringify(bounds)}`);
      fullscreenDrawers.push({ kind, viewport: { width, height: 1000 }, bounds });
      await page.screenshot({ path: path.join(out, `${kind}-fullscreen-${width}.png`), animations: 'disabled' });
    }
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await verifyFullscreen(board, 'initiative', width);
      await board.getByRole('button', { name: '可拖动任务', exact: true }).click();
      const todoDetail = page.getByRole('dialog', { name: 'TODO · 可拖动任务' });
      await verifyFullscreen(todoDetail, 'todo', width);
      await todoDetail.locator('.ant-drawer-close').click();
      await todoDetail.waitFor({ state: 'hidden' });
    }
    await page.setViewportSize({ width: 1440, height: 1000 });
    console.log('PASS fullscreen initiative and TODO drawers');
    await board.getByLabel('搜索专项 TODO', { exact: true }).fill('任务');
    await board.getByRole('combobox', { name: '筛选 Tag', exact: true }).click();
    await page.locator('.ant-select-item-option-content').getByText('模块', { exact: true }).click();
    await board.getByLabel('搜索专项 TODO', { exact: true }).click();
    assert.equal(await board.getByRole('button', { name: '隐藏任务', exact: true }).count(), 0);
    assert.equal((await drag(board.getByRole('button', { name: '拖动 可拖动任务', exact: true }), board.getByRole('button', { name: '目标任务', exact: true }))).status(), 200);
    await board.getByText('正在保存状态和顺序…', { exact: true }).waitFor({ state: 'hidden' });
    const move = requests.find((r) => r.path.endsWith('/todos/order'));
    assert(move, 'filtered drag did not persist'); assert.equal(move.body.initiative_id, 'i'); assert.equal(move.body.board_status, 'done');
    assert.deepEqual(move.body.ids, ['hidden', 'a', 'b']);
    await board.getByText('100%', { exact: true }).waitFor();
    console.log('PASS filtered drag');
    await board.locator('.ant-select-selection-item-remove').first().click();
    await board.getByText('按 Tag 分组', { exact: true }).click();
    assert.equal(await board.getByRole('button', { name: '可拖动任务', exact: true }).count(), 2);
    await page.screenshot({ path: path.join(out, 'grouped-board.png'), fullPage: true, animations: 'disabled' });
    const lanes = board.locator('.project-board-lane');
    failOrder = true;
    assert.equal((await drag(board.getByRole('button', { name: '拖动 可拖动任务', exact: true }).first(), lanes.nth(1))).status(), 500);
    await board.getByText('正在保存状态和顺序…', { exact: true }).waitFor({ state: 'hidden' });
    assert.equal(await lanes.filter({ hasText: '已完成' }).getByRole('button', { name: '可拖动任务', exact: true }).count(), 2);
    assert.equal(overview.goals[0].initiatives[0].todos.find((t) => t.id === 'a').board_status, 'done');
    failOrder = false;
    assert.equal((await drag(board.getByRole('button', { name: '拖动 可拖动任务', exact: true }).first(), lanes.nth(2))).status(), 200);
    await board.getByText('正在保存状态和顺序…', { exact: true }).waitFor({ state: 'hidden' });
    assert.equal(await lanes.filter({ hasText: '进行中' }).getByRole('button', { name: '可拖动任务', exact: true }).count(), 2);
    assert.deepEqual(overview.goals[0].initiatives[0].todos.find((t) => t.id === 'a').tag_ids, ['local', 'focus']);

    console.log('PASS grouped sync and failed save rollback');
    await board.getByRole('button', { name: '可拖动任务', exact: true }).first().click();
    const detail = page.getByRole('dialog', { name: 'TODO · 可拖动任务' });
    await detail.getByRole('combobox', { name: 'TODO Tag', exact: true }).click();
    assert.equal(await page.locator('.ant-select-dropdown').last().locator('.ant-select-item-option-content').getByText('模块', { exact: true }).count(), 1);
    await detail.getByLabel('TODO 标题').click(); await detail.locator('.ant-drawer-close').click();
    await board.locator('.ant-drawer-close').click();
    await project.locator('.ant-drawer-close').click();
    await page.getByRole('tab', { name: '专项', exact: true }).click(); await page.getByRole('button', { name: '专项浏览器验收' }).click();
    await board.getByText('按 Tag 分组', { exact: true }).waitFor();
    assert.equal(await board.getByRole('button', { name: '可拖动任务', exact: true }).count(), 2);
    await board.locator('.ant-drawer-close').click();
    await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: 'Tag', exact: true }).click();
    const tags = page.getByLabel('Tag 表格', { exact: true });
    assert.equal(await tags.getByText('模块', { exact: true }).count(), 2);
    await page.getByRole('button', { name: '新建 Tag', exact: true }).click();
    let tagEditor = page.getByRole('dialog', { name: '新建 Tag', exact: true });
    await tagEditor.getByLabel('Tag 名称', { exact: true }).fill('验收标签');
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await page.waitForFunction(() => {
        const dialog = [...document.querySelectorAll('[role="dialog"]')].find((el) => document.getElementById(el.getAttribute('aria-labelledby'))?.textContent === '新建 Tag');
        if (!dialog) return false;
        const bounds = dialog.getBoundingClientRect();
        return bounds.left >= -1 && bounds.right <= innerWidth + 1;
      });
      const bounds = await tagEditor.evaluate((el) => ({ left: el.getBoundingClientRect().left, right: el.getBoundingClientRect().right, scroll: document.documentElement.scrollWidth }));
      assert(bounds.left >= -1 && bounds.right <= width + 1 && bounds.scroll <= width + 1, `Tag editor ${width}: ${JSON.stringify(bounds)}`);
      await page.screenshot({ path: path.join(out, `tag-editor-${width}.png`), animations: 'disabled' });
    }
    await page.setViewportSize({ width: 1440, height: 1000 });
    async function chooseTagOwner(type, owner) {
      await tagEditor.getByRole('combobox', { name: 'Tag 归属类型', exact: true }).click();
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option-content').getByText(type, { exact: true }).click();
      await tagEditor.getByRole('combobox', { name: 'Tag 归属', exact: true }).click();
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option-content').getByText(owner, { exact: true }).click();
    }
    await chooseTagOwner('专项', '独立专项验收 · 独立专项');
    await tagEditor.getByRole('button', { name: /保存 Tag/ }).click();
    await tagEditor.waitFor({ state: 'hidden' });
    await tags.getByLabel('编辑 Tag 验收标签', { exact: true }).waitFor();
    assert.deepEqual(requests.find((r) => r.path === '/api/project/tags').body, { name: '验收标签', scope_type: 'initiative', scope_id: 'solo' });
    await tags.getByLabel('编辑 Tag 验收标签', { exact: true }).click();
    tagEditor = page.getByRole('dialog', { name: '编辑 Tag', exact: true });
    await chooseTagOwner('项目', '项目浏览器验收');
    await tagEditor.getByLabel('Tag 名称', { exact: true }).fill('模块');
    await tagEditor.getByRole('button', { name: /保存 Tag/ }).click();
    await page.getByText('同一归属已有同名 Tag', { exact: true }).waitFor();
    assert.equal(await tagEditor.getByLabel('Tag 名称', { exact: true }).inputValue(), '模块');
    await tagEditor.getByLabel('Tag 名称', { exact: true }).fill('已改名标签');
    await tagEditor.getByRole('button', { name: /保存 Tag/ }).click();
    await tagEditor.waitFor({ state: 'hidden' });
    const movedTag = tags.locator('.ant-table-row').filter({ hasText: '已改名标签' });
    await movedTag.getByText('项目浏览器验收', { exact: true }).waitFor();
    assert.deepEqual(requests.filter((r) => r.path === '/api/project/tags/created-tag' && r.method === 'PATCH').at(-1).body, { name: '已改名标签', scope_type: 'project', scope_id: 'p' });
    await movedTag.getByLabel('删除 Tag 已改名标签', { exact: true }).click();
    await page.getByRole('tooltip').getByRole('button', { name: /删.*除/ }).click();
    await tags.getByLabel('删除 Tag 已改名标签', { exact: true }).waitFor({ state: 'hidden' });
    console.log('PASS tag CRUD, owner changes and failed-save draft');
    await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: '项目', exact: true }).click();
    await page.getByRole('button', { name: '新建项目', exact: true }).click();
    const editor = page.getByRole('dialog', { name: '新建项目', exact: true });
    await editor.getByPlaceholder('一句话标题').fill('CRUD 验收项目');
    await editor.getByRole('button', { name: /保.*存/ }).click();
    await page.getByRole('button', { name: 'CRUD 验收项目', exact: true }).waitFor();
    const row = page.locator('.ant-table-row').filter({ hasText: 'CRUD 验收项目' });
    await row.getByRole('button', { name: /操.*作/ }).click();
    await page.getByRole('menuitem', { name: '编辑', exact: true }).click();
    const editing = page.getByRole('dialog', { name: '编辑项目', exact: true });
    await editing.getByPlaceholder('一句话标题').fill('CRUD 已修改项目');
    await editing.getByRole('button', { name: /保.*存/ }).click();
    const updatedRow = page.locator('.ant-table-row').filter({ hasText: 'CRUD 已修改项目' });
    await updatedRow.getByRole('button', { name: /操.*作/ }).click();
    await page.getByRole('menuitem', { name: '删除', exact: true }).click();
    await page.getByRole('dialog', { name: '删除该项目？' }).getByRole('button', { name: /确.*定/ }).click();
    await page.getByRole('button', { name: 'CRUD 已修改项目', exact: true }).waitFor({ state: 'hidden' });
    assert.deepEqual(errors, []);
    const receipt = { result: 'PASS', screenshots: out, widths: [1920, 1280, 768, 390], fullscreen_drawers: fullscreenDrawers, filtered_drag: move.body, grouped_cards: true, grouped_move_sync: true, failed_save_rollback: true, scope_precedence: true, tag_crud: true, tag_owner_changes: true, tag_failed_save_draft: true, project_crud: true, page_errors: errors };
    await writeFile(path.join(out, 'receipt.json'), JSON.stringify(receipt, null, 2));
    console.log(JSON.stringify(receipt));
  } catch (error) {
    await page.screenshot({ path: path.join(out, 'failure.png'), fullPage: true, animations: 'disabled' });
    require('node:fs').writeFileSync(path.join(out, 'failure.html'), await page.content());
    throw error;
  } finally { await browser.close(); await new Promise((resolve) => server.close(resolve)); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
