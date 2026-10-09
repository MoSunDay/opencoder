import { describe, expect, it } from 'vitest';
import { clarificationMembers, decodeRecord, topicNotice, turnView } from './model.js';

describe('Team conversation boundaries', () => {
  it('preserves a missing large turn without inventing alignment or participants', () => {
    const meta = { omitted: true, field: 'team.topic', read_via: 'detail_field' };
    expect(turnView({ turn: 51, meta })).toEqual({ number: 51, question: '本轮讨论', participants: [], aligned: undefined, steps: 0, plan: 'team.turn.51.plan', omitted: meta });
  });
  it('targets only unique members identified by the previous summary', () => {
    expect(clarificationMembers({ ambiguities: [{ node_id: 'review' }, { node_id: 'review' }, { node_id: 'act' }] })).toEqual(['review', 'act']);
  });
  it('does not hide an execution error behind a completed topic', () => {
    expect(topicNotice({ finish_reason: 'complete' }, 'error')[0]).toBe('error');
  });
  it('rejects malformed, oversized and inconsistent record windows', () => {
    const valid = { encoding: 'json-base64', offset: 0, next_offset: 2, total_bytes: 2, eof: true, bytes_b64: btoa('{}') };
    expect(decodeRecord(valid)).toEqual({ value: {} });
    for (const patch of [{ encoding: 'utf8-base64' }, { offset: 2 }, { next_offset: 3 }, { total_bytes: -1 }, { eof: false }, { bytes_b64: btoa('x'.repeat(65537)), next_offset: 65537, total_bytes: 65537 }]) {
      expect(() => decodeRecord({ ...valid, ...patch })).toThrow();
    }
  });
});
