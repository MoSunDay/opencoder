const fs = require('node:fs');
const path = require('node:path');

async function captureFailure(page, artifacts, error, browserErrors) {
  const failure = { message: String(error), stack: error?.stack, browserErrors, captureErrors: [] };
  // Print the original failure before asking a possibly crashed renderer for evidence.
  console.error('Brain browser failure:', error);
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
