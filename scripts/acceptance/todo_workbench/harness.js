// Independent UI acceptance for the durable TODO initialization window.
const { spawn, spawnSync } = require('child_process');
const { chromium } = require('../../../crates/web/spa/node_modules/playwright-core');
const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const http = require('http');
const os = require('os');
const path = require('path');

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
  await until(() => child.exitCode !== null || child.signalCode, `stop ${child.pid}`, 40_000);
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
    const chunks = [];
    for await (const chunk of incoming) chunks.push(chunk);
    const text = JSON.stringify(await responseFor(latestPrompt(Buffer.concat(chunks).toString())));
    outgoing.writeHead(200, { 'content-type': 'text/event-stream' });
    outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: { role: 'assistant', content: text }, finish_reason: null }] })}\n\n`);
    outgoing.write(`data: ${JSON.stringify({ choices: [{ index: 0, delta: {}, finish_reason: 'stop' }], usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 } })}\n\n`);
    outgoing.end('data: [DONE]\n\n');
  });
  await new Promise((resolve) => mock.listen(0, '127.0.0.1', resolve));
}

function writeConfig(directory) {
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(path.join(directory, 'opencoder.json'), JSON.stringify({
    providers: { fixture: { base_url: `http://127.0.0.1:${mock.address().port}/v1`, api_key: 'fixture' } },
    model: 'fixture/model', cache_salt: false,
  }));
}


async function open(answer) {
  responseFor=answer;await startMock();
  const serverWork=path.join(root,'server-work'),nodeWork=path.join(root,'node-work');
  writeConfig(serverWork);writeConfig(nodeWork);
  const server=start('opencoder-server',['--workdir',serverWork,'--data-dir',path.join(root,'server-data'),'--port','0','--token',token],serverWork,'server');
  await until(()=>{const m=fs.readFileSync(server.logPath,'utf8').match(/listening on (http:\/\/127\.0\.0\.1:\d+)/);if(m)base=m[1];return base;},'server ready');
  const args=['--remote',base,'--token',token,'--name','todo-review-node','--workdir',nodeWork,'--data-dir',path.join(root,'node-data'),'--no-dag'];
  let agent=start('opencoder-agent',args,nodeWork,'agent');
  const nodeId=await until(async()=>(await api('GET','/api/nodes')).nodes.find(n=>n.online&&n.snapshot?.ready)?.id,'node ready');
  browser=await chromium.launch({executablePath:process.env.CHROME_PATH||chromium.executablePath(),args:['--no-sandbox','--disable-dev-shm-usage']});
  page=await browser.newPage({viewport:{width:1600,height:1000}});page.setDefaultTimeout(15000);
  page.on('pageerror',error=>browserErrors.push(error.message));
  await page.addInitScript(value=>localStorage.setItem('oc_token',value),token);
  await page.goto(base,{waitUntil:'networkidle'});
  return {page,root,nodeId,api,request,until,pause,errors:browserErrors,
    restart:async()=>{await stop(agent,'SIGKILL');agent=start('opencoder-agent',args,nodeWork,'agent-restarted');await until(async()=>(await api('GET','/api/nodes')).nodes.find(n=>n.id===nodeId)?.online,'node reconnected');}};
}
async function close(){if(browser)await browser.close();for(const child of children.reverse())await stop(child,'SIGKILL');if(mock?.listening)await new Promise(resolve=>mock.close(resolve));}
module.exports={open,close};
