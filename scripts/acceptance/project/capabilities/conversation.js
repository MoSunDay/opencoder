// The project drawer and CLI must continue the same native conversation.
const assert = require('node:assert/strict');
const path = require('node:path');
const { execFile } = require('node:child_process');
const { promisify } = require('node:util');
const { openTodo, closeTodo } = require('./browser');
const execute = promisify(execFile);

async function verify(h, todo, id) {
  const links = '/api/project/todos/' + todo.id + '/executions';
  const before = await h.api('GET', links);
  const owner = await h.api('GET', '/api/executions/' + id + '/index');
  const base = new URL(h.page.url()).origin;
  const token = await h.page.evaluate(() => localStorage.getItem('oc_token'));
  const cli = async (...args) => {
    const { stdout } = await execute(path.join(process.env.PLATFORM_BIN_DIR, 'opencoder-cli'),
      ['--server', base, '--token', token, 'project', 'todos', ...args],
      { timeout: 120000, maxBuffer: 4 * 1024 * 1024 });
    return JSON.parse(stdout);
  };
  const completed = async (prompt) => h.until(async () => {
    const session = await h.api('GET', '/api/sessions/' + id);
    const messages = session.messages || [];
    const index = await h.api('GET', '/api/executions/' + id + '/index');
    return index.status === 'idle' && messages.at(-1)?.role === 'assistant'
      && JSON.stringify(messages).includes(prompt);
  }, 'continued conversation: ' + id, 180000);

  const fromPage = 'Continue the same task. Keep marker ' + todo.marker + '. Page follow-up.';
  const drawer = await openTodo(h.page, todo);
  const input = drawer.getByPlaceholder('继续会话', { exact: true });
  await input.fill(fromPage);
  const receipt = h.page.waitForResponse((response) =>
    new URL(response.url()).pathname === '/api/executions/' + id + '/commands'
      && response.request().method() === 'POST');
  await input.press('Enter');
  const response = await receipt;
  assert(response.ok(), await response.text());
  await completed(fromPage);
  await closeTodo(h.page, todo);

  const fromCli = 'Continue the same task. Keep marker ' + todo.marker + '. CLI follow-up.';
  await cli('prompt', todo.id, '--json', JSON.stringify({
    prompt: fromCli, input_id: todo.id + '-cli-followup', delivery: 'queue',
  }));
  await completed(fromCli);
  const page = await cli('messages', todo.id);
  const text = (page.chunks || []).map((chunk) =>
    Buffer.from(chunk.bytes_b64 || '', 'base64').toString('utf8')).join('');
  assert(text.includes(fromPage) && text.includes(fromCli), 'CLI must read both submitted inputs');
  const result = await cli('result', todo.id);
  assert(result.summary.includes(todo.marker));
  assert.deepEqual(await h.api('GET', links), before, 'continuation created or changed execution links');
  const after = await h.api('GET', '/api/executions/' + id + '/index');
  assert.equal(after.node_id, owner.node_id);
  return { execution_id: id, node_id: after.node_id, page_input: true, cli_input: true, cli_read: true };
}
module.exports = { verify };
