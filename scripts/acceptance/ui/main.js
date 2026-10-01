const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const fs = require('node:fs');
const net = require('node:net');
const path = require('node:path');
const { spawn, execFileSync } = require('node:child_process');
const { CASES, verifyCoverage } = require('./scope');

const repo = path.resolve(__dirname, '../../..');
function argument(name) {
  const index = process.argv.indexOf(`--${name}`);
  if (index < 0 || !process.argv[index + 1]) throw new Error(`required: --${name}`);
  return path.resolve(process.argv[index + 1]);
}
async function availablePort() {
  const listener = net.createServer();
  await new Promise((resolve) => listener.listen(0, '127.0.0.1', resolve));
  const port = listener.address().port;
  await new Promise((resolve) => listener.close(resolve));
  return port;
}
const sha256 = (file) => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');

function run(command, args, options) {
  return new Promise((resolve, reject) => {
    const descriptor = fs.openSync(options.log, 'wx');
    const started = Date.now();
    const child = spawn(command, args, { cwd: repo, env: options.env, stdio: ['ignore', descriptor, descriptor], detached: true });
    fs.closeSync(descriptor);
    let expired = false;
    const timer = setTimeout(() => {
      expired = true;
      try { process.kill(-child.pid, 'SIGKILL'); } catch {}
    }, options.timeout || 240000);
    child.once('error', (error) => { clearTimeout(timer); reject(error); });
    child.once('exit', (code, signal) => {
      clearTimeout(timer);
      const result = { passed: code === 0 && !expired, code, signal, expired, seconds: (Date.now() - started) / 1000, log: options.log };
      resolve(result);
    });
  });
}

async function main() {
  const binaries = argument('bin-dir');
  const rootfs = argument('rootfs');
  const output = argument('output');
  assert(!fs.existsSync(output), 'use a new evidence directory');
  fs.mkdirSync(output, { recursive: true });
  const { ALL_PAGES } = await import(path.join(repo, 'crates/web/spa/src/nav.js'));
  const report = { passed: false, coverage: verifyCoverage(ALL_PAGES), artifacts: {}, checks: [] };
  const receipt = path.join(output, 'receipt.json');
  const env = { ...process.env, PLATFORM_BIN_DIR: binaries, PROJECT_UI_ARTIFACTS: path.join(output, 'project-board') };
  const check = async (name, command, args, timeout) => {
    console.log(`START ${name}`);
    const result = await run(command, args, { log: path.join(output, `${name}.log`), env, timeout });
    report.checks.push({ name, ...result });
    fs.writeFileSync(receipt, JSON.stringify(report, null, 2));
    assert(result.passed, `${name} failed: ${result.log}`);
    console.log(`PASS ${name} (${result.seconds.toFixed(1)}s)`);
  };
  try {
    let build;
    for (const name of ['opencoder', 'opencoder-cli', 'opencoder-server', 'opencoder-agent', 'dag-runner', 'agent-step-runner']) {
      const binary = path.join(binaries, name);
      const info = JSON.parse(execFileSync(binary, ['--build-info'], { encoding: 'utf8' }));
      if (!build) build = info;
      else assert.deepEqual(info, build, `build metadata differs: ${name}`);
      report.artifacts[name] = sha256(binary);
    }
    report.build = build;
    for (const name of ['dag-runner', 'agent-step-runner']) assert.equal(sha256(path.join(rootfs, 'usr/bin', name)), report.artifacts[name], `rootfs runner differs: ${name}`);
    const spa = path.join(repo, 'crates/web/spa/dist');
    const digest = crypto.createHash('sha256');
    const visit = (directory) => fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => entry.isDirectory() ? visit(path.join(directory, entry.name)) : entry.isFile() ? [path.join(directory, entry.name)] : []);
    for (const file of visit(spa).sort()) digest.update(fs.readFileSync(file));
    assert.equal(build.spa_sha256, digest.digest('hex'), 'Server build does not match current SPA');
    await check('spa-drift', 'bash', ['scripts/check-spa-drift.sh']);
    for (const width of [1920, 1280, 768, 390]) {
      await check(`responsive-${width}`, process.execPath, ['scripts/acceptance/spa_responsive.js', '--width', String(width), '--port', String(await availablePort()), '--shots', path.join(output, `responsive-${width}`)]);
    }
    for (const item of CASES) {
      if (item.cargo) await check(item.name, 'cargo', ['test', '--locked', '-p', 'opencoder-worker', '--test', 'brain_browser', 'schema_seven_canvas_parallel_return_and_execution_detail', '--', '--ignored', '--nocapture'], 900000);
      else if (item.terminal) await check(item.name, 'python3', ['scripts/acceptance/tui_server.py', path.join(binaries, 'opencoder')]);
      else await check(item.name, process.execPath, [path.join('scripts/acceptance', item.script), ...(item.native ? [rootfs] : []), ...(item.args || [])], item.name === 'platform' ? 900000 : 600000);
    }
    report.passed = true;
  } catch (error) {
    report.error = error.message;
    throw error;
  } finally {
    fs.writeFileSync(receipt, JSON.stringify(report, null, 2));
  }
  console.log(`PASS global UI: ${receipt}`);
}
main().catch((error) => { console.error(error); process.exitCode = 1; });
