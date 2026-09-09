// Exercise the deployed SPA against real, persisted Runner results.
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { createHash } from 'node:crypto';
import { pathToFileURL } from 'node:url';
const [root, capability] = process.argv.slice(2);
process.env.PLAYWRIGHT_BROWSERS_PATH = '/root/.cache/ms-playwright';
const read = async path => JSON.parse(await readFile(path, 'utf8'));
const { chromium } = await import(pathToFileURL('/root/workspace/tools/eval-diagnose-cli/node_modules/playwright/index.mjs'));
const { server } = await read(`${root}/runtime/endpoints.json`);
const token = (await readFile(`${root}/runtime/control-token`, 'utf8')).trim();
const jobs = await read(`${root}/evidence/jobs.json`);
if (capability && !Object.hasOwn(jobs, capability)) throw Error('Unknown business capability');
await mkdir(`${root}/evidence/browser`, { recursive: true });
const browser = await chromium.launch({ headless: true, args: ['--no-sandbox'] });
const context = await browser.newContext({ viewport: { width: 1500, height: 1000 }, acceptDownloads: true });
await context.addInitScript(value => {
  localStorage.setItem('oc_token', value); localStorage.setItem('oc_base', '');
}, token);
const page = await context.newPage();
const errors = [];
page.on('pageerror', error => errors.push(error.message));
page.on('response', response => {
  if (response.status() >= 400) errors.push(`${response.status()} ${new URL(response.url()).pathname}`);
});
const report = {};
async function open(id) {
  await page.goto(server, { waitUntil: 'networkidle' });
  await page.getByText('Agent', { exact: true }).first().click();
  await page.getByRole('menuitem', { name: '全部执行' }).click();
  await page.getByRole('button', { name: id, exact: true }).click();
  await page.getByText('业务任务', { exact: true }).waitFor();
}
async function unfold() {
  const labels = [];
  for (const pattern of [/\d+ Steps?/, /Step\(\d+\)/, /Function call/]) {
    const row = page.locator('.ant-collapse-header').filter({ hasText: pattern }).first();
    await row.waitFor();
    labels.push(await row.innerText());
    if (await row.getAttribute('aria-expanded') !== 'true') await row.click();
  }
  // A tool row below the aggregate must expose its actual command/result on demand.
  const tool = page.locator('.ant-collapse-header').filter({ hasText: /bash|exec_command|shell|command/ }).last();
  await tool.waitFor();
  if (await tool.getAttribute('aria-expanded') !== 'true') await tool.click();
  const expanded = tool.locator('..');
  await expanded.getByText('input:', { exact: true }).waitFor();
  labels.push(await tool.innerText());
  return labels;
}
try {
  for (const name of capability ? [capability] : Object.keys(jobs)) {
    const business = await read(`${root}/evidence/${name}/business.json`);
    const detail = await read(`${root}/evidence/${name}/execution.json`);
    await open(business.execution.id);
    const folds = await unfold();
    await page.screenshot({ path: `${root}/evidence/browser/${name}-folds.png`, fullPage: true });
    const artifact = detail.runners[0].artifacts.find(a => /(?:^|\/)report\.html$/.test(a.file))
      || detail.runners[0].artifacts.find(a => a.file === 'result.json');
    const downloaded = new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('Artifact download did not start')), 30000);
      const handle = value => { clearTimeout(timer); resolve(value); };
      page.once('download', handle);
      context.once('page', popup => popup.once('download', handle));
    });
    await page.getByRole('button', { name: artifact.file, exact: true }).click();
    const download = await downloaded;
    const target = `${root}/evidence/browser/${name}-download.${artifact.file.split('.').at(-1)}`;
    await download.saveAs(target);
    const hash = createHash('sha256').update(await readFile(target)).digest('hex');
    if (hash !== artifact.sha256) throw Error('Browser download does not match Runner manifest');
    await page.reload({ waitUntil: 'networkidle' });
    await open(business.execution.id);
    const replay = await unfold();
    await page.screenshot({ path: `${root}/evidence/browser/${name}-replay.png`, fullPage: true });
    report[name] = { executionId: business.execution.id, folds, replay,
      artifact: artifact.file, sha256: hash, refreshedReplay: true };
  }
  if (errors.length) throw Error(JSON.stringify(errors));
  report.passed = true;
  await writeFile(`${root}/evidence/${capability ? `browser-${capability}` : 'browser'}.json`, JSON.stringify(report, null, 2));
} catch (error) {
  await page.screenshot({ path: `${root}/evidence/browser/failure.png`, fullPage: true });
  await writeFile(`${root}/evidence/browser/failure.txt`, `${error.stack}\n${await page.locator('body').innerText()}`);
  throw error;
} finally { await browser.close(); }
