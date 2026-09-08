const assert = require('assert/strict');
const crypto = require('crypto');
const fs = require('fs');
const path = require('path');
const { execFileSync } = require('child_process');
const digest = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');

// Every historical payload is read through its execution index, then compared
// with the immutable node archive. No database or archive writes occur here.
async function audit(h, ids) {
  const report = { attempts: 0, messages: 0, events: 0, model_files: 0, large_events: 0 };
  for (const id of ids) {
    const detail = await h.api('GET', `/api/executions/${id}`);
    const trace = detail.replay;
    const root = path.join(h.dirs[1], 'state/project-runs', id);
    const input = await h.field(id, `project.run.${id}.input_snapshot`);
    assert.deepEqual(JSON.parse(input), JSON.parse(fs.readFileSync(path.join(root, 'input.json'), 'utf8')));
    for (const file of trace.files || []) {
      const raw = await h.field(id, `archive.${file}`);
      assert.equal(digest(raw), digest(fs.readFileSync(path.join(root, file))));
      report.model_files++;
    }
    let after = 0;
    do {
      const page = await h.api('GET', `/api/executions/${id}/events-page?after=${after}`);
      for (const event of page.events) {
        assert.equal(event.seq, ++after);
        const expected = fs.readFileSync(path.join(root, `event-${event.seq}.json`));
        if (event.data?.omitted) {
          const parts = []; let offset = 0;
          while (true) {
            const chunk = await h.api('GET', `/api/executions/${id}/events/${event.seq}/payload?offset=${offset}`);
            const bytes = Buffer.from(chunk.bytes_b64, 'base64');
            assert(bytes.length <= 65536); assert.equal(chunk.next_offset, offset + bytes.length);
            parts.push(bytes); offset = chunk.next_offset;
            if (chunk.eof) break;
          }
          assert.equal(digest(Buffer.concat(parts)), digest(expected));
          report.large_events++;
        } else assert.deepEqual(event.data, JSON.parse(expected));
      }
      if (!page.more) break;
    } while (true);
    assert.equal(after, trace.event_count);
    report.events += after;
    let cursor = { seq: 0, offset: 0 };
    const messages = new Set();
    do {
      const page = await h.api('GET', `/api/executions/${id}/messages?${new URLSearchParams(cursor)}`);
      for (const message of page.chunks) {
        assert(message.seq > trace.messages_after && message.seq <= trace.messages_through);
        messages.add(message.seq);
      }
      if (!page.more) break;
      assert(page.next_cursor); assert.notDeepEqual(page.next_cursor, cursor);
      cursor = page.next_cursor;
    } while (true);
    assert(messages.size > 0, `${id} must retain its submitted message`);
    report.messages += messages.size;
    report.attempts++;
  }
  fs.writeFileSync(path.join(h.root, 'payload-audit.json'), JSON.stringify(report, null, 2));
  execFileSync('python3', [path.join(__dirname, 'storage_audit.py')], {
    input: JSON.stringify({ root: h.root, base: h.base, token: h.token, ids }),
    stdio: ['pipe', 'pipe', 'pipe'],
  });
  return report;
}
module.exports = { audit };
