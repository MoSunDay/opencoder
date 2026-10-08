// Independent UI acceptance for the durable TODO initialization window.
const { spawn, spawnSync, execFileSync } = require('child_process');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');
const { prepareNative } = require('../harness/native');
require('../harness/namespace').isolateFixture();

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-todo-workbench-'));
const bin = process.env.PLATFORM_BIN_DIR || path.join(__dirname, '../../../target/debug');
const token = crypto.randomBytes(24).toString('hex');
const children = [];
const browserErrors = [];
const expectedOfflineErrors = [];
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
let base;
let browser;
let page;
let mock;
let responseFor;
let native;
let agentMount;

async function until(check, label, timeout = 40_000) {
  const deadline = Date.now() + timeout;
  let last;
  while (Date.now() < deadline) {
    try {
      last = await check();
      if (last) return last;
    } catch (error) { last = error.message; }
    await pause(100);
  }
  throw new Error(`timeout: ${label}; last=${last}`);
}

function start(binary, args, cwd, label) {
  const logPath = path.join(root, `${label}.log`);
  const log = fs.openSync(logPath, 'w');
  const child = spawn(path.join(bin, binary), args, {
    cwd, stdio: ['ignore', log, log],
    env: { ...process.env, HOME: root, XDG_CONFIG_HOME: path.join(root, 'config'), XDG_DATA_HOME: path.join(root, 'data') },
  });
  fs.closeSync(log);
  child.logPath = logPath;
  children.push(child);
  return child;
}

async function stop(child, signal = 'SIGTERM') {
  if (child.exitCode !== null || child.signalCode) return;
  child.kill(signal);
  try {
    await until(() => child.exitCode !== null || child.signalCode, `stop ${child.pid}`, 40_000);
  } catch (error) {
    child.kill('SIGKILL');
    await until(() => child.exitCode !== null || child.signalCode, `kill ${child.pid}`, 5_000);
    process.exitCode = 1;
    console.error(error.message);
  }
}

async function request(method, route, body) {
  const raw = body === undefined ? '' : JSON.stringify(body);
  const response = await fetch(base + route, {
    method, signal: AbortSignal.timeout(20_000),
    headers: { Authorization: `Bearer ${token}`, ...(raw ? { 'content-type': 'application/json' } : {}) },
    body: raw || undefined,
  });
  const text = await response.text();
  let value;
  try { value = text ? JSON.parse(text) : null; } catch { value = text; }
  return { response, value };
}

async function api(method, route, body) {
  const { response, value } = await request(method, route, body);
  assert(response.ok, `${method} ${route}: ${response.status} ${JSON.stringify(value).slice(0, 500)}`);
  return value;
}

function latestPrompt(raw) {
  try {
    const messages = JSON.parse(raw).messages || [];
    const message = [...messages].reverse().find(({ role }) => role === 'user');
    if (typeof message?.content === 'string') return message.content;
    if (Array.isArray(message?.content)) return message.content.map((part) => part.text || '').join('\n');
  } catch {}
  return raw;
}

async function startMock() {
  mock = http.createServer(async (incoming, outgoing) => {
    try {
      const chunks = [];
      for await (const chunk of incoming) chunks.push(chunk);
      const raw = Buffer.concat(chunks).toString();
      const response = await responseFor(latestPrompt(raw), JSON.parse(raw));
      const text = typeof response === 'string' ? response : JSON.stringify(response);
      outgoing.writeHead(200, { 'content-type': 'text/event-stream' });
      outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: { role: 'assistant', content: text }, finish_reason: null }] })}\n\n`);
      outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } })}\n\n`);
      outgoing.end('data: [DONE]\n\n');
    } catch (error) {
      // A fixture assertion must fail the request and let main() clean up its
      // real Server/Worker processes, rather than crash in an async callback.
      console.error(error);
      if (!outgoing.headersSent) outgoing.writeHead(500, { 'content-type': 'application/json' });
      outgoing.end(JSON.stringify({ error: { message: `fixture response failed: ${error.message}` } }));
    }
  });
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve));
}

function writeConfig(directory, modelConfig, contextLimit) {
  fs.mkdirSync(directory, { recursive: true });
  const config = modelConfig || {
    providers: { fixture: { base_url: `http://127.0.0.1:${mock.address().port}/v1`, api_key: 'fixture' } },
    model: 'fixture/model', cache_salt: false,
  };
  fs.writeFileSync(path.join(directory, 'opencoder.json'), JSON.stringify({
    ...config, ...(contextLimit === undefined ? {} : { context_limit: contextLimit }),
  }), { mode: 0o600 });
}


