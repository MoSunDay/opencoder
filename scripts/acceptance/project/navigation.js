// Real browser history, sharing and login over the built SPA; isolated read-only fixtures.
// node scripts/acceptance/project/navigation.js OUTPUT_DIR
const assert = require('node:assert/strict');
const { createServer } = require('node:http');
const { readFile, writeFile, mkdir } = require('node:fs/promises');
const path = require('node:path');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const { FIXTURES } = require('../spa_responsive_fixtures');
const spa = path.resolve(__dirname, '../../../crates/web/spa/dist');
const task = { id: 'todo-link', title: '链接验收任务', draft: '# 任务要求\n\n正文预览', board_status: 'todo', tag_ids: [] };
const initiative = { id: 'initiative-link', title: '链接验收子项', todos: [task] };
const project = { id: 'project-link', title: '链接验收项目', status: 'active', initiatives: [initiative] };
const overview = { goals: [project], standalone_initiatives: [{ id: 'solo-link', title: '独立子项', todos: [] }],
  backlog: [{ ...task, id: 'free-link', title: '未归属任务' }], tags: [] };

async function main() {
  const output = process.argv[2]; assert(output, 'OUTPUT_DIR required'); await mkdir(output, { recursive: true });
  const errors = [], writes = [], checks = [];
  let failOverview = false;
  const server = createServer(async (req, res) => {
    const url = new URL(req.url, 'http://localhost');
    const send = (status, value) => { res.writeHead(status, { 'content-type': 'application/json' }); res.end(JSON.stringify(value)); };
    try {
      if (url.pathname.startsWith('/api/')) {
        if (req.method !== 'GET') { writes.push(req.url); return send(405, { error: 'read-only acceptance' }); }
        if (url.pathname === '/api/auth/status') return send(200, { enabled: true });
        if (req.headers.authorization !== 'Bearer link-reader') return send(401, { error: '请登录' });
        if (url.pathname === '/api/me') return send(200, { name: 'link-reader', role: 'editor' });
        if (url.pathname === '/api/project/overview') return failOverview ? send(503, { error: '总览暂时不可用' }) : send(200, overview);
        if (url.pathname === '/api/project/people') return send(200, { people: [] });
        if (url.pathname.endsWith('/comments')) return send(200, { comments: [], has_more: false });
        if (url.pathname.endsWith('/executions')) return send(200, { assignments: [] });
        return send(200, FIXTURES[url.pathname] || {});
      }
      const file = url.pathname === '/' ? 'index.html' : url.pathname.slice(1);
      const bytes = await readFile(path.join(spa, file));
      res.writeHead(200, { 'content-type': file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html' }); res.end(bytes);
    } catch (error) { send(500, { error: error.message }); }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  const context = await browser.newContext({ viewport: { width: 1280, height: 1000 } });
  context.setDefaultTimeout(30000);
  context.on('page', (page) => page.on('pageerror', (error) => errors.push(error.message)));
  const page = await context.newPage();
  const dialog = (name, target = page) => target.getByRole('dialog', { name, exact: true });
  const hash = (value) => page.waitForURL((url) => url.hash === value);
  const close = async (name) => { const panel = dialog(name); await panel.locator('.ant-drawer-close').click(); await panel.waitFor({ state: 'hidden' }); };
  try {
    // First visit has no login state and an unrelated saved landing page.
    await context.addInitScript((origin) => { if (location.origin === origin) localStorage.setItem('oc_nav_page', JSON.stringify('nodes')); }, base);
    await page.goto(base + '/#todo=todo-link', { waitUntil: 'networkidle' });
    await dialog('Opencoder Fleet · 登录').getByPlaceholder('访问令牌').fill('link-reader');
    await dialog('Opencoder Fleet · 登录').getByRole('button', { name: /连\s*接/ }).click();
    await dialog('TODO · 链接验收任务').waitFor();
    assert.equal(new URL(page.url()).hash, '#todo=todo-link');
    checks.push('login retains shared TODO destination');

    const targets = [
      ['project=project-link', '项目 · 链接验收项目'], ['initiative=initiative-link', '子项 · 链接验收子项'],
      ['todo=todo-link', 'TODO · 链接验收任务'], ['initiative=solo-link', '子项 · 独立子项'], ['todo=free-link', 'TODO · 未归属任务'],
    ];
    for (const [route, title] of targets) {
      await page.goto(base + '/#' + route); await dialog(title).waitFor();
      await page.reload(); await dialog(title).waitFor();
      const shared = await context.newPage(); await shared.goto(page.url()); await dialog(title, shared).waitFor(); await shared.close();
      if (route.includes('link') && !route.includes('solo') && !route.includes('free')) {
        for (const width of [1920, 1280, 768, 390]) {
          await page.setViewportSize({ width, height: 1000 });
          const fits = await dialog(title).evaluate((el) => el.scrollWidth <= el.clientWidth + 1);
          assert(fits, `${title} overflows at ${width}`);
          await page.screenshot({ path: path.join(output, `${route.split('=')[0]}-${width}.png`), animations: 'disabled' });
        }
      }
      await close(title); assert.equal(new URL(page.url()).hash, '');
    }
    checks.push('project, initiative, TODO, standalone and unassigned links survive refresh and new tabs');
    await page.setViewportSize({ width: 1280, height: 1000 });
    await page.goto(base + '/#project=project-link');
    const projectPanel = dialog('项目 · 链接验收项目'); await projectPanel.getByRole('tab', { name: '子项', exact: true }).click();
    await projectPanel.getByRole('button', { name: '链接验收子项', exact: true }).click();
    await dialog('子项 · 链接验收子项').getByRole('button', { name: '链接验收任务', exact: true }).click();
    const nested = '#project=project-link&initiative=initiative-link&todo=todo-link';
    await hash(nested);
    await page.goBack(); await hash('#project=project-link&initiative=initiative-link');
    await dialog('TODO · 链接验收任务').waitFor({ state: 'hidden' });
    await page.goForward(); await dialog('TODO · 链接验收任务').waitFor(); await hash(nested);
    checks.push('nested details track browser Back and Forward');

    await page.getByLabel('TODO 标题', { exact: true }).fill('未保存的任务标题');
    await page.getByText('有未保存的修改', { exact: true }).waitFor();
    await page.goBack();
    await dialog('有内容尚未保存').waitFor(); await hash(nested);
    await dialog('有内容尚未保存').getByRole('button', { name: '继续编辑' }).click();
    await dialog('有内容尚未保存').waitFor({ state: 'hidden' });
    assert.equal(await page.getByLabel('TODO 标题', { exact: true }).inputValue(), '未保存的任务标题');
    await page.goBack(); await dialog('有内容尚未保存').getByRole('button', { name: '放弃并继续' }).click();
    await hash('#project=project-link&initiative=initiative-link');
    await page.goForward(); await dialog('TODO · 链接验收任务').waitFor();
    assert.equal(await page.getByLabel('TODO 标题', { exact: true }).inputValue(), task.title);
    checks.push('cancelled Back restores draft and URL; confirmed Back preserves Forward destination');
    await page.reload(); await dialog('TODO · 链接验收任务').waitFor(); await hash(nested);
    checks.push('nested details survive refresh');

    await page.getByLabel('评论正文', { exact: true }).fill('未发送的评论');
    const prompt = page.waitForEvent('dialog');
    await page.evaluate(() => { setTimeout(() => location.reload(), 0); });
    const leave = await prompt; assert.equal(leave.type(), 'beforeunload'); await leave.dismiss();
    await hash(nested);
    assert.equal(await page.getByLabel('评论正文', { exact: true }).inputValue(), '未发送的评论');
    await page.getByLabel('评论正文', { exact: true }).fill('');
    checks.push('cancelled page reload retains the unsent comment and destination');

    await close('TODO · 链接验收任务'); await close('子项 · 链接验收子项'); await close('项目 · 链接验收项目');
    await page.goto(base + '/#todo=missing');
    await dialog('TODO · missing').getByText('TODO不存在或已被删除').waitFor();
    failOverview = true; await page.reload();
    await dialog('TODO · missing').getByText('无法读取TODO').waitFor();
    failOverview = false; await dialog('TODO · missing').getByRole('button', { name: /重\s*试/ }).click();
    await dialog('TODO · missing').getByText('TODO不存在或已被删除').waitFor();
    checks.push('missing and unavailable records remain distinct and retryable');
    assert.deepEqual(errors, []); assert.deepEqual(writes, []);
    await writeFile(path.join(output, 'result.json'), JSON.stringify({ result: 'PASS', checks, widths: [1920, 1280, 768, 390], page_errors: errors, writes }, null, 2));
    console.log(JSON.stringify({ result: 'PASS', checks: checks.length, output }));
  } catch (error) {
    await page.screenshot({ path: path.join(output, 'failure.png') }).catch(() => {});
    await writeFile(path.join(output, 'failure.json'), JSON.stringify({ error: error.stack, checks, page_errors: errors, writes }, null, 2));
    throw error;
  } finally { await browser.close(); await new Promise((resolve) => server.close(resolve)); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
