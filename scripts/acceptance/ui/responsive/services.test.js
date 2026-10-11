const assert = require('node:assert/strict');
const path = require('node:path');
const { test } = require('node:test');
const services = require('./services');
const repo = path.resolve(__dirname, '../../../..');

test('concurrent responsive fixtures bind independent ports and serve requests', async (t) => {
  const instances = await Promise.all([services(repo, 'fixture-a').serve(0), services(repo, 'fixture-b').serve(0)]);
  for (const instance of instances) t.after(() => instance.close());
  assert(instances.every((instance) => instance.port > 0));
  assert.notEqual(instances[0].port, instances[1].port);
  for (const [index, instance] of instances.entries()) {
    const response = await fetch(`http://127.0.0.1:${instance.port}/api/project/people`, {
      headers: { Authorization: `Bearer fixture-${index ? 'b' : 'a'}` },
    });
    assert.equal(response.status, 200);
    assert(Array.isArray((await response.json()).people));
    assert.deepEqual(instance.fixtureMisses, []);
  }
});

test('an occupied explicit port rejects startup without crashing the process', async (t) => {
  const first = await services(repo, 'fixture').serve(0);
  t.after(() => first.close());
  await assert.rejects(services(repo, 'fixture').serve(first.port), { code: 'EADDRINUSE' });
});
