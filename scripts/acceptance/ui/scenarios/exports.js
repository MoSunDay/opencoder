const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');

async function exportsStatus({ page, api, root, until }) {
  await page.getByRole('menuitem', { name: /Agent 配置$/ }).click();
  await page.getByRole('tab', { name: 'NFS 配置', exact: true }).click();
  const states = [];
  for (const [route, label] of [['/api/agents/nfs', 'nfs'], ['/api/dag/binaries/nfs', 'binary-nfs'], ['/api/dag/workspace/nfs', 'workspace-nfs']]) {
    const value = await api('GET', route);
    const toggle = page.getByRole('switch', { name: `${label}-enabled`, exact: true });
    await toggle.waitFor();
    await until(async () => !(await toggle.isDisabled()), `${label} read actual status`);
    assert.equal(await toggle.getAttribute('aria-checked'), String(value.status?.running || false));
    const exportRoot = value.root || value.status?.export_root;
    if (exportRoot) await page.getByText(exportRoot, { exact: true }).waitFor();
    states.push({ endpoint: route, running: !!value.status?.running, root: exportRoot });
  }
  const binary = page.getByRole('switch', { name: 'binary-nfs-enabled', exact: true });
  await binary.click();
  await page.locator('.ant-modal').getByRole('button', { name: /^取\s*消$/ }).click();
  assert.equal((await api('GET', '/api/dag/binaries/nfs')).status.running, true);
  await binary.click();
  await page.getByRole('button', { name: '确认停止导出', exact: true }).click();
  await until(async () => !(await api('GET', '/api/dag/binaries/nfs')).status?.running, 'binary export stopped');
  await until(async () => await binary.getAttribute('aria-checked') === 'false' && !(await binary.isDisabled()), 'stop state reflected');
  await binary.click();
  await until(async () => (await api('GET', '/api/dag/binaries/nfs')).status?.running, 'binary export restored');
  await until(async () => await binary.getAttribute('aria-checked') === 'true', 'start state reflected');
  await page.screenshot({ path: path.join(root, 'nfs-exports.png'), animations: 'disabled' });
  fs.writeFileSync(path.join(root, 'nfs-ui.json'), JSON.stringify({ states, confirmation_cancel_preserved: true, restart_verified: true }, null, 2));
}

module.exports = { exportsStatus };
