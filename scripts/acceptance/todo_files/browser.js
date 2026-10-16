// Public TODO page acceptance. Credentials are read locally and never logged.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');

async function main() {
  const [base, credential, id, output] = process.argv.slice(2);
  const token = fs.readFileSync(credential, 'utf8').trim();
  const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || '/usr/bin/chromium',
    args: ['--no-sandbox', '--disable-dev-shm-usage'] });
  try {
    const page = await browser.newPage({ viewport: { width: 1920, height: 1000 }, acceptDownloads: true });
    const errors = []; const layouts = [];
    page.on('pageerror', (error) => errors.push(error.message));
    await page.addInitScript((value) => localStorage.setItem('oc_token', value), token);
    await page.goto(`${base}/#todo=${id}`, { waitUntil: 'networkidle' });
    const drawer = page.locator('[role=dialog].project-todo-drawer');
    await drawer.getByLabel('关键信息预览', { exact: true }).waitFor();
    await drawer.getByLabel('TODO 标题', { exact: true }).fill('验收未保存标题');
    await drawer.getByLabel('评论正文', { exact: true }).fill('验收未提交评论');
    await drawer.getByRole('tab', { name: '文件', exact: true }).click();
    const files = drawer.getByRole('region', { name: '任务文件' });
    await files.getByText(/存储目录/).waitFor();
    const content = Buffer.from('# 交接\n\n当前进展与下一步。\n');
    await files.getByLabel('选择任务文件').setInputFiles({ name: '说明.md', mimeType: 'text/markdown', buffer: content });
    await files.getByLabel('文件存储路径').fill('handoff/说明.md');
    const uploaded = page.waitForResponse((r) => r.request().method() === 'PUT' && r.url().includes('/file?'));
    await files.getByRole('button', { name: '上传文件', exact: true }).click();
    assert.equal((await uploaded).status(), 201);
    const row = files.getByRole('row').filter({ hasText: 'handoff/说明.md' });
    await row.waitFor();
    const downloading = page.waitForEvent('download');
    await row.getByRole('button', { name: /下\s*载/ }).click();
    const download = await downloading;
    assert.deepEqual(fs.readFileSync(await download.path()), content);
    for (const width of [1920, 1280, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 });
      await page.waitForFunction(() => {
        const element = document.querySelector('[role=dialog].project-todo-drawer');
        return element && Math.abs(element.clientWidth - innerWidth) <= 1;
      });
      const layout = await drawer.evaluate((element) => ({ width: innerWidth,
        page: document.documentElement.scrollWidth, drawer: element.clientWidth, content: element.scrollWidth }));
      assert(layout.page <= width + 1 && layout.content <= layout.drawer + 1, JSON.stringify(layout));
      layouts.push(layout);
      await page.screenshot({ path: path.join(output, `public-files-${width}.png`), animations: 'disabled' });
      await drawer.getByRole('tab', { name: '任务', exact: true }).click();
      assert.equal(await drawer.getByLabel('TODO 标题', { exact: true }).inputValue(), '验收未保存标题');
      assert.equal(await drawer.getByLabel('评论正文', { exact: true }).inputValue(), '验收未提交评论');
      await drawer.getByRole('tab', { name: '文件', exact: true }).click();
    }
    assert.deepEqual(errors, []);
    fs.writeFileSync(path.join(output, 'browser.json'), JSON.stringify({ result: 'PASS', layouts, errors,
      file: 'handoff/说明.md', sha256: crypto.createHash('sha256').update(content).digest('hex') }, null, 2));
  } finally { await browser.close(); }
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
