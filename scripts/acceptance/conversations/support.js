const assert = require('node:assert/strict');

function options(argv) {
  assert.equal(argv.length % 2, 0, 'arguments must be --name value pairs');
  return Object.fromEntries(Array.from({ length: argv.length / 2 }, (_, i) => [argv[2 * i], argv[2 * i + 1]]));
}

async function api(page, route) {
  return page.evaluate(async route => {
    const response = await fetch(route, {
      headers: { authorization: `Bearer ${localStorage.getItem('oc_token')}` },
      signal: AbortSignal.timeout(60000),
    });
    return { status: response.status, body: await response.json() };
  }, route);
}

async function ready(page, nodeName) {
  const health = await api(page, '/api/ready');
  assert.equal(health.status, 200, `entry unavailable: ${JSON.stringify(health.body)}`);
  assert.equal(health.body.mode, 'open');
  const nodes = await api(page, '/api/nodes');
  assert.equal(nodes.status, 200);
  const matches = nodes.body.nodes.filter(n => n.name === nodeName);
  assert.equal(matches.length, 1, 'execution node name must resolve uniquely');
  const node = matches[0];
  assert(node.online && node.snapshot?.ready, `node not ready: ${JSON.stringify(node.snapshot)}`);
  return node;
}

async function enter(page, base, mode, nodeName) {
  await page.goto(base, { waitUntil: 'networkidle' });
  await page.locator('.fleet-nav-category').getByText('Agent', { exact: true }).click();
  await page.getByRole('menuitem', { name: /Agent$/ }).click();
  await page.getByText(mode === 'agent' ? 'Agent 模式' : 'Operator 模式', { exact: true }).click();
  const node = await ready(page, nodeName);
  assert(node.kinds.includes(mode), `node does not support ${mode}`);
  const combo = page.getByRole('combobox', { name: '执行节点', exact: true });
  await combo.click(); await combo.fill(nodeName);
  const option = page.locator('.ant-select-item-option').filter({
    has: page.locator('.ant-select-item-option-content').getByText(nodeName, { exact: true }),
  });
  assert(await option.evaluate(el => el.getAttribute('aria-disabled') !== 'true'
    && !el.classList.contains('ant-select-item-option-disabled')), 'node option disabled');
  const history = page.waitForResponse(r => {
    const url = new URL(r.url());
    return url.pathname === `/api/nodes/${node.id}/dialogs` && url.searchParams.get('kind') === mode;
  }, { timeout: 60000 }).catch(error => ({ error }));
  await option.click();
  await page.getByPlaceholder('输入提示词，Enter 发送，Shift+Enter 换行').waitFor();
  await page.waitForFunction(() => !document.querySelector('.ant-sender textarea')?.disabled);
  const response = await history;
  if (response.error) throw response.error;
  assert(response.ok(), 'history load failed');
  page.conversationRows = (await response.json()).dialogs;
  await page.waitForFunction(count => document.querySelectorAll('.oc-chat-sidebar .ant-conversations-item').length === count,
    page.conversationRows.length);
  console.log('UI NODE SELECTED', mode, nodeName, page.conversationRows.length, 'conversations');
  return node;
}

async function chooseAgent(page, mode, name) {
  const sender = page.getByPlaceholder('输入提示词，Enter 发送，Shift+Enter 换行');
  if (mode === 'agent') {
    const cards = (await api(page, '/api/agents')).body.agents.filter(a => a.primary);
    const index = cards.findIndex(a => a.name === name); assert(index >= 0, 'registered Agent missing');
    const combo = page.getByRole('combobox', { name: '执行 Agent', exact: true });
    await combo.click();
    for (let i = 0; i < index; i++) await combo.press('ArrowDown');
    await page.locator('.ant-select-item-option-content').getByText(name, { exact: true }).click();
  } else {
    await sender.fill(`@${name}`);
    await page.locator(`[data-cmd="@${name}"]`).click();
  }
  return sender;
}

async function reopen(page, node, mode, id) {
  const rows = page.conversationRows;
  assert(Array.isArray(rows), 'enter the selected node before reopening history');
  const row = rows.find(d => d.session_id === id); assert(row, 'conversation missing from history');
  const label = row.title || id.slice(0, 12) + '…';
  const candidates = page.locator('.oc-chat-sidebar .ant-conversations-item').filter({
    has: page.locator('.ant-conversations-label').getByText(label, { exact: true }),
  });
  await candidates.first().waitFor({ timeout: 60000 });
  const duplicates = rows.filter(d => (d.title || d.session_id.slice(0, 12) + '…') === label);
  const index = duplicates.findIndex(d => d.session_id === id);
  const loaded = page.waitForResponse(r => new URL(r.url()).pathname === `/api/sessions/${id}` && r.request().method() === 'GET');
  await candidates.nth(index).click();
  assert((await loaded).ok(), 'opening the selected conversation failed');
}

async function send(page, prompt, id, onReceipt = () => {}) {
  const route = id ? `/api/sessions/${id}/prompt` : '/api/sessions';
  await page.getByPlaceholder('输入提示词，Enter 发送，Shift+Enter 换行').fill(prompt);
  const started = Date.now();
  const responsePromise = page.waitForResponse(r => new URL(r.url()).pathname === route && r.request().method() === 'POST');
  await page.locator('.ant-sender button:has(.anticon-arrow-up)').click();
  const response = await responsePromise;
  const receipt = { status: response.status(), request: response.request().postDataJSON(),
    body: await response.json(), milliseconds: Date.now() - started };
  onReceipt(receipt);
  assert(response.ok(), `submission rejected: ${JSON.stringify(receipt)}`);
  assert(receipt.milliseconds <= 30000, `admission exceeds 30 seconds: ${receipt.milliseconds}`);
  return receipt;
}

async function answer(page, id, marker, update = () => {}) {
  const deadline = Date.now() + 900000;
  while (Date.now() < deadline) {
    const response = await api(page, `/api/executions/${id}`);
    assert.equal(response.status, 200);
    const status = response.body.execution?.status;
    const text = await page.locator('.ant-bubble-start').allTextContents();
    update({ status, visible_characters: text.join('').length, checked_at: new Date().toISOString() });
    assert(!['failed', 'error', 'cancelled', 'interrupted'].includes(status), `execution ended: ${status}`);
    if (['idle', 'done'].includes(status) && text.some(t => t.includes(marker))) {
      await page.locator('.ant-sender-actions-btn-loading-button').waitFor({ state: 'hidden' });
      return { passed: true, status, visible_reply: text.findLast(t => t.includes(marker)) };
    }
    await new Promise(resolve => setTimeout(resolve, 2000));
  }
  throw new Error(`no complete browser reply within 15 minutes: ${id}, ${marker}`);
}

module.exports = { options, api, ready, enter, chooseAgent, reopen, send, answer };
