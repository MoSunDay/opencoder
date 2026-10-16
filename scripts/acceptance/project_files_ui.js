// Real isolated Server + embedded SPA: TODO handoff files, download and layout.
// node scripts/acceptance/project_files_ui.js SERVER_BINARY SPA_DIST OUTPUT_DIR
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
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-todo-files-'));
  fs.mkdirSync(path.join(root, '.opencoder'));
  fs.writeFileSync(path.join(root, '.opencoder/schedules.json'), JSON.stringify({ schedules: [] }));
  fs.writeFileSync(path.join(root, 'opencoder.json'), JSON.stringify({
    agent: { agents_dir: path.join(root, 'agents'), share_dir: path.join(root, 'share') },
    project: { todo_storage: { files_dir: path.join(root, 'files'), nfs: { port: 0 } } },
  }));
  const token = crypto.randomBytes(24).toString('hex');
  const credential = path.join(root, 'credential'); fs.writeFileSync(credential, token, { mode: 0o600 });
  const log = path.join(output, 'server.log'); const fd = fs.openSync(log, 'w');
  const server = spawn(binary, ['--web', '--workdir', root, '--data-dir', path.join(root, 'data'), '--port', '0', '--token-file', credential], { cwd: root, stdio: ['ignore', fd, fd] });
  fs.closeSync(fd);
  let browser; let page; let proxy;
  try {
    let base;
    const deadline = Date.now() + 45000;
    while (!base && Date.now() < deadline) {
      assert.equal(server.exitCode, null, 'Server exited; see server.log');
      base = fs.readFileSync(log, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/)?.[1];
      if (!base) await pause(100);
    }
    assert(base, 'Server did not start');
    // Exercise the generated public proxy, including uploads above its default 2 MiB limit.
    async function freePort() {
      const listener = require('node:net').createServer();
      await new Promise((resolve) => listener.listen(0, '127.0.0.1', resolve));
      const port = listener.address().port; await new Promise((resolve) => listener.close(resolve)); return port;
    }
    const proxyPort = await freePort(); const hostPort = await freePort();
    const generated = execFileSync('python', ['-c',
      'import sys; from types import SimpleNamespace; sys.path.insert(0,sys.argv[1]); from rolling import units; print(units.nginx(SimpleNamespace(listen="127.0.0.1:"+sys.argv[2],host_port=int(sys.argv[3])),int(sys.argv[4]),1))',
      path.resolve(__dirname, '../platform'), String(proxyPort), String(hostPort), new URL(base).port], { encoding: 'utf8' });
    const proxyConfig = path.join(root, 'nginx.conf');
    fs.writeFileSync(proxyConfig, `pid ${root}/nginx.pid; error_log ${output}/nginx.log; events {} http { ${generated} }`);
    proxy = spawn(process.env.NGINX_PATH || 'nginx', ['-p', root, '-c', proxyConfig, '-g', 'daemon off;'], { stdio: ['ignore', 'ignore', 'ignore'] });
    base = `http://127.0.0.1:${proxyPort}`;
    const proxyDeadline = Date.now() + 10000;
    let ready = false;
    while (!ready && Date.now() < proxyDeadline) {
      assert.equal(proxy.exitCode, null, 'Nginx exited; see nginx.log');
      try { ready = (await fetch(`${base}/api/health`, { headers: { authorization: `Bearer ${token}` } })).ok; } catch {}
      if (!ready) await pause(100);
    }
    assert(ready, 'public proxy did not start');
    async function api(method, route, body, expected = 200) {
      const response = await fetch(base + route, { method, headers: { authorization: `Bearer ${token}`, 'content-type': 'application/json' }, body: body == null ? undefined : JSON.stringify(body) });
      const data = await response.json(); assert.equal(response.status, expected, JSON.stringify(data)); return data;
    }
    const todo = await api('POST', '/api/project/todos', { title: '任务文件验收', draft: '## 当前进展\n\n交接信息' });
    browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(), args: ['--no-sandbox', '--disable-dev-shm-usage'] });
    page = await browser.newPage({ viewport: { width: 1920, height: 1000 }, acceptDownloads: true });
    const errors = []; const measurements = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
    await page.goto(`${base}/#todo=${todo.id}`, { waitUntil: 'networkidle' });
    const drawer = page.getByRole('dialog', { name: 'TODO · 任务文件验收', exact: true });
    await drawer.getByLabel('关键信息预览', { exact: true }).waitFor();
    await drawer.getByLabel('TODO 标题', { exact: true }).fill('未保存的标题');
    await drawer.getByLabel('评论正文', { exact: true }).fill('未提交的评论');
    await drawer.getByRole('tab', { name: '文件', exact: true }).click();
    const files = drawer.getByRole('region', { name: '任务文件' });
    await files.getByText(/存储目录/).waitFor();
    const content = Buffer.alloc(3 * 1024 * 1024, 0x7b);
    await files.getByLabel('选择任务文件').setInputFiles({ name: '过程.bin', mimeType: 'application/octet-stream', buffer: content });
    await files.getByLabel('文件存储路径').fill('日志/过程.bin');
    const uploaded = page.waitForResponse((r) => r.request().method() === 'PUT' && r.url().includes('/file?'));
    await files.getByRole('button', { name: '上传文件', exact: true }).click();
    assert.equal((await uploaded).status(), 201);
    await files.getByText('日志/过程.bin', { exact: true }).waitFor();
    const downloading = page.waitForEvent('download');
    await files.getByRole('button', { name: /下\s*载/ }).click();
    const download = await downloading;
    assert.equal(download.suggestedFilename(), '过程.bin');
    assert.deepEqual(fs.readFileSync(await download.path()), content);
    await api('DELETE', `/api/project/todos/${todo.id}`, undefined, 409);
    console.log('PASS binary upload/download through Nginx and TODO deletion guard');
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await page.waitForFunction(() => { const drawer = document.querySelector('[role=dialog].project-todo-drawer'); return drawer && Math.abs(drawer.clientWidth - innerWidth) <= 1; });
      const measure = await drawer.evaluate((element) => ({ width: innerWidth, page: document.documentElement.scrollWidth, drawer: element.clientWidth, content: element.scrollWidth }));
      assert(measure.page <= width + 1 && measure.content <= measure.drawer + 1, JSON.stringify(measure));
      measurements.push(measure);
      await page.screenshot({ path: path.join(output, `files-${width}.png`), animations: 'disabled' });
      await drawer.getByRole('tab', { name: '任务', exact: true }).click();
      assert.equal(await drawer.getByLabel('TODO 标题', { exact: true }).inputValue(), '未保存的标题');
      assert.equal(await drawer.getByLabel('评论正文', { exact: true }).inputValue(), '未提交的评论');
      await page.screenshot({ path: path.join(output, `task-${width}.png`), animations: 'disabled' });
      await drawer.getByRole('tab', { name: '文件', exact: true }).click();
    }
    await page.setViewportSize({ width: 1280, height: 1000 });
    // A duplicate upload must preserve the selection and the existing file.
    await files.getByLabel('选择任务文件').setInputFiles({ name: '过程.bin', mimeType: 'application/octet-stream', buffer: Buffer.from('new') });
    await files.getByLabel('文件存储路径').fill('日志/过程.bin');
    await files.getByRole('button', { name: '上传文件', exact: true }).click();
    await files.getByText(/file already exists/).waitFor();
    assert.equal(await files.getByLabel('文件存储路径').inputValue(), '日志/过程.bin');
    await files.getByRole('button', { name: '取消选择' }).click();
    await files.getByRole('button', { name: /删\s*除/ }).click();
    await page.getByRole('button', { name: /删\s*除/ }).last().click();
    await files.getByText('日志/过程.bin', { exact: true }).waitFor({ state: 'hidden' });
    assert.deepEqual((await api('GET', `/api/project/todos/${todo.id}/files`)).files, []);
    await drawer.getByRole('tab', { name: '任务', exact: true }).click();
    const saved = page.waitForResponse((r) => r.request().method() === 'PATCH' && r.url().endsWith(todo.id));
    await drawer.getByRole('button', { name: '保存 TODO', exact: true }).click();
    assert.equal((await saved).status(), 200);
    assert.equal((await api('GET', `/api/project/todos/${todo.id}`)).title, '未保存的标题');
    await api('DELETE', `/api/project/todos/${todo.id}`);
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'receipt.json'), JSON.stringify({ result: 'PASS', server_spa_sha256: build.spa_sha256, measurements,
      binary_roundtrip_bytes: content.length, duplicate_preserves_file: true, todo_delete_guard: true, drafts_preserved: true, page_errors: errors }, null, 2));
    console.log(`PASS: TODO file upload/download and responsive layout; ${output}`);
  } catch (error) {
    if (page) { await page.screenshot({ path: path.join(output, 'failure.png'), animations: 'disabled' }).catch(() => {}); fs.writeFileSync(path.join(output, 'failure.html'), await page.content()); }
    throw error;
  } finally {
    await browser?.close();
    if (proxy && proxy.exitCode === null && !proxy.signalCode) {
      const stopped = new Promise((resolve) => proxy.once('exit', resolve)); proxy.kill('SIGTERM'); await stopped;
    }
    if (server.exitCode === null && !server.signalCode) {
      const stopped = new Promise((resolve) => server.once('exit', resolve)); server.kill('SIGTERM');
      const force = setTimeout(() => server.kill('SIGKILL'), 10000); await stopped; clearTimeout(force);
    }
    fs.rmSync(credential, { force: true });
  }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
