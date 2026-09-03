// mock-llm-saypairs.js -- deterministic OpenAI-compatible SSE mock for the
// say-pairs browser acceptance (scripts/browser-acceptance-saypairs.js).
//
// Plain node + stdlib `http` only (no deps, no classes). Two modes:
//   * standalone: `node scripts/mock-llm-saypairs.js [port]` -> binds (0 =
//     OS pick), prints the port on stdout, serves forever.
//   * requirable: `const { startMockLlm } = require('./mock-llm-saypairs.js')`
//     -> startMockLlm({ port, logFile }) resolves { port, close }.
//
// Script rules (keyed off the LAST message role + markers found anywhere in
// the USER texts; after a steer is consumed the request carries BOTH the
// original and the steer user messages, so "any user contains X" is the
// stable signal). Every response ends with a finish_reason frame (a missing
// one makes the client treat the stream as Truncated and retry) and the
// final text round carries usage {10,5,15}.
// A client that dies mid-stream is LOGGED (client_gone_before_finish /
// stream_write_error / res_error): Node silently discards writes to an
// aborted response, so the observable mock-side signal is res 'close' firing
// before the plan's frames completed. The stream then ends without a
// finish_reason frame -> the client reports Truncated; both sides stay
// diagnosable instead of the failure being masked as a clean stop.
//   no tools                      -> short text "mock title" (title gen etc.)
//   last=tool   + STEER-B         -> Say "Steer-B handled."      (4x300ms)
//   last=tool   + STEER-A only    -> Say "Initial findings are in." (8x700ms)
//   last=tool   + SLOW            -> Say "Slowly done. 慢速完成"  (10x600ms)
//   last=tool   + 第二回合/第一回合  -> Say "Say-<marker>-done"     (6x400ms)
//   last=user   + STEER-B         -> bash tool_call "sleep 4 && echo steer-b <n>"
//   last=user   + SLOW            -> slow Say 10x600ms (busy window for steer)
//   last=user   + 收尾总结          -> empty Say + stop (renders no bubble)
//   last=user   (default)         -> bash tool_call "sleep 1 && echo hi <n>"
// <n> = messages.length: the doom-loop guard (20 identical name:input pairs)
// needs distinct tool inputs across rounds.
'use strict';

const http = require('http');
const fs = require('fs');

const CREATED = 1735689600; // fixed "created" so frames diff cleanly in logs
const USAGE = { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 };

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/// One SSE chunk frame in the exact wire shape the Rust client parses.
function chunkFrame(delta, finish, usage) {
  const choice = { index: 0, delta: delta || {} };
  if (finish) {
    choice.finish_reason = finish;
  }
  const frame = {
    id: 'chatcmpl-m', object: 'chat.completion.chunk', created: CREATED,
    model: 'm-1', choices: [choice],
  };
  if (usage) {
    frame.usage = usage;
  }
  return 'data: ' + JSON.stringify(frame) + '\n\n';
}

/// Text of one message: string content, or content arrays ({type:'text',
/// text}|{type:'input_text', text}) the way OpenAI-compatible bodies carry it.
function textOfMessage(m) {
  const c = m && m.content;
  if (typeof c === 'string') {
    return c;
  }
  if (Array.isArray(c)) {
    return c.map((p) => (p && typeof p.text === 'string' ? p.text : '')).join('');
  }
  return '';
}

function userTexts(messages) {
  return (messages || []).filter((m) => m && m.role === 'user').map(textOfMessage);
}

function lastMessage(messages) {
  const list = messages || [];
  return list.length ? list[list.length - 1] : null;
}

/// Pure decision: request -> {kind, ...} plan. Kept side-effect free so the
/// plan can be logged before any byte is written.
function decide(messages, hasTools) {
  const users = userTexts(messages);
  const any = users.join('\n');
  const last = lastMessage(messages) || {};
  const nonce = String(messages.length);
  if (!hasTools) {
    return { kind: 'say', text: 'mock title', chunks: 1, delay: 0, tag: 'title' };
  }
  if (last.role === 'tool') {
    if (any.includes('STEER-B')) {
      return { kind: 'say', text: 'Steer-B handled.', chunks: 4, delay: 300, tag: 'say-steer-b' };
    }
    if (any.includes('STEER-A')) {
      return { kind: 'say', text: 'Initial findings are in.', chunks: 8, delay: 700, tag: 'say-steer-a' };
    }
    if (any.includes('SLOW')) {
      return { kind: 'say', text: 'Slowly done. 慢速完成', chunks: 10, delay: 600, tag: 'say-slow' };
    }
    if (any.includes('第二回合')) {
      return { kind: 'say', text: 'Say-第二回合-done', chunks: 6, delay: 400, tag: 'say-r2' };
    }
    if (any.includes('第一回合')) {
      return { kind: 'say', text: 'Say-第一回合-done', chunks: 6, delay: 400, tag: 'say-r1' };
    }
    return { kind: 'say', text: 'Say-done', chunks: 3, delay: 200, tag: 'say-generic' };
  }
  // last.role === 'user' (a fresh prompt or a consumed steer boundary).
  if (any.includes('STEER-B')) {
    // sleep widens B's running-tag window so the browser can catch the split.
    return { kind: 'tool', command: 'sleep 4 && echo steer-b ' + nonce, gap: 300, tag: 'tool-steer-b' };
  }
  if (any.includes('SLOW')) {
    return { kind: 'say', text: 'Slowly done. 慢速完成', chunks: 10, delay: 600, tag: 'slow-final' };
  }
  if (textOfMessage(last).includes('收尾总结')) {
    // Post-/act_clear_context continuation: empty Say renders NO bubble, so
    // the surviving user echo stays the transcript's last element.
    return { kind: 'empty', tag: 'empty-tail' };
  }
  return { kind: 'tool', command: 'sleep 1 && echo hi ' + nonce, gap: 150, tag: 'tool-hi' };
}

