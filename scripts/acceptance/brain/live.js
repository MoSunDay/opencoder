// Five real capability types, real configured model, no database cleanup.
// Usage: node live.js /etc/opencoder/server/opencoder.json /var/tmp/evidence-dir
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { prepare, collect } = require('./scenario');

async function main() {
  const config = JSON.parse(fs.readFileSync(process.argv[2], 'utf8')).deployment;
  const evidence = process.argv[3];
  assert(config && evidence, 'configuration and evidence directory are required');
  fs.mkdirSync(evidence, { recursive: true });
  const token = fs.readFileSync(config.token_file, 'utf8').trim();
  const api = async (method, route, body, base = config.public_url) => {
    const response = await fetch(base + route, { method, signal: AbortSignal.timeout(90000),
      headers: { Authorization: `Bearer ${token}`, 'Content-Type': 'application/json' },
      body: body === undefined ? undefined : JSON.stringify(body) });
    const value = await response.json();
    assert(response.ok, `${method} ${route}: ${response.status}: ${JSON.stringify(value).slice(0, 1000)}`);
    return value;
  };
  const state = JSON.parse(fs.readFileSync(path.join(config.state_dir, 'release-state.json'), 'utf8'));
  assert.equal(state.phase, 'complete');
  const runtime = state.releases[state.current];
  const inventory = await api('GET', '/inventory', undefined, `http://127.0.0.1:${runtime.runtime_port}`);
  const tag = `v3-live-${Date.now()}`;
  const request = await prepare(api, tag, inventory.registration.id);
  fs.writeFileSync(path.join(evidence, 'identity.json'), JSON.stringify({ run_id: request.id, release: state.current, node_id: inventory.registration.id }, null, 2));
  assert.equal((await api('POST', '/api/brain/runs', request)).run_id, request.id);
  const until = async (check, label, timeout) => {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) {
      const value = await check();
      if (value) return value;
      await new Promise((resolve) => setTimeout(resolve, 1000));
    }
    throw new Error(`timeout: ${label}; run ${request.id} is retained for inspection`);
  };
  const receipt = await collect(api, request, until);
  fs.writeFileSync(path.join(evidence, 'receipt.json'), JSON.stringify({ ...receipt, release: state.current }, null, 2));
  console.log(JSON.stringify({ ...receipt, evidence }));
}
main().catch((error) => { console.error(error.message); process.exitCode = 1; });
