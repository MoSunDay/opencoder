const { spawnSync } = require('node:child_process');

function isolateFixture() {
  const marker = '--native-fixture-namespace';
  if (process.argv.includes(marker)) return;
  const result = spawnSync('unshare', [
    '--mount', '--propagation', 'private', process.execPath,
    ...process.argv.slice(1), marker,
  ], { stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.signal) process.kill(process.pid, result.signal);
  process.exit(result.status ?? 1);
}

module.exports = { isolateFixture };
