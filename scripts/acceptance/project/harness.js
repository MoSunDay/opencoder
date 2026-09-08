const { spawn, execFileSync } = require('child_process');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');
const assert = require('assert/strict');
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(check, label, timeout = 10000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { if (await check()) return; await pause(100); }
  throw new Error(`timeout: ${label}`);
}
async function harness() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-project-acceptance-'));
  const token = crypto.randomBytes(24).toString('hex');
  const sourceBin = process.env.PLATFORM_BIN_DIR || path.resolve(__dirname, '../../../target/debug');
  const bin = path.join(root, 'bin'); fs.mkdirSync(bin);
  for (const name of ['opencoder-server', 'opencoder-agent']) fs.copyFileSync(path.join(sourceBin, name), path.join(bin, name), fs.constants.COPYFILE_FICLONE);
  const children = [];
  process.once("exit", () => { for (const child of children) if (child.exitCode === null && !child.signalCode) child.kill("SIGKILL"); });
  const mode = { kind: 'normal', text: 'fixture completed', requests: 0 };
  const mock = http.createServer(async (req, res) => {
    let raw = ''; for await (const part of req) raw += part;
    const request = JSON.parse(raw); mode.requests += 1;
    if (mode.kind === 'hang') return;
    if (mode.kind === 'failure') {
      res.writeHead(400, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ error: { message: 'injected model failure' } })); return;
    }
    const tools = mode.kind === 'artifact' && request.messages.at(-1)?.role !== 'tool'
      ? [{ index: 0, id: `artifact-${mode.requests}`, type: 'function', function: { name: 'project_artifact', arguments: JSON.stringify({ path: 'report.txt' }) } }] : null;
    res.writeHead(200, { 'content-type': 'text/event-stream' });
    const text = mode.text;
    for (const chunk of [
      { choices: [{ index: 0, delta: tools ? { role: 'assistant', tool_calls: tools } : { role: 'assistant', content: text }, finish_reason: null }] },
      { choices: [{ index: 0, delta: {}, finish_reason: tools ? 'tool_calls' : 'stop' }], usage: { prompt_tokens: 100, completion_tokens: 10, total_tokens: 110 } },
    ]) res.write(`data: ${JSON.stringify(chunk)}\n\n`);
    res.end('data: [DONE]\n\n');
  });
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve));
  const agents = path.join(root, 'agents'); fs.mkdirSync(agents);
  const mount = path.join(root, 'mounted-agents'); fs.mkdirSync(mount);
  let mounted = false;
  const dirs = ['server', 'node-a', 'node-b'].map((name) => {
    const dir = path.join(root, name); fs.mkdirSync(dir);
    fs.writeFileSync(path.join(dir, 'opencoder.json'), JSON.stringify({
      providers: { fixture: { base_url: `http://127.0.0.1:${mock.address().port}/v1`, api_key: crypto.randomBytes(12).toString('hex') } },
      model: 'fixture/model', cache_salt: false, agent: { agents_dir: name === 'server' ? agents : mount, nfs: { host: '127.0.0.1', port: 0 } },
    }));
    fs.writeFileSync(path.join(dir, 'report.txt'), 'original immutable artifact 界\n');
    return dir;
  });
  function start(name, args, workdir) {
    const logPath = path.join(root, `${name}-${children.length}.log`);
    const log = fs.openSync(logPath, 'w');
    const child = spawn(path.join(bin, name), args, { cwd: workdir, stdio: ['ignore', log, log], env: {
      ...process.env, HOME: root, XDG_CONFIG_HOME: path.join(root, 'config'), XDG_DATA_HOME: path.join(root, 'data'),
    } });
    fs.closeSync(log); child.logPath = logPath; child.workdir = workdir;
    children.push(child); return child;
  }
  let base;
  const server = start('opencoder-server', ['--workdir', dirs[0], '--port', '0', '--token', token], dirs[0]);
  await until(async () => {
    assert.equal(server.exitCode, null, fs.readFileSync(server.logPath, 'utf8'));
    const match = fs.readFileSync(server.logPath, 'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);
    if (match) base = match[1]; return !!base;
  }, 'server listening', 30000);
  async function api(method, route, body, expected) {
    const response = await fetch(base + route, { method, signal: AbortSignal.timeout(10000), headers: {
      authorization: `Bearer ${token}`, 'content-type': 'application/json',
    }, body: body === undefined ? undefined : JSON.stringify(body) });
    const raw = await response.text();
    assert(response.headers.get('content-type')?.includes('application/json'), `${route}: ${response.status} non-JSON`);
    const value = JSON.parse(raw);
    if (expected) assert.equal(response.status, expected, `${route}: ${raw}`);
    else assert(response.ok, `${route}: ${response.status}: ${raw}`);
    return value;
  }
  const nfs = await api('POST', '/api/agents/nfs', { enabled: true });
  const port = nfs.status.port;
  execFileSync('mount', ['-t', 'nfs', '-o', `ro,nfsvers=3,proto=tcp,nolock,port=${port},mountport=${port},mountproto=tcp,noac,lookupcache=none`, '127.0.0.1:/', mount]);
  mounted = true;
  const nodes = dirs.slice(1).map((dir) => start('opencoder-agent', ['--remote', base, '--token', token, '--name', path.basename(dir), '--workdir', dir, '--data-dir', path.join(dir, 'state')], dir));
  await until(async () => (await api('GET', '/api/nodes')).nodes.filter((node) => node.online && node.snapshot.ready).length === 2, 'two ready nodes', 30000);
  const field = async (id, name) => {
    const chunks = []; let offset = 0;
    while (true) {
      const page = await api('GET', `/api/executions/${id}/detail-field?field=${encodeURIComponent(name)}&offset=${offset}`);
      const bytes = Buffer.from(page.bytes_b64, 'base64'); assert(bytes.length <= 65536);
      assert.equal(page.next_offset, offset + bytes.length); chunks.push(bytes); offset = page.next_offset;
      if (page.eof) return Buffer.concat(chunks).toString();
    }
  };
  async function stop(child, signal = 'SIGTERM') {
    if (child.exitCode !== null || child.signalCode) return;
    child.kill(signal); await until(async () => child.exitCode !== null || child.signalCode, 'child exit', 30000);
  }
  return { root, base, token, dirs, nodes, mode, api, field, start, stop,
    async close() { for (const child of children.toReversed().filter((child) => child !== server)) await stop(child); if (mounted) { execFileSync('umount', [mount]); mounted = false; } await stop(server); mock.closeAllConnections(); mock.close(); },
  };
}
module.exports = { harness, until, pause };
