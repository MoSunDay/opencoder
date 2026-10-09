const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');
const { compile } = require('../../harness/native');

const digest = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
const source = (version) => `#include <stdio.h>
#include <string.h>
#include <unistd.h>
int main(int count, char **arguments) {
  char directory[512], value[128];
  if (!getcwd(directory, sizeof(directory)) || count != 2) return 2;
  if (strcmp(arguments[1], "write") == 0) {
    FILE *seed = fopen("../seed.txt", "w");
    if (!seed) return 3;
    fputs("private-cow", seed); fclose(seed);
    FILE *shared = fopen("../shared.txt", "w");
    if (!shared) return 4;
    fputs("shared-value", shared); fclose(shared);
  } else {
    FILE *seed = fopen("../seed.txt", "r");
    if (!seed || !fgets(value, sizeof(value), seed) || strcmp(value, "private-cow")) return 5;
    fclose(seed);
    FILE *shared = fopen("../shared.txt", "r");
    if (!shared || !fgets(value, sizeof(value), shared) || strcmp(value, "shared-value")) return 6;
    fclose(shared);
  }
  FILE *output = fopen("output.json", "w");
  if (!output) return 7;
  fprintf(output, "{\\"version\\":${version},\\"uid\\":%u,\\"cwd\\":\\"%s\\"}", (unsigned)getuid(), directory);
  fclose(output); puts("native-version-${version}"); return 0;
}`;

