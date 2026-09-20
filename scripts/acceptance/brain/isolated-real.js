// Run the same five real executors against the configured model before release.
// Usage: PLATFORM_BIN_DIR=... node isolated-real.js <config>
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const harness = require('../todo_workbench/harness');
const { prepare, collect } = require('./scenario');

async function main() {
  const source = JSON.parse(fs.readFileSync(process.argv[2], 'utf8'));
  const keys = ['model', 'small_model', 'provider', 'providers', 'reasoning_effort', 'max_tokens'];
  const modelConfig = Object.fromEntries(keys.filter((key) => source[key] !== undefined).map((key) => [key, source[key]]));
  assert(modelConfig.model && modelConfig.providers, 'explicit model configuration required');
  modelConfig.cache_salt = false;
  const h = await harness.open(null, { dag: true, modelConfig, withBrowser: false });
  console.log(JSON.stringify({ stage: 'isolated_real_ready', evidence: h.root }));
  const request = await prepare(h.api, `real-${Date.now()}`, h.nodeId);
  const response = await h.api('POST', '/api/brain/runs', request);
  assert.equal(response.run_id, request.id);
  console.log(JSON.stringify({ stage: 'submitted', run_id: request.id }));
  const receipt = await collect(h.api, request, h.until);
  fs.writeFileSync(path.join(h.root, 'receipt.json'), JSON.stringify({ ...receipt, model_transport: 'real' }, null, 2));
  console.log(JSON.stringify({ ...receipt, evidence: h.root, model_transport: 'real' }));
}
main().catch((error) => { console.error(error.message); process.exitCode = 1; })
  .finally(() => harness.close());