async function open(answer, { dag = false, rootfs, modelConfig, contextLimit, withBrowser = true, agents = false } = {}) {
  responseFor=answer;if (!modelConfig) await startMock();
  const serverWork=path.join(root,'server-work'),nodeWork=path.join(root,'node-work');
  writeConfig(serverWork,modelConfig,contextLimit);writeConfig(nodeWork,modelConfig,contextLimit);
  if (agents) {
    for (const [workdir, name] of [[serverWork, 'agent-pool'], [nodeWork, 'mounted-agents']]) {
      const directory = path.join(root, name); fs.mkdirSync(directory);
      const file = path.join(workdir, 'opencoder.json');
      const config = JSON.parse(fs.readFileSync(file));
      config.agent = { ...config.agent, agents_dir: directory, nfs: { host: '127.0.0.1', port: 0 } };
      fs.writeFileSync(file, JSON.stringify(config), { mode: 0o600 });
    }
  }
  if (dag) native = await prepareNative(root, serverWork, [nodeWork], rootfs, 'node artifact', { [nodeWork]: path.join(root, 'node-data') });
  const server=start('opencoder-server',['--workdir',serverWork,'--data-dir',path.join(root,'server-data'),'--port','0','--token',token],serverWork,'server');
  await until(()=>{const m=fs.readFileSync(server.logPath,'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);if(m)base=m[1];return base;},'server ready');
  if (native) native.mount();
  if (agents) {
    const { status } = await api('POST', '/api/agents/nfs', { enabled: true });
    const directory = path.join(root, 'mounted-agents');
    execFileSync('mount', ['-t', 'nfs', '-o',
      'ro,nfsvers=3,proto=tcp,nolock,port=' + status.port + ',mountport=' + status.port + ',mountproto=tcp,noac,lookupcache=none',
      '127.0.0.1:/', directory]);
    agentMount = directory;
  }
  const args=['--remote',base,'--token',token,'--name','todo-review-node','--workdir',nodeWork,'--data-dir',path.join(root,'node-data'),...(dag ? [] : ['--no-dag'])];
  let agent=start('opencoder-agent',args,nodeWork,'agent');
  const nodeId=await until(async()=>(await api('GET','/api/nodes')).nodes.find(n=>n.online&&n.snapshot?.ready)?.id,'node ready');
  if (withBrowser) {
    browser=await chromium.launch({executablePath:process.env.CHROME_PATH||chromium.executablePath(),args:['--no-sandbox','--disable-dev-shm-usage']});
    page=await browser.newPage({viewport:{width:1600,height:1000}});page.setDefaultTimeout(15000);
    page.on('pageerror',error=>browserErrors.push(error.message));
    await page.addInitScript(value=>localStorage.setItem('oc_token',value),token);
    await page.goto(base,{waitUntil:'networkidle'});
  }
  return {page,root,nodeId,api,request,until,pause,errors:browserErrors,
    stopNode: async () => stop(agent, 'SIGKILL'),
    startNode: async () => {
      agent=start('opencoder-agent',args,nodeWork,'agent-started');
      await until(async () => (await api('GET','/api/nodes')).nodes.some(
        node => node.id === nodeId && node.online && node.snapshot?.ready), 'node started');
    },
    restart:async()=>{
      const previous=(await api('GET','/api/nodes')).nodes.find(n=>n.id===nodeId)?.snapshot?.generation;
      await stop(agent,'SIGKILL');agent=start('opencoder-agent',args,nodeWork,'agent-restarted');
      await until(async()=>{
        const node=(await api('GET','/api/nodes')).nodes.find(n=>n.id===nodeId);
        return node?.online&&node.snapshot?.ready&&node.snapshot.generation!==previous;
      },'restarted node index ready');
    }};
}
async function close() {
  const errors = [];
  const attempt = async (action) => { try { await action(); } catch (error) { errors.push(error); } };
  if (browser) await attempt(() => browser.close());
  const servers = children.filter((child) => path.basename(child.spawnargs[0]) === 'opencoder-server');
  for (const child of children.filter((child) => !servers.includes(child)).reverse()) await attempt(() => stop(child));
  if (native) await attempt(() => native.close());
  if (agentMount) await attempt(() => execFileSync('umount', [agentMount]));
  for (const child of servers.reverse()) await attempt(() => stop(child));
  if (mock?.listening) await attempt(() => new Promise((resolve) => mock.close(resolve)));
  if (errors.length) throw new AggregateError(errors, 'fixture cleanup failed');
}
module.exports={open,close};
