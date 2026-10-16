// node scripts/acceptance/project/forms.js SERVER_BINARY SPA_DIST OUTPUT_DIR [all|todo|details|links|cards]
// Real isolated CRUD; only the capability catalog and failed responses are fixtures.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, execFileSync } = require('node:child_process');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const { verifyComments, verifyDetails } = require('./interactions');
const { verifyExecutionLinks } = require('./design/links');
const { verifyCardSurfaces } = require('./design/cards');
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function main() {
  const [binary, dist, output, scope = 'all'] = process.argv.slice(2);
  assert(['all', 'todo', 'details', 'links', 'cards'].includes(scope), 'scope must be all, todo, details, links or cards');
  assert(binary && dist && output, 'SERVER_BINARY SPA_DIST OUTPUT_DIR required');
  fs.mkdirSync(output, { recursive: true });
  const hash = crypto.createHash('sha256');
  for (const file of fs.readdirSync(dist, { recursive: true }).filter((name) => fs.statSync(path.join(dist, name)).isFile()).sort()) hash.update(fs.readFileSync(path.join(dist, file)));
  const build = JSON.parse(execFileSync(binary, ['--build-info'], { encoding: 'utf8' }));
  assert.equal(build.spa_sha256, hash.digest('hex'), 'Server must embed the tested SPA');
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-todo-form-'));
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
      const data = await response.json(); assert(response.ok, `${method} ${route}: ${response.status}`); return data;
    }
    const project = await api('POST', '/api/project/goals', { title: '表单布局验收项目' });
    const groupTitle = '完整专项名称：覆盖长内容、多标签、多人关注、保存失败重试与手机屏幕下的任务编辑';
    const group = await api('POST', '/api/project/initiatives', { title: groupTitle, goal_id: project.id });
    const names = ['form_owner', 'form_follower_alexander', 'form_follower_catherine', 'form_follower_christopher'];
    for (const name of names) await api('POST', '/api/users', { name, role: 'editor' });
    const tagNames = ['重要任务', '跨模块协作', '需要覆盖所有长名称与换行内容的验收标签'];
    const tags = [];
    for (const name of tagNames) tags.push(await api('POST', '/api/project/tags', { name, scope_type: 'global' }));
    const scopedTag = await api('POST', '/api/project/tags', { name: '专项专用', scope_type: 'initiative', scope_id: group.id });
    const draft = Array.from({ length: 8 }, (_, i) => `${i + 1}. 检查字段分组、可见标签、输入间距和保存后的完整内容。`).join('\n');
    const todo = await api('POST', '/api/project/todos', { title: '编辑表单布局验收', draft, initiative_id: group.id,
      board_status: 'in_progress', owner_user_name: names[0], follower_user_names: names.slice(1), tag_ids: [...tags.map((tag) => tag.id), scopedTag.id] });
    const readTodo = () => api('GET', `/api/project/todos/${todo.id}`);
    browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    page = await browser.newPage({ viewport: { width: 1920, height: 1000 } });
    const errors = []; const measurements = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
    const cap = { id: 'agent:form-layout', kind: 'agent', target: '表单显示验收', summary: '执行设置表单', definition: {}, required_inputs: ['目标路径', '验收要求'], input_desc: '填写任务说明和能力需要的参数。' };
    await page.route('**/api/brain/library', (route) => route.fulfill({ json: { capabilities: [cap] } }));
    await page.goto(base, { waitUntil: 'networkidle' });
    await page.locator('.fleet-nav-category').getByText('项目', { exact: true }).click();
    const tab = (name) => page.locator('.fleet-content .ant-tabs').getByRole('tab', { name, exact: true });
    await tab('TODO').click();
    const dialog = (name) => page.getByRole('dialog', { name, exact: true });
    const editor = dialog(`TODO · ${todo.title}`);
    async function choose(panel, label, option) {
      const control = panel.getByLabel(label, { exact: true });
      await control.click(); await control.fill(option);
      await page.locator('.ant-select-dropdown:visible .ant-select-item-option').filter({ hasText: option }).click();
      await control.press('Escape');
    }
    async function measure(panel, screen, width, footer = true) {
      await panel.locator('.ant-drawer-body').waitFor();
      await panel.locator('.ant-drawer-body').evaluate((el) => { el.scrollTop = 0; });
      const result = await panel.evaluate((element) => {
        const rect = (el) => { const r = el.getBoundingClientRect(); return { x: r.x, y: r.y, width: r.width, height: r.height, bottom: r.bottom, right: r.right }; };
        const fields = [...element.querySelectorAll('.ant-form-item')].filter((el) => el.getBoundingClientRect().height > 0).map((el) => ({ ...rect(el), label: el.querySelector('label')?.textContent }));
        const overlap = fields.some((a, i) => fields.slice(i + 1).some((b) => Math.min(a.right, b.right) - Math.max(a.x, b.x) > 1 && Math.min(a.bottom, b.bottom) - Math.max(a.y, b.y) > 1));
        const sections = [...element.querySelectorAll('.project-todo-section')];
        const body = element.querySelector('.ant-drawer-body');
        const content = body.querySelector('.project-todo-workspace, :scope > .project-detail-overview');
        const fitsBody = !content || Math.abs(content.getBoundingClientRect().left - body.getBoundingClientRect().left - parseFloat(getComputedStyle(body).paddingLeft)) <= 1
          && Math.abs(content.getBoundingClientRect().width - body.clientWidth + parseFloat(getComputedStyle(body).paddingLeft) + parseFloat(getComputedStyle(body).paddingRight)) <= 1;
        const toggles = [...element.querySelectorAll('.project-markdown-toolbar .ant-segmented')].filter((el) => el.getBoundingClientRect().height > 0);
        const save = [...element.querySelectorAll('.ant-drawer-footer button')].find((el) => el.textContent.includes('保存 TODO'));
        const taskHeading = element.querySelector('.project-task-heading');
        const title = taskHeading?.querySelector('input');
        const status = taskHeading?.querySelector('.ant-select');
        const statusWithTitle = !taskHeading || !taskHeading.getBoundingClientRect().height || title && status
          && Math.abs(rect(title).y - rect(status).y) <= 1 && rect(title).right <= rect(status).x
          && rect(status).right <= element.getBoundingClientRect().right;
        const launch = element.querySelector('.project-execution-launch');
        const launchParts = launch && launch.getBoundingClientRect().height > 0 ? [...launch.children].map(rect) : [];
        const inlineLaunch = launchParts.length === 0 || launchParts.length === 2
          && launchParts[0].x >= element.getBoundingClientRect().left && launchParts[1].right <= element.getBoundingClientRect().right
          && Math.abs(launchParts[0].bottom - launchParts[1].bottom) <= 1 && launchParts[1].x >= launchParts[0].right;
        return { statusWithTitle, inlineLaunch, fillsBody: fitsBody, compactToggle: toggles.every((el) => el.getBoundingClientRect().width <= 160), compactSave: !save || save.getBoundingClientRect().width <= 160, outerFits: element.scrollWidth <= element.clientWidth + 1, overlap,
          sectionFits: sections.every((el) => el.scrollWidth <= el.clientWidth + 1),
          fieldGaps: [...element.querySelectorAll('.project-todo-field-grid')].filter((el) => el.getBoundingClientRect().height > 0).map((el) => parseFloat(getComputedStyle(el).rowGap)),
          columns: [...element.querySelectorAll('.project-todo-field-grid')].filter((el) => el.getBoundingClientRect().height > 0).map((el) => getComputedStyle(el).gridTemplateColumns.split(' ').length),
          selectedTextFits: [...element.querySelectorAll('.ant-select-multiple .ant-select-selection-item-content')].filter((el) => el.getBoundingClientRect().height > 0).every((el) => el.scrollWidth <= el.clientWidth + 1),
          footer: element.querySelector('.ant-drawer-footer') ? rect(element.querySelector('.ant-drawer-footer')) : null,
          labelCount: [...element.querySelectorAll('label[for]')].filter((label) => label.getBoundingClientRect().height > 0 && document.getElementById(label.htmlFor)).length,
          fieldCount: fields.length };
      });
      assert(result.statusWithTitle && result.inlineLaunch && result.fillsBody && result.compactToggle && result.compactSave && result.outerFits && result.sectionFits && result.selectedTextFits && !result.overlap, JSON.stringify({ screen, width, ...result }));
      assert(result.fieldGaps.every((gap) => gap >= 20), JSON.stringify(result));
      assert(result.columns.every((count) => count === (width <= 600 ? 1 : 2)), JSON.stringify(result));
      assert.equal(result.labelCount, result.fieldCount, 'Each field needs an associated visible label');
      if (footer) assert(result.footer.y >= 0 && result.footer.bottom <= 1001, 'footer must remain in viewport');
      measurements.push({ screen, width, ...result });
      await page.screenshot({ path: path.join(output, `${screen}-${width}.png`), animations: 'disabled' });
      await panel.locator('.ant-drawer-body').evaluate((el) => { el.scrollTop = el.scrollHeight; });
      await page.screenshot({ path: path.join(output, `${screen}-bottom-${width}.png`), animations: 'disabled' });
    }
    if (scope === 'cards') {
      const result = await verifyCardSurfaces({ page, api, group, output });
      assert.deepEqual(errors, []);
      fs.writeFileSync(path.join(output, 'receipt.json'), JSON.stringify({ result: 'PASS', server_spa_sha256: build.spa_sha256, scope, ...result, page_errors: errors }, null, 2));
      console.log(`PASS: card surface drag, menu, keyboard, rollback and four widths; ${output}`);
      return;
    }
    if (scope === 'links') {
      await verifyExecutionLinks({ page, api, todo, output });
      assert.deepEqual(errors, []);
      fs.writeFileSync(path.join(output, 'receipt.json'), JSON.stringify({ result: 'PASS', server_spa_sha256: build.spa_sha256, scope,
        ...JSON.parse(fs.readFileSync(path.join(output, 'execution-links.json'))), page_errors: errors }, null, 2));
      return;
    }
    if (scope === 'details') {
      await verifyDetails({ page, api, project, group, measure });
      assert.deepEqual(errors, []);
      fs.writeFileSync(path.join(output, 'receipt.json'), JSON.stringify({ result: 'PASS', server_spa_sha256: build.spa_sha256, scope, measurements,
        real_initiative_description_save: true, pointer_drag_persisted: true, keyboard_cancel_preserves_drawer: true, page_errors: errors }, null, 2));
      console.log(`PASS: initiative/card layout, description save and drag cancellation; ${output}`);
      return;
    }
    await page.getByRole('button', { name: todo.title, exact: true }).click();
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      if (width !== 1920) { await editor.locator('.ant-drawer-close').click(); await editor.waitFor({ state: 'hidden' }); await page.getByRole('button', { name: todo.title, exact: true }).click(); }
      await measure(editor, 'reading', width);
      assert.equal(await editor.getByRole('button', { name: '编辑任务', exact: true }).count(), 0);
      assert.equal(await editor.getByLabel('任务说明', { exact: true }).isVisible(), false);
      await editor.getByLabel('任务说明显示方式', { exact: true }).getByText('编辑', { exact: true }).click();
      await measure(editor, 'task-edit', width);
      await editor.getByRole('tab', { name: '设置', exact: true }).click();
      await measure(editor, 'settings', width);
      await editor.getByRole('tab', { name: '执行', exact: true }).click();
      assert.equal(await editor.getByLabel('已有执行 ID', { exact: true }).count(), 0);
      await measure(editor, 'execution-records', width);
      await editor.getByRole('tab', { name: '任务', exact: true }).click();
    }
    console.log('PASS editor labels, long content, multiple tags and followers at four widths');
    await editor.getByLabel('TODO 标题', { exact: true }).fill('');
    await editor.getByRole('button', { name: /保存 TODO/ }).click();
    await editor.getByText('TODO 标题不能为空', { exact: true }).waitFor();
    assert.equal(await editor.getByLabel('TODO 标题', { exact: true }).evaluate((el) => document.activeElement === el), true);
    await editor.getByLabel('TODO 标题', { exact: true }).fill(todo.title);
    const changed = `${draft}\n新增：失败时保留未保存的内容。`;
    await editor.getByLabel('任务说明', { exact: true }).fill(changed);
    const patchPath = `/api/project/todos/${todo.id}`;
    await page.route(`**${patchPath}`, (route) => route.request().method() === 'PATCH' ? route.fulfill({ status: 500, json: { error: '表单保存失败验收' } }) : route.continue());
    await editor.getByRole('button', { name: /保存 TODO/ }).click();
    await editor.getByText('表单保存失败验收', { exact: true }).waitFor();
    assert((await editor.getByText('表单保存失败验收', { exact: true }).boundingBox()).y < 1000, 'save error must be visible beside actions');
    assert.equal(await editor.getByLabel('任务说明', { exact: true }).inputValue(), changed);
    await page.unroute(`**${patchPath}`);
    const saved = page.waitForResponse((r) => r.url().endsWith(patchPath) && r.request().method() === 'PATCH');
    await editor.getByRole('button', { name: /保存 TODO/ }).click(); assert.equal((await saved).status(), 200);
    assert.equal((await readTodo()).draft, changed);
    await editor.getByRole('tab', { name: '执行', exact: true }).click();
    await choose(editor, '执行能力', cap.summary);
    await editor.getByRole('button', { name: /指派所选能力/ }).click();
    await editor.getByLabel('执行任务', { exact: true }).waitFor();
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await measure(editor, 'launch', width, false);
    }
    await editor.getByRole('button', { name: '返回指派记录', exact: true }).click();
    await editor.locator('.ant-drawer-close').click(); await editor.waitFor({ state: 'hidden' });
    console.log('PASS edit failure recovery, real persistence and execution input layout');
    await page.getByRole('button', { name: '新建 TODO', exact: true }).click();
    const create = dialog('新建 TODO');
    await create.getByLabel('标题', { exact: true }).fill('新建布局验收');
    await create.getByLabel('任务说明', { exact: true }).fill(draft);
    await create.getByRole('tab', { name: '设置', exact: true }).click();
    await choose(create, '所属专项', groupTitle);
    for (const name of tagNames) await choose(create, 'TODO Tag', name);
    await choose(create, '负责人', names[0]);
    for (const name of names.slice(1)) await choose(create, '关注人', name);
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await measure(create, 'create-settings', width);
      await create.getByRole('tab', { name: '任务', exact: true }).click();
      await measure(create, 'create-task', width);
      await create.getByRole('tab', { name: '设置', exact: true }).click();
    }
    await page.route('**/api/project/todos', (route) => route.request().method() === 'POST' ? route.fulfill({ status: 500, json: { error: '创建失败验收' } }) : route.continue());
    await create.getByRole('button', { name: /^创\s*建$/ }).click();
    await create.getByText('创建失败验收', { exact: true }).waitFor();
    assert((await create.getByText('创建失败验收', { exact: true }).boundingBox()).y < 1000, 'create error must be visible beside actions');
    assert.equal(await create.getByLabel('标题', { exact: true }).inputValue(), '新建布局验收');
    await page.unroute('**/api/project/todos');
    const createdResponse = page.waitForResponse((r) => r.url().endsWith('/api/project/todos') && r.request().method() === 'POST');
    await create.getByRole('button', { name: /创\s*建/ }).click();
    const created = await (await createdResponse).json();
    assert.equal(created.title, '新建布局验收'); assert.equal(created.draft, draft); assert.equal(created.initiative_id, group.id);
    assert.equal(created.owner_user_name, names[0]); assert.deepEqual(created.follower_user_names.sort(), names.slice(1).sort());
    const overview = await api('GET', '/api/project/overview');
    const persisted = overview.goals[0].initiatives[0].todos.find((row) => row.id === created.id);
    assert.deepEqual(persisted.tag_ids.sort(), tags.map((tag) => tag.id).sort());
    const next = dialog('TODO · 新建布局验收'); await next.getByLabel('任务说明预览', { exact: true }).waitFor();
    await page.reload({ waitUntil: 'networkidle' });
    await tab('TODO').click();
    await page.getByRole('button', { name: '新建布局验收', exact: true }).click();
    assert.equal(await next.getByLabel('任务说明', { exact: true }).inputValue(), draft);
    await next.locator('.ant-drawer-close').click(); await next.waitFor({ state: 'hidden' });
    await tab('专项').click(); await page.getByRole('button', { name: groupTitle, exact: true }).click();
    const initiative = dialog(`专项 · ${groupTitle}`);
    await initiative.getByRole('button', { name: '新建 TODO', exact: true }).click();
    await create.getByLabel('标题', { exact: true }).fill('专项内新建布局验收');
    const boardCreated = page.waitForResponse((r) => r.url().endsWith('/api/project/todos') && r.request().method() === 'POST');
    await create.getByRole('button', { name: /^创\s*建$/ }).click();
    assert.equal((await (await boardCreated).json()).initiative_id, group.id);
    const inner = dialog('TODO · 专项内新建布局验收'); await inner.waitFor();
    await inner.locator('.ant-drawer-close').click(); await inner.waitFor({ state: 'hidden' });
    await initiative.locator('.ant-drawer-close').click(); await initiative.waitFor({ state: 'hidden' });
    await verifyComments({ page, api, editor, todo, binary, base, credential, root, output, measure });
    await verifyExecutionLinks({ page, api, todo, output });
    if (scope !== 'todo') await verifyDetails({ page, api, project, group, measure });
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'receipt.json'), JSON.stringify({ result: 'PASS', server_spa_sha256: build.spa_sha256,
      measurements, scope, real_create_and_edit: true, create_from_initiative: true, failed_drafts_retained: true,
      refresh_persistence: true, empty_title_focus: true, markdown_reading: true, comments_and_cli: true, project_tabs_and_continuous_initiative: true, page_errors: errors }, null, 2));
    console.log(`PASS: TODO forms, CRUD and retry; ${output}`);
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
