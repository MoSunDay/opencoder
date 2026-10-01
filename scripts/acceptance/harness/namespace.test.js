const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const test = require('node:test');

test('native fixtures run in a private mount namespace without changing their caller', () => {
  const before = fs.readlinkSync('/proc/self/ns/mnt');
  const marker = '--native-fixture-namespace';
  const script = `require(${JSON.stringify(require.resolve('./namespace'))}).isolateFixture();
    const fs = require('node:fs');
    console.log(JSON.stringify({namespace: fs.readlinkSync('/proc/self/ns/mnt'),
      mounts: fs.readFileSync('/proc/self/mountinfo', 'utf8'), args: process.argv.slice(1)}));`;
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'opencoder-namespace-test-'));
  const probe = path.join(directory, 'probe.js');
  fs.writeFileSync(probe, script);
  let result;
  try {
    result = spawnSync(process.execPath, [probe, '/fixture/rootfs'], { encoding: 'utf8' });
  } finally {
    fs.rmSync(directory, { recursive: true });
  }
  assert.equal(result.status, 0, result.stderr);
  const receipt = JSON.parse(result.stdout);
  assert.notEqual(receipt.namespace, before);
  assert.equal(fs.readlinkSync('/proc/self/ns/mnt'), before);
  assert(!receipt.mounts.split('\n').some((line) => line.split(' - ')[0].includes(' shared:')));
  assert.deepEqual(receipt.args, [probe, '/fixture/rootfs', marker]);
});
