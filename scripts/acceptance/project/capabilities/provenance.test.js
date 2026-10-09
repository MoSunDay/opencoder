const { test } = require('node:test');
const assert = require('node:assert/strict');
const http = require('node:http');
const { observe } = require('./provenance');

test('forwards the real response unchanged and records its reported model without credentials or bodies', async () => {
  let request;
  const wire = 'data: {"model":"actual-model","choices":[{"delta":{"content":"private answer"}}]}\n\n' +
    'data: {"usage":{"total_tokens":42,"private":"hidden"},"choices":[]}\n\ndata: [DONE]\n\n';
  const upstream = http.createServer(async (incoming, outgoing) => {
    assert.equal(incoming.headers.authorization, 'Bearer private-key');
    const chunks = []; for await (const chunk of incoming) chunks.push(chunk);
    request = Buffer.concat(chunks).toString();
    outgoing.writeHead(200, { 'content-type': 'text/event-stream' });
    outgoing.write(wire.slice(0, 3)); outgoing.end(wire.slice(3));
  });
  await new Promise((resolve) => upstream.listen(0, '127.0.0.1', resolve));
  const observer = await observe({ model: 'named/declared-model', providers: { named: {
    base_url: `http://127.0.0.1:${upstream.address().port}`, api_key: 'private-key',
  } } });
  try {
    const body = JSON.stringify({ model: 'declared-model', messages: [{ content: 'private prompt' }] });
    const response = await fetch(observer.config.providers.named.base_url + '/chat/completions', {
      method: 'POST', headers: { authorization: 'Bearer private-key' }, body,
    });
    assert.equal(await response.text(), wire); assert.equal(request, body);
    const [record] = observer.ledger.requests;
    assert.equal(record.requestModel, 'declared-model');
    assert.deepEqual(record.responseModels, ['actual-model']);
    assert.deepEqual(record.usage, { total_tokens: 42 }); assert.equal(record.complete, true);
    assert(!/private-key|private prompt|private answer|hidden/.test(JSON.stringify(observer.ledger)));
  } finally {
    await observer.close(); upstream.closeAllConnections(); await new Promise((resolve) => upstream.close(resolve));
  }
});