async function resources({ page, api, root, until }) {
  const name = 'ui-native-tool';
  const binaries = [1, 2].map((version) => compile(path.join(root, 'ui-native-build'), `tool-${version}`, source(version)));
  await page.getByRole('menuitem', { name: 'DAG 工作流' }).click();
  await page.getByRole('tab', { name: '二进制资源', exact: true }).click();
  await page.getByRole('button', { name: '上传二进制', exact: true }).click();
  const modal = page.locator('.ant-modal');
  await modal.getByLabel('二进制资源名称', { exact: true }).fill(name);
  await modal.getByLabel('二进制资源说明').fill('UI native workspace acceptance');
  await modal.getByLabel('Linux 可执行文件').setInputFiles({ name: 'tool-v1', mimeType: 'application/octet-stream', buffer: binaries[0] });
  await modal.getByRole('button', { name: '保存二进制', exact: true }).click();
  const row = page.getByRole('row').filter({ has: page.getByRole('button', { name, exact: true }) });
  await row.waitFor();
  await row.getByRole('button', { name: '追加版本', exact: true }).click();
  await modal.getByLabel('Linux 可执行文件').setInputFiles({ name: 'tool-v2', mimeType: 'application/octet-stream', buffer: binaries[1] });
  await modal.getByRole('button', { name: '保存二进制', exact: true }).click();
  await until(async () => (await api('GET', `/api/dag/binaries/${name}`)).current === 2, 'binary version two');
  await row.getByRole('button', { name, exact: true }).click();
  const history = page.locator('.ant-drawer').filter({ hasText: `二进制版本 · ${name}` });
  const downloaded = page.waitForEvent('download');
  await history.getByRole('button', { name: '下载 v2', exact: true }).click();
  const download = await downloaded;
  const downloadPath = path.join(root, 'downloaded-tool-v2');
  await download.saveAs(downloadPath);
  assert.equal(digest(fs.readFileSync(downloadPath)), digest(binaries[1]));
  await history.getByRole('button', { name: '使用 v1', exact: true }).click();
  await modal.getByRole('button', { name: '确认切换版本', exact: true }).click();
  await until(async () => (await api('GET', `/api/dag/binaries/${name}`)).current === 1, 'current pointer rollback');
  assert.deepEqual((await api('GET', `/api/dag/binaries/${name}`)).history.map((version) => version.version), [1, 2]);
  await history.locator('.ant-drawer-close').click();

  const workspace = path.join(root, 'native-source/workspace');
  const seed = path.join(workspace, 'seed.txt');
  fs.writeFileSync(seed, 'host-original', { mode: 0o666 });
  const stat = fs.statSync(seed);
  const hostBefore = { bytes: fs.readFileSync(seed, 'utf8'), inode: stat.ino, mode: stat.mode, uid: stat.uid, gid: stat.gid };
  await page.getByRole('tab', { name: '定义', exact: true }).click();
  await page.getByRole('button', { name: '新建定义', exact: true }).click();
  const editor = page.locator('.ant-drawer').filter({ hasText: '新建工作流定义' });
  await editor.getByText('JSON', { exact: true }).click();
  const spec = { name: 'ui-native-workspace', steps: [
    { name: 'writer', kind: { type: 'binary', resource: `${name}@v1`, args: ['write'] } },
    { name: 'reader', depends_on: ['writer'], kind: { type: 'binary', resource: `${name}@v1`, args: ['read'] } },
  ] };
  await editor.locator('textarea').fill(JSON.stringify(spec));
  await editor.getByText('画布', { exact: true }).click();
  await editor.locator('.react-flow__node[data-id="writer"]').click();
  await editor.getByRole('combobox', { name: '二进制资源版本', exact: true }).waitFor();
  await editor.getByText('JSON', { exact: true }).click();
  assert.deepEqual(JSON.parse(await editor.locator('textarea').inputValue()), spec);
  const saved = page.waitForResponse((response) => response.url().endsWith('/api/dag/defs') && response.request().method() === 'POST');
  await editor.getByRole('button', { name: /^保\s*存$/ }).click();
  assert((await saved).ok());
  await editor.waitFor({ state: 'hidden' });
  const definition = (await api('GET', '/api/dag/defs')).find((item) => item.name === spec.name);
  assert(definition);
  await page.getByRole('row').filter({ hasText: spec.name }).getByRole('button', { name: /^派\s*发$/ }).click();
  const dispatch = page.waitForResponse((response) => response.url().endsWith(`/api/dag/defs/${definition.id}/dispatch`) && response.request().method() === 'POST');
  await page.getByRole('button', { name: '确认派发', exact: true }).click();
  const run = await (await dispatch).json();
  assert(run.run_id);
  await until(async () => {
    const detail = await api('GET', `/api/executions/${run.run_id}`);
    assert(!['error', 'interrupted', 'cancelled'].includes(detail.execution.status), JSON.stringify(detail));
    return detail.execution.status === 'done';
  }, 'shared workspace DAG completed', 120000);
  const detail = await api('GET', `/api/executions/${run.run_id}`);
  assert.equal(detail.dag_context.state, 'ready');
  assert.equal(detail.dag_context.container_id, `dag-run-${run.run_id}`);
  const output = [];
  for (const step of ['writer', 'reader']) {
    const pin = detail.dag_context.steps.find((entry) => entry.name === step);
    assert.equal(pin.cwd, `/workspace/${step}`);
    assert.equal(pin.resource.version, 1);
    assert.equal(pin.resource.sha256, digest(binaries[0]));
    const artifact = await api('POST', `/api/executions/${run.run_id}/commands`, { action: 'artifact', input: { step, file: 'output.json' } });
    output.push(JSON.parse(Buffer.from(artifact.bytes_b64, 'base64').toString()));
  }
  assert.equal(output[0].uid, output[1].uid);
  assert.deepEqual(output.map((value) => value.cwd), ['/workspace/writer', '/workspace/reader']);
  assert.deepEqual(output.map((value) => value.version), [1, 1]);
  const after = fs.statSync(seed);
  assert.deepEqual({ bytes: fs.readFileSync(seed, 'utf8'), inode: after.ino, mode: after.mode, uid: after.uid, gid: after.gid }, hostBefore);
  assert(!fs.existsSync(path.join(workspace, 'shared.txt')));
  await page.getByText(detail.dag_context.container_id, { exact: true }).waitFor();
  assert.equal(await page.getByText('全部步骤共享本次 DAG 容器', { exact: true }).count(), 0);
  await page.screenshot({ path: path.join(root, 'native-workspace-context.png'), animations: 'disabled' });
  await page.getByRole('button', { name: '← 返回运行列表', exact: true }).click();
  await page.getByRole('tab', { name: '二进制资源', exact: true }).click();
  await row.getByRole('button', { name, exact: true }).click();
  await history.getByRole('button', { name: '使用 v2', exact: true }).click();
  await modal.getByRole('button', { name: '确认切换版本', exact: true }).click();
  await until(async () => (await api('GET', `/api/dag/binaries/${name}`)).current === 2, 'new current pointer');
  await history.locator('.ant-drawer-close').click();
  await row.getByRole('button', { name: '删除资源', exact: true }).click();
  await modal.getByRole('button', { name: '确认删除资源', exact: true }).click();
  await until(async () => !(await api('GET', '/api/dag/binaries')).pools.some((pool) => pool.name === name), 'resource removed');
  assert.deepEqual((await api('GET', `/api/executions/${run.run_id}`)).dag_context, detail.dag_context);
  fs.writeFileSync(path.join(root, 'native-workspace.json'), JSON.stringify({ run_id: run.run_id, context: detail.dag_context, output, source_unchanged: true, downloaded_sha256: digest(binaries[1]) }, null, 2));
  return definition.id;
}

module.exports = { resources };
