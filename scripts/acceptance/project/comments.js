// Real isolated Server + embedded SPA: editable comments and conflict handling.
// node scripts/acceptance/project/comments.js SERVER_BINARY SPA_DIST OUTPUT_DIR
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, execFileSync } = require('node:child_process');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function main() {
  const [binary, dist, output] = process.argv.slice(2);
  assert(binary && dist && output, 'SERVER_BINARY SPA_DIST OUTPUT_DIR required');
  fs.mkdirSync(output, { recursive: true });
  const hash = crypto.createHash('sha256');
  for (const file of fs.readdirSync(dist, { recursive: true }).filter((name) => fs.statSync(path.join(dist, name)).isFile()).sort()) hash.update(fs.readFileSync(path.join(dist, file)));
  const build = JSON.parse(execFileSync(binary, ['--build-info'], { encoding: 'utf8' }));
  assert.equal(build.spa_sha256, hash.digest('hex'), 'Server must embed the tested SPA');
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-comments-'));
  fs.mkdirSync(path.join(root, '.opencoder'));
  fs.writeFileSync(path.join(root, '.opencoder/schedules.json'), JSON.stringify({ schedules: [] }));
  fs.writeFileSync(path.join(root, 'opencoder.json'), JSON.stringify({ agent: { agents_dir: path.join(root, 'agents'), share_dir: path.join(root, 'share') }, project: { todo_storage: { files_dir: path.join(root, 'files'), nfs: { port: 0 } } } }));
  const token = crypto.randomBytes(24).toString('hex');
  const credential = path.join(root, 'credential'); fs.writeFileSync(credential, token, { mode: 0o600 });
  const log = path.join(output, 'server.log'); const fd = fs.openSync(log, 'w');
  const server = spawn(binary, ['--web', '--workdir', root, '--data-dir', path.join(root, 'data'), '--port', '0', '--token-file', credential], { cwd: root, stdio: ['ignore', fd, fd] });
  fs.closeSync(fd);
  let browser;
  const report = { passed: false, build, checks: [], measurements: [] };
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
      const value = await response.json(); assert.equal(response.status, 200, JSON.stringify(value)); return value;
    }
    const todo = await api('POST', '/api/project/todos', { title: '评论维护验收', draft: 'Task description' });
    const route = `/api/project/todos/${todo.id}/comments`;
    const first = await api('POST', route, { request_id: 'first', body_md: 'Initial context' });
    const second = await api('POST', route, { request_id: 'second', body_md: 'Supplement' });
    browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage', '--no-proxy-server'] });
    const page = await browser.newPage({ viewport: { width: 1920, height: 1000 } });
    const errors = []; page.on('pageerror', (error) => errors.push(error.message));
    await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
    await page.goto(`${base}/#todo=${todo.id}`, { waitUntil: 'networkidle' });
    const drawer = page.getByRole('dialog', { name: 'TODO · 评论维护验收', exact: true });
    const article = drawer.locator(`[data-comment-seq="${first.seq}"]`);
    await article.getByRole('button', { name: '编辑评论', exact: true }).click();
    await article.getByLabel('编辑评论正文', { exact: true }).fill('Merged **context**');
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await article.getByRole('button', { name: '保存评论', exact: true }).scrollIntoViewIfNeeded();
      const measurement = await drawer.evaluate((element) => ({ width: innerWidth, page: document.documentElement.scrollWidth, drawer: element.clientWidth, content: element.scrollWidth }));
      assert(measurement.page <= width + 1 && measurement.content <= measurement.drawer + 1, JSON.stringify(measurement));
      report.measurements.push(measurement);
      await page.screenshot({ path: path.join(output, `comments-${width}.png`), animations: 'disabled' });
    }
    await article.getByRole('button', { name: '保存评论', exact: true }).click();
    await article.getByText('context', { exact: true }).waitFor();
    let comments = (await api('GET', route)).comments;
    assert.equal(comments.length, 2); assert.equal(comments[0].seq, first.seq);
    assert.equal(comments[0].body_md, 'Merged **context**'); assert.equal(comments[0].revision, 2);
    assert.equal(comments[0].created_at, first.created_at);
    report.checks.push('in-place edit preserves sequence, signature and creation time');
    await article.getByRole('button', { name: '编辑评论', exact: true }).click();
    await article.getByLabel('编辑评论正文', { exact: true }).fill('Unsaved draft');
    await api('PATCH', `${route}/${first.seq}`, { body_md: 'Remote update', expected_revision: 2 });
    await article.getByRole('button', { name: '保存评论', exact: true }).click();
    await drawer.getByText('评论已变化，请重新读取后再操作', { exact: true }).waitFor();
    assert.equal(await article.getByLabel('编辑评论正文', { exact: true }).inputValue(), 'Unsaved draft');
    await article.getByRole('button', { name: '取消编辑', exact: true }).click();
    await article.getByText('Remote update', { exact: true }).waitFor({ timeout: 15000 });
    report.checks.push('conflict preserves draft and polling shows remote edits');
    await drawer.locator(`[data-comment-seq="${second.seq}"]`).getByRole('button', { name: '删除评论', exact: true }).click();
    const confirmation = page.getByRole('dialog', { name: '删除这条评论？', exact: true });
    await confirmation.getByRole('button', { name: /删\s*除/ }).click();
    await drawer.locator(`[data-comment-seq="${second.seq}"]`).waitFor({ state: 'hidden' });
    assert.equal((await api('GET', route)).comments.length, 1);
    await api('DELETE', `${route}/${first.seq}?expected_revision=3`);
    await article.waitFor({ state: 'hidden', timeout: 15000 });
    report.checks.push('confirmed deletion and remote deletion refresh');
    await drawer.getByLabel('评论正文', { exact: true }).fill('One complete comment');
    await drawer.getByRole('button', { name: '发表评论', exact: true }).click();
    await drawer.getByText('One complete comment', { exact: true }).waitFor();
    comments = (await api('GET', route)).comments;
    assert.equal(comments.length, 1); assert.equal(comments[0].body_md, 'One complete comment');
    assert.equal((await api('GET', `/api/project/todos/${todo.id}`)).board_status, 'backlog');
    assert.deepEqual(errors, []);
    report.checks.push('append remains independent of TODO board state');
    report.passed = true;
  } catch (error) { report.error = error.stack; throw error; }
  finally {
    fs.writeFileSync(path.join(output, 'result.json'), JSON.stringify(report, null, 2));
    await browser?.close();
    if (server.exitCode === null) {
      server.kill('SIGTERM');
      await Promise.race([new Promise((resolve) => server.once('exit', resolve)), pause(10000)]);
      if (server.exitCode === null) server.kill('SIGKILL');
    }
  }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
