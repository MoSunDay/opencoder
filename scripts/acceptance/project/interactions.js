const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { execFileSync } = require('node:child_process');
const { seedCards, verifyCards } = require('./design/cards');

async function verifyComments({ page, api, editor, todo, binary, base, credential, root, output, measure }) {
  await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: 'TODO', exact: true }).click();
  await page.getByRole('button', { name: todo.title, exact: true }).click();
  await editor.getByRole('tab', { name: '任务', exact: true }).click();
  await editor.getByText('暂无评论', { exact: true }).waitFor();
  assert.equal(await editor.getByRole('tab', { name: '评论', exact: true }).count(), 0);
  const order = await editor.evaluate((element) => element.querySelector('.project-todo-field-grid').compareDocumentPosition(element.querySelector('.project-task-comments')));
  assert(order & 4, 'comments follow task fields');
  const route = `/api/project/todos/${todo.id}/comments`;
  const requests = [];
  const observe = (request) => { if (request.method() === 'POST' && new URL(request.url()).pathname === route) requests.push(request.postDataJSON()); };
  page.on('request', observe);
  await editor.getByLabel('评论正文', { exact: true }).fill('## 人工补充\n\n**确认验收标准**');
  await page.route(`**${route}`, (request) => request.request().method() === 'POST' ? request.fulfill({ status: 500, json: { error: '评论失败重试验收' } }) : request.continue());
  await editor.getByRole('button', { name: /发表评论$/ }).click();
  await editor.getByText('评论失败重试验收', { exact: true }).waitFor();
  assert((await editor.getByLabel('评论正文', { exact: true }).inputValue()).includes('确认验收标准'));
  await page.unroute(`**${route}`);
  await editor.getByRole('button', { name: /发表评论$/ }).click();
  await editor.getByRole('heading', { name: '人工补充', exact: true }).waitFor();
  assert.deepEqual(requests[0], requests[1], 'comment retry must reuse the input ID');
  page.off('request', observe);
  const jsonFile = path.join(root, 'comment.json');
  fs.writeFileSync(jsonFile, JSON.stringify({ request_id: 'browser-capability-note', body_md: '**来自能力的追加信息**', capability_id: 'builtin-operator' }));
  const clientConfig = path.join(root, 'ctl.json'); fs.writeFileSync(clientConfig, '{}');
  const cli = path.join(path.dirname(binary), 'opencoder-cli');
  const args = ['--client-config', clientConfig, '--server', base, '--token-file', credential, 'project', 'todos', 'comments', 'add', todo.id, '--json', '@' + jsonFile];
  const first = JSON.parse(execFileSync(cli, args, { encoding: 'utf8' }));
  const retry = JSON.parse(execFileSync(cli, args, { encoding: 'utf8' }));
  assert.equal(first.seq, retry.seq); assert.equal(first.author_kind, 'capability');
  await editor.getByText('来自能力的追加信息', { exact: true }).waitFor();
  for (let i = 0; i < 55; i++) await api('POST', route, { request_id: `bulk-${i}`, body_md: `分页补充 ${i}` });
  await editor.getByText('分页补充 54', { exact: true }).waitFor();
  for (const width of [1920, 1280, 768, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await measure(editor, 'comments', width, false);
  }
  await editor.locator('.ant-drawer-close').click(); await editor.waitFor({ state: 'hidden' });
  await page.getByRole('button', { name: todo.title, exact: true }).click();
  await editor.getByRole('tab', { name: '任务', exact: true }).click();
  await editor.getByRole('button', { name: '加载更早评论', exact: true }).click();
  await editor.getByRole('heading', { name: '人工补充', exact: true }).waitFor();
  assert.equal(await editor.locator('[data-comment-seq]').count(), 57);
  await editor.getByLabel('评论正文', { exact: true }).fill('需要保留的草稿');
  await editor.getByRole('tab', { name: '设置', exact: true }).click();
  await editor.getByRole('tab', { name: '任务', exact: true }).click();
  assert.equal(await editor.getByLabel('评论正文', { exact: true }).inputValue(), '需要保留的草稿');
  await editor.locator('.ant-drawer-close').click();
  await page.getByRole('button', { name: '继续编辑', exact: true }).click();
  assert.equal(await editor.getByLabel('评论正文', { exact: true }).inputValue(), '需要保留的草稿');
  await editor.locator('.ant-drawer-close').click();
  await page.getByRole('button', { name: '放弃修改', exact: true }).click();
  await editor.waitFor({ state: 'hidden' });
  fs.writeFileSync(path.join(output, 'comments.json'), JSON.stringify({ count: 57, human_signature: true, capability_signature: first.author_id, retry_idempotency: true, pagination: true, draft_preservation: true }));
  console.log('PASS Markdown comments, real CLI, idempotency, pagination and draft preservation');
}