/// Split text into `chunks` SSE-sized pieces (>=2 deltas for real Says).
function splitText(text, chunks) {
  if (chunks <= 1) {
    return [text];
  }
  const per = Math.max(1, Math.ceil(text.length / chunks));
  const out = [];
  for (let i = 0; i < text.length; i += per) {
    out.push(text.slice(i, i + per));
  }
  return out;
}

async function writeSay(res, text, chunks, delay) {
  for (const piece of splitText(text, chunks)) {
    res.write(chunkFrame({ content: piece }));
    if (delay) {
      await sleep(delay);
    }
  }
  res.write(chunkFrame({}, 'stop', USAGE)); // empty delta + finish + usage
}

async function writeToolCall(res, command, callId, gap) {
  res.write(chunkFrame({
    role: 'assistant',
    tool_calls: [{ index: 0, id: callId, type: 'function', function: { name: 'bash', arguments: '' } }],
  }));
  if (gap) {
    await sleep(gap);
  }
  res.write(chunkFrame({
    tool_calls: [{ index: 0, function: { arguments: JSON.stringify({ command }) } }],
  }));
  if (gap) {
    await sleep(gap);
  }
  res.write(chunkFrame({}, 'tool_calls'));
}

async function writeEmpty(res) {
  res.write(chunkFrame({ content: '' }));
  res.write(chunkFrame({}, 'stop', USAGE));
}

let callSeq = 0;

function handle(req, res) {
  const logLine = (obj) => {
    const line = JSON.stringify({ t: new Date().toISOString(), ...obj });
    console.log('[mock-llm] ' + line);
    if (handle.logFile) {
      try { fs.appendFileSync(handle.logFile, line + '\n'); } catch {}
    }
  };
  if (req.method === 'GET' && req.url.split('?')[0] === '/health') {
    res.writeHead(200, { 'content-type': 'text/plain' });
    res.end('ok');
    return;
  }
  let body = '';
  req.on('data', (d) => { body += d; });
  req.on('end', async () => {
    let parsed = {};
    try { parsed = JSON.parse(body || '{}'); } catch {}
    const messages = Array.isArray(parsed.messages) ? parsed.messages : [];
    const plan = decide(messages, Array.isArray(parsed.tools) && parsed.tools.length > 0);
    const last = lastMessage(messages) || {};
    logLine({
      lastRole: last.role || 'none',
      lastText: textOfMessage(last).slice(0, 60),
      users: userTexts(messages).length,
      plan: plan.tag,
      command: plan.command || null,
    });
    res.writeHead(200, {
      'content-type': 'text/event-stream; charset=utf-8',
      'cache-control': 'no-cache',
      connection: 'keep-alive',
    });
    res.on('error', (e) => logLine({ event: 'res_error', error: (e && e.message) || String(e) }));
    let finished = false;
    res.on('close', () => { // also fires after a normal end: only the cut-short case logs
      if (!finished) logLine({ event: 'client_gone_before_finish', plan: plan.tag });
    });
    try {
      if (plan.kind === 'tool') {
        callSeq += 1;
        await writeToolCall(res, plan.command, 'call_' + callSeq, plan.gap || 0);
      } else if (plan.kind === 'empty') {
        await writeEmpty(res);
      } else {
        await writeSay(res, plan.text, plan.chunks, plan.delay || 0);
      }
    } catch (e) {
      // Recorded, not swallowed: the stream below still ends with [DONE] but
      // NO finish_reason frame, so the client sees Truncated — both sides stay
      // observable instead of the failure being masked as a clean stop.
      logLine({ event: 'stream_write_error', error: (e && e.message) || String(e) });
    }
    finished = true;
    res.end('data: [DONE]\n\n');
  });
}

/// Start the mock. Resolves once listening; {port, close} returned.
function startMockLlm({ port = 0, logFile } = {}) {
  return new Promise((resolve, reject) => {
    const server = http.createServer(handle);
    handle.logFile = logFile || handle.logFile || null;
    server.on('error', reject);
    server.listen(port, '127.0.0.1', () => {
      const addr = server.address();
      resolve({
        port: addr.port,
        close: () => new Promise((r) => server.close(() => r())),
      });
    });
  });
}

module.exports = { startMockLlm, decide };

if (require.main === module) {
  const portArg = Number.parseInt(process.argv[2] || '0', 10) || 0;
  startMockLlm({ port: portArg }).then(({ port }) => {
    console.log(String(port)); // standalone contract: the port, one line
    process.stderr.write('mock-llm-saypairs listening on 127.0.0.1:' + port + '\n');
  }).catch((e) => {
    process.stderr.write('mock failed: ' + (e && e.message) + '\n');
    process.exit(1);
  });
}
