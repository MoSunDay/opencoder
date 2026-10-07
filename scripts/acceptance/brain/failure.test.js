const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { test } = require('node:test');
const { captureFailure } = require('./failure');

test('a crashed renderer cannot hide the original acceptance failure', async (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'brain-failure-'));
  t.after(() => fs.rmSync(directory, { recursive: true }));
  const error = new Error('original execution assertion');
  const page = {
    screenshot: async () => { throw new Error('Target crashed'); },
    content: async () => { throw new Error('Page closed'); },
  };
  await captureFailure(page, directory, error, ['renderer crashed']);
  const report = JSON.parse(fs.readFileSync(path.join(directory, 'failure.json')));
  assert.equal(report.message, 'Error: original execution assertion');
  assert.equal(report.stack, error.stack);
  assert.deepEqual(report.browserErrors, ['renderer crashed']);
  assert.deepEqual(report.captureErrors.map((entry) => entry.name), ['screenshot', 'html']);
});

test('failure evidence remains available while the page is responsive', async (t) => {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), 'brain-failure-'));
  t.after(() => fs.rmSync(directory, { recursive: true }));
  const page = {
    screenshot: async ({ path: target }) => fs.writeFileSync(target, 'image evidence'),
    content: async () => '<body>failed state</body>',
  };
  await captureFailure(page, directory, new Error('assertion failed'), []);
  assert.equal(fs.readFileSync(path.join(directory, 'failure.html'), 'utf8'), '<body>failed state</body>');
  assert.equal(fs.readFileSync(path.join(directory, 'failure.png'), 'utf8'), 'image evidence');
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(directory, 'failure.json'))).captureErrors, []);
});