async function verifyDetails({ page, api, project, group, measure }) {
  const cards = await seedCards(api, group);
  await page.reload({ waitUntil: 'networkidle' });
  await page.locator('.fleet-content .ant-tabs').getByRole('tab', { name: '项目', exact: true }).click();
  await page.getByRole('button', { name: project.title, exact: true }).click();
  const projectPanel = page.getByRole('dialog', { name: `项目 · ${project.title}`, exact: true });
  for (const width of [1920, 1280, 768, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await projectPanel.getByRole('tab', { name: '概览', exact: true }).click();
    await measure(projectPanel, 'project-overview', width, false);
    const size = await projectPanel.boundingBox();
    assert(Math.abs(size.width - (width > 1024 ? width * 0.7 : width)) < 2, 'project drawer uses 70% on desktop');
    await projectPanel.getByRole('tab', { name: '专项', exact: true }).click();
    await measure(projectPanel, 'project-initiatives', width, false);
  }
  await projectPanel.getByRole('button', { name: group.title, exact: true }).click();
  const initiative = page.getByRole('dialog', { name: `专项 · ${group.title}`, exact: true });
  for (const width of [1920, 1280, 768, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    assert.equal(await initiative.getByRole('tab').count(), 0, 'initiative has no tabs');
    await initiative.getByLabel('当前概述', { exact: true }).waitFor();
    const layout = await initiative.evaluate((element) => {
      const context = element.querySelector('.project-initiative-context'), meta = element.querySelector('.project-detail-meta');
      const people = meta.querySelector('.project-detail-people').getBoundingClientRect(), progress = meta.querySelector('.project-progress').getBoundingClientRect();
      const cards = [...element.querySelectorAll('.project-task-card')];
      return { contextFirst: !!(context.compareDocumentPosition(meta) & 4), leftAligned: getComputedStyle(meta).justifyContent === 'flex-start',
        compactMeta: progress.top >= people.bottom || Math.abs(people.right + parseFloat(getComputedStyle(meta).columnGap) - progress.left) < 2,
        cardsFit: cards.every((card) => card.scrollWidth <= card.clientWidth + 1),
        cardTitleFits: cards.every((card) => { const title = card.querySelector('.project-card-title').getBoundingClientRect(), controls = card.querySelector('.project-card-controls').getBoundingClientRect(); return title.right <= controls.left + 1; }) };
    });
    assert(Object.values(layout).every(Boolean), JSON.stringify({ width, ...layout }));
    await measure(initiative, 'initiative-board', width, false);
  }
  await verifyCards({ page, api, initiative, rows: cards });
  await initiative.getByRole('button', { name: '编辑专项', exact: true }).click();
  const edit = page.getByRole('dialog', { name: '编辑专项', exact: true });
  for (const width of [1920, 1280, 768, 390]) {
    await page.setViewportSize({ width, height: 1000 });
    await measure(edit, 'initiative-edit', width, false);
  }
  const description = '## 专项交付范围\n\n**编辑专项说明后保留当前概述**';
  await edit.getByLabel('专项说明', { exact: true }).fill(description);
  const updated = page.waitForResponse((r) => r.url().endsWith(`/api/project/initiatives/${group.id}`) && r.request().method() === 'PATCH');
  await edit.getByRole('button', { name: /^保\s*存$/ }).click();
  assert.equal((await updated).status(), 200);
  await edit.waitFor({ state: 'hidden' });
  await initiative.getByLabel('专项说明', { exact: true }).getByText('编辑专项说明后保留当前概述', { exact: true }).waitFor();
  const savedGroup = await api('GET', `/api/project/initiatives/${group.id}`);
  assert.equal(savedGroup.detail_md, description);
  await initiative.press('Escape'); await initiative.waitFor({ state: 'hidden' });
  await projectPanel.locator('.ant-drawer-close').click(); await projectPanel.waitFor({ state: 'hidden' });
  console.log('PASS project tabs and continuous initiative layout at four widths');
}

module.exports = { verifyComments, verifyDetails };
