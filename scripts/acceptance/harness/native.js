const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');
const crypto = require('node:crypto');
const { execFileSync } = require('node:child_process');

async function port(excluded = new Set()) {
  // Linux may reuse a released ephemeral port for an outgoing fixture
  // connection before the NFS exporter binds it. Select outside that range
  // and keep the two exporter ports distinct even before either starts.
  const [first, last] = fs.readFileSync('/proc/sys/net/ipv4/ip_local_port_range', 'utf8').trim().split(/\s+/).map(Number);
  for (let attempt = 0; attempt < 100; attempt++) {
    const value = crypto.randomInt(1024, 65536);
    if ((value >= first && value <= last) || excluded.has(value)) continue;
    const listener = net.createServer();
    const available = await new Promise((resolve, reject) => {
      listener.once('error', (error) => error.code === 'EADDRINUSE' ? resolve(false) : reject(error));
      listener.listen(value, '127.0.0.1', () => resolve(true));
    });
    if (!available) continue;
    await new Promise((resolve) => listener.close(resolve));
    return value;
  }
  throw new Error('no available NFS fixture port outside the ephemeral range');
}

function compile(directory, name, source) {
  const input = path.join(directory, `${name}.c`);
  const binary = path.join(directory, name);
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(input, source);
  execFileSync('cc', ['-O2', '-static', '-s', '-Wl,--build-id=none', input, '-o', binary]);
  return fs.readFileSync(binary);
}

function stage(pool, name, bytes) {
  const directory = path.join(pool, name, 'v1');
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(path.join(directory, 'binary.bin'), bytes, { mode: 0o555 });
  const updated = new Date().toISOString();
  fs.writeFileSync(path.join(directory, 'meta.json'), JSON.stringify({ version: 1, description: 'native fixture',
    sha256: crypto.createHash('sha256').update(bytes).digest('hex'), size_bytes: bytes.length, updated_at: updated }));
  fs.writeFileSync(path.join(pool, name, 'meta.json'), JSON.stringify({ name, description: 'native fixture',
    current: 1, history: [1], created_at: updated, updated_at: updated }));
}

function updateConfig(directory, dag) {
  const file = path.join(directory, 'opencoder.json');
  const config = fs.existsSync(file) ? JSON.parse(fs.readFileSync(file)) : {};
  config.dag = { ...config.dag, ...dag };
  fs.writeFileSync(file, JSON.stringify(config), { mode: 0o600 });
}

async function prepareNative(root, server, nodes, rootfs, message = 'node artifact', dataDirs = {}) {
  if (!rootfs || !path.isAbsolute(rootfs)) throw new Error('provide an absolute native DAG rootfs path as the first argument');
  for (const name of ['dag-runner', 'agent-step-runner']) {
    if (!fs.statSync(path.join(rootfs, 'usr/bin', name)).isFile()) throw new Error(`DAG rootfs is missing ${name}`);
  }
  const source = path.join(root, 'native-source');
  const binaries = path.join(source, 'binaries');
  const workspace = path.join(source, 'workspace');
  fs.mkdirSync(binaries, { recursive: true });
  fs.mkdirSync(workspace, { recursive: true });
  stage(binaries, 'stdout', compile(path.join(root, 'native-build'), 'stdout', `#include <stdio.h>\nint main(void) { puts(${JSON.stringify(message)}); return 0; }`));
  stage(binaries, 'spin', compile(path.join(root, 'native-build'), 'spin', '#include <unistd.h>\nint main(void) { for (;;) pause(); }'));
  const binaryPort = await port();
  const workspacePort = await port(new Set([binaryPort]));
  updateConfig(server, { binary_dir: binaries, workspace_dir: workspace,
    nfs: { enabled: true, host: '127.0.0.1', port: binaryPort, read_only: true },
    workspace_nfs: { enabled: true, host: '127.0.0.1', port: workspacePort } });
  const mounted = [];
  const plans = [];
  for (const node of nodes) {
    const mounts = path.join(node, 'native-mounts');
    const binaryMount = path.join(mounts, 'binaries');
    const workspaceMount = path.join(mounts, 'workspace');
    for (const directory of [binaryMount, workspaceMount]) fs.mkdirSync(directory, { recursive: true });
    updateConfig(node, { binary_dir: binaryMount, workspace_dir: workspaceMount, rootfs_dir: rootfs,
      data_dir: path.join(dataDirs[node] || path.join(node, 'state'), 'dag/runs') });
    plans.push([binaryMount, binaryPort], [workspaceMount, workspacePort]);
  }
  return {
    mount() {
      for (const [directory, listener] of plans) {
        execFileSync('mount', ['-t', 'nfs', '-o', `ro,vers=3,tcp,port=${listener},mountport=${listener},nolock,soft,timeo=10,retrans=1,actimeo=0,lookupcache=none`, '127.0.0.1:/', directory], { timeout: 30000 });
        mounted.push(directory);
      }
    },
    close() {
      for (const node of nodes) {
        const data = dataDirs[node] || path.join(node, 'state');
        const journals = path.join(data, 'dag');
        if (!fs.existsSync(journals)) continue;
        for (const entry of fs.readdirSync(journals)) {
          const journal = path.join(journals, entry, 'execution.json');
          if (!fs.existsSync(journal)) continue;
          const accepted = JSON.parse(fs.readFileSync(journal));
          if (!accepted.annotations?.dag_parent) continue;
          const run = path.join(accepted.annotations.dag_parent, entry);
          if (!run.startsWith(data + path.sep) || !/^[a-zA-Z0-9_-]+$/.test(entry)) throw new Error('invalid native fixture cleanup ownership');
          const state = path.join(run, 'runc-state');
          if (fs.existsSync(path.join(state, `dag-run-${entry}`))) {
            execFileSync('runc', ['--root', state, 'delete', '--force', `dag-run-${entry}`], { timeout: 30000 });
          }
          const mounts = fs.readFileSync('/proc/self/mountinfo', 'utf8');
          for (const target of [path.join(run, 'bundle/rootfs'), path.join(run, 'workspace')]) {
            if (mounts.split('\n').some((line) => line.split(' ')[4] === target)) execFileSync('umount', [target], { timeout: 30000 });
          }
        }
      }
      for (const directory of mounted.reverse()) execFileSync('umount', [directory], { timeout: 30000 });
      mounted.length = 0;
    },
  };
}

function artifactPath(data, run, relative) {
  const record = JSON.parse(fs.readFileSync(path.join(data, 'dag', run, 'execution.json')));
  return path.join(record.annotations.dag_parent, run, relative);
}

module.exports = { prepareNative, artifactPath, compile, stage };
