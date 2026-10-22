import assert from 'node:assert/strict';
import { writeFile } from 'node:fs/promises';
import { chromium } from '../../../crates/web/spa/node_modules/playwright-core/index.mjs';

// Focused rendering checks against Vite; functional checks use graph.py and a real Server.
const [base, output] = process.argv.slice(2);
const browser = await chromium.launch({ executablePath: process.env.CHROME_PATH || chromium.executablePath(),
  args: ['--no-sandbox', '--disable-dev-shm-usage', '--use-angle=swiftshader', '--enable-unsafe-swiftshader'] });
const html = `<!doctype html><html><head><meta charset="UTF-8"></head><body style="margin:0"><div id="preview"></div><script type="module">
import RefreshRuntime from '/@react-refresh';
RefreshRuntime.injectIntoGlobalHook(window); window.$RefreshReg$ = () => {}; window.$RefreshSig$ = () => (type) => type;
window.__vite_plugin_react_preamble_installed__ = true;
const React = (await import('/node_modules/.vite/deps/react.js')).default;
const { createRoot } = (await import('/node_modules/.vite/deps/react-dom_client.js')).default;
const { ConfigProvider, theme } = await import('/node_modules/.vite/deps/antd.js');
const { default: GraphCanvas } = await import('/src/ontology/pages/graph/GraphCanvas.tsx');
await import('/src/ontology/pages/graph/workspace.css');
const nodes = ['a','b','c'].map((id) => ({ id, env_num: 1, entity_type_id: id, name: '组件 ' + id, description: '', revision: 1, is_deleted: false }));
const data = { nodes, edges: [{ id: 'edge', env_num: 1, relationship_type_id: 'rel', source_entity_id: 'a', target_entity_id: 'b', description: '', revision: 1, is_deleted: false, is_pinned: false }] };
const root = createRoot(document.getElementById('preview'));
window.renderPreview = (dark) => root.render(React.createElement(ConfigProvider, { theme: { algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm } },
 React.createElement('div', { style: { height: '100vh', display: 'flex', padding: 8, boxSizing: 'border-box' } },
 React.createElement(GraphCanvas, { data, centerIds: ['a'], entityTypeNames: {a:'组件',b:'资源',c:'接口'}, relationshipTypeNames:{rel:'完整关系名称：通过已确认的接口表达组件之间的依赖，不省略文字'}, onNodeClick: () => {}, onEdgeClick: () => {} }))));
window.renderPreview(false);
</script></body></html>`;
const page = await browser.newPage({ viewport: { width: 1280, height: 850 } });
const errors = []; page.on('pageerror', (error) => errors.push(error.message));
await page.route(`${base}/__graph_visual`, (route) => route.fulfill({ contentType: 'text/html; charset=utf-8', body: html }));
try {
  await page.goto(`${base}/__graph_visual`);
  await page.locator('.graph-3d[aria-busy="false"]').waitFor({ timeout: 30000 });
  const colors = [];
  for (const dark of [false, true]) {
    await page.evaluate((value) => window.renderPreview(value), dark);
    for (const width of [1280, 390]) {
      await page.setViewportSize({ width, height: 850 });
      await page.getByRole('button', { name: '自动取景' }).click(); await page.mouse.move(0, 0); await page.waitForTimeout(200);
      const styles = await page.locator('.graph-surface').evaluate((element) => ({ background: getComputedStyle(element).backgroundColor, text: getComputedStyle(element.querySelector('.graph-3d-name')).color }));
      colors.push({ dark, width, ...styles }); assert.notEqual(styles.background, styles.text);
      const text = await page.locator('.graph-3d-relation').evaluate((element) => ({ height: element.clientHeight, scroll: element.scrollHeight, text: element.textContent }));
      assert.equal(text.height, text.scroll); assert(text.text.endsWith('不省略文字'));
      await page.screenshot({ path: `${output}/graph-${dark ? 'dark' : 'light'}-${width}.png` });
    }
  }
  assert.notEqual(colors[0].background, colors[2].background); assert.deepEqual(errors, []);
  const unsupported = await browser.newPage();
  await unsupported.addInitScript(() => {
    const original = HTMLCanvasElement.prototype.getContext;
    HTMLCanvasElement.prototype.getContext = function(type, ...args) { return String(type).includes('webgl') ? null : original.call(this, type, ...args); };
  });
  await unsupported.route(`${base}/__graph_visual`, (route) => route.fulfill({ contentType: 'text/html; charset=utf-8', body: html }));
  await unsupported.goto(`${base}/__graph_visual`);
  await unsupported.getByText('三维画布暂不可用，已显示完整列表').waitFor();
  assert.equal(await unsupported.locator('.graph-result-list tbody tr[data-row-key]').count(), 3);
  assert.equal(await unsupported.locator('.graph-3d').count(), 0);
  await unsupported.close();
  await writeFile(`${output}/visual.json`, JSON.stringify({ passed: true, colors, complete_labels: true, webgl_unavailable: true }, null, 2));
  console.log(JSON.stringify({ passed: true, themes: 2, widths: 2, webgl_unavailable: true }));
} catch (error) { await page.screenshot({ path: `${output}/visual-failure.png` }); console.error(errors); throw error; }
finally { await browser.close(); }
