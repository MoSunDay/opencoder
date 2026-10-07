const fs = require('node:fs');
const path = require('node:path');

function hostResources(directory) {
  const read = (file) => { try { return fs.readFileSync(file, 'utf8'); } catch { return undefined; } };
  const group = read('/proc/self/cgroup')?.split('\n').find((line) => line.startsWith('0::'))?.slice(3);
  const memory = read('/proc/meminfo')?.split('\n').filter((line) => /^(MemTotal|MemAvailable|SwapFree):/.test(line));
  const events = group && read(path.join('/sys/fs/cgroup', group, 'memory.events'));
  let availableBytes;
  try { const stats = fs.statfsSync(directory); availableBytes = stats.bavail * stats.bsize; } catch {}
  return { memory, cgroupMemoryEvents: events, availableBytes };
}

async function captureFailure(page, artifacts, error, browserErrors) {
  const failure = { message: String(error), stack: error?.stack, browserErrors,
    resources: hostResources(artifacts), captureErrors: [] };
  // Print the original failure before asking a possibly crashed renderer for evidence.
  console.error('Brain browser failure:', error);
  console.error('Browser host resources:', JSON.stringify(failure.resources));
  for (const [name, capture] of [
    ['screenshot', () => page.screenshot({ path: path.join(artifacts, 'failure.png'), timeout: 5000 })],
    ['html', async () => fs.writeFileSync(path.join(artifacts, 'failure.html'), await page.content())],
  ]) {
    try { await capture(); }
    catch (secondary) { failure.captureErrors.push({ name, message: String(secondary) }); }
  }
  fs.writeFileSync(path.join(artifacts, 'failure.json'), JSON.stringify(failure, null, 2));
  console.error(JSON.stringify({ artifacts, ...failure }));
}

module.exports = { captureFailure };
