// Optional byte-for-byte forwarding to observe the real provider response.
// Credentials and prompt/response bodies never enter the observation ledger.
const assert = require('node:assert/strict');
const crypto = require('node:crypto');
const http = require('node:http');
const https = require('node:https');
const { StringDecoder } = require('node:string_decoder');

function metadata(line) {
  if (!line.startsWith('data: ')) return {};
  try {
    const value = JSON.parse(line.slice(6));
    const usage = value.usage && Object.fromEntries(
      ['prompt_tokens', 'completion_tokens', 'total_tokens'].filter((key) =>
        Number.isSafeInteger(value.usage[key])).map((key) => [key, value.usage[key]]));
    return { ...(typeof value.model === 'string' ? { model: value.model } : {}),
      ...(usage ? { usage } : {}) };
  } catch { return {}; }
}

async function observe(config) {
  const provider = config.model.split('/')[0];
  const endpoint = config.providers?.[provider];
  assert(endpoint?.base_url, 'model observation requires a named provider');
  const backend = new URL(endpoint.base_url);
  assert(['http:', 'https:'].includes(backend.protocol));
  assert(!backend.username && !backend.password && !backend.search,
    'provider credentials must use the existing authorization headers');
  const requests = [];
  const server = http.createServer(async (incoming, outgoing) => {
    let upstream;
    try {
      assert(incoming.method === 'POST' && ['/v1/chat/completions', '/v1/responses'].includes(incoming.url));
      const chunks = []; let size = 0;
      for await (const chunk of incoming) {
        size += chunk.length; assert(size <= 16 * 1024 * 1024, 'observation request too large'); chunks.push(chunk);
      }
      const body = Buffer.concat(chunks);
      const parsed = JSON.parse(body);
      const record = { ordinal: requests.length + 1, requestModel: parsed.model,
        markers: [...new Set((body.toString().match(/pcap-[a-f0-9]{12}-(?:agent|operator|dag|brain)/g) || []))],
        requestSha256: crypto.createHash('sha256').update(body).digest('hex'), responseModels: [], complete: false };
      requests.push(record);
      const target = backend.href.replace(/\/$/, '') + incoming.url.slice(3);
      upstream = (backend.protocol === 'https:' ? https : http).request(target, {
        method: incoming.method, headers: { ...incoming.headers, host: backend.host },
      }, (response) => {
        record.httpStatus = response.statusCode;
        outgoing.writeHead(response.statusCode, response.headers);
        const digest = crypto.createHash('sha256'); const decoder = new StringDecoder('utf8'); let pending = '';
        response.on('data', (chunk) => {
          digest.update(chunk); pending += decoder.write(chunk);
          let newline;
          while ((newline = pending.indexOf('\n')) >= 0) {
            const value = metadata(pending.slice(0, newline).trimEnd()); pending = pending.slice(newline + 1);
            if (value.model && !record.responseModels.includes(value.model)) record.responseModels.push(value.model);
            if (value.usage) record.usage = value.usage;
          }
          if (pending.length > 1024 * 1024) pending = '';
          if (!outgoing.write(chunk)) response.pause();
        });
        outgoing.on('drain', () => response.resume());
        response.once('end', () => {
          record.complete = true; record.responseSha256 = digest.digest('hex'); outgoing.end();
        });
        response.once('error', () => { record.error = 'upstream response interrupted'; outgoing.destroy(); });
      });
      upstream.once('error', () => {
        record.error = 'upstream connection failed';
        if (!outgoing.headersSent) outgoing.writeHead(502);
        outgoing.end();
      });
      outgoing.once('close', () => { if (!record.complete) upstream.destroy(); });
      upstream.end(body);
    } catch {
      upstream?.destroy(); if (!outgoing.headersSent) outgoing.writeHead(400); outgoing.end();
    }
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  return {
    config: { ...config, providers: { ...config.providers,
      [provider]: { ...endpoint, base_url: `http://127.0.0.1:${server.address().port}/v1` } } },
    ledger: { gateway: backend.href, requests },
    close: async () => { server.closeAllConnections(); await new Promise((resolve) => server.close(resolve)); },
  };
}
module.exports = { observe, metadata };
