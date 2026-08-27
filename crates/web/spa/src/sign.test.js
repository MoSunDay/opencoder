// vitest smoke tests for the signing primitives (sign.js). Vectors mirror the
// Rust tests in crates/core/src/auth_sig.rs plus RFC 4231 case 2 for HMAC.
import { describe, expect, it } from 'vitest';
import { canonicalString, EMPTY_BODY_SHA, hmacSha256Hex, sha256Hex } from './sign.js';

describe('canonicalString', () => {
  it('is METHOD \\n path \\n ts \\n body-hash', () => {
    expect(canonicalString('GET', '/api/nodes', 42, 'ab cd')).toBe('GET\n/api/nodes\n42\nab cd');
  });

  it('uppercases the method', () => {
    expect(canonicalString('post', '/p', 1, EMPTY_BODY_SHA)).toBe('POST\n/p\n1\n' + EMPTY_BODY_SHA);
  });

  it('falls back to the empty-body hash', () => {
    expect(canonicalString('GET', '/p', 1)).toBe('GET\n/p\n1\n' + EMPTY_BODY_SHA);
  });

  it('the query string is part of the signed material', () => {
    const a = canonicalString('GET', '/api/nodes/tasks/claim?node_id=n1', 1, EMPTY_BODY_SHA);
    const b = canonicalString('GET', '/api/nodes/tasks/claim?node_id=n2', 1, EMPTY_BODY_SHA);
    expect(a).not.toBe(b);
  });
});

describe('sha256Hex', () => {
  it('matches the empty-input vector (same constant as auth_sig.rs)', async () => {
    expect(await sha256Hex(new Uint8Array(0))).toBe(
      'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
    );
  });

  it('matches the "abc" vector used by the Rust tests', async () => {
    expect(await sha256Hex(new TextEncoder().encode('abc'))).toBe(
      'ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad',
    );
  });
});

describe('hmacSha256Hex', () => {
  // RFC 4231 test case 2: key "Jefe", data "what do ya want for nothing?".
  it('matches the RFC 4231 HMAC-SHA256 vector', async () => {
    const sig = await hmacSha256Hex('Jefe', 'what do ya want for nothing?');
    expect(sig).toBe('5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843');
  });

  it('changes when the token changes', async () => {
    const a = await hmacSha256Hex('right', canonicalString('GET', '/p', 1, ''));
    const b = await hmacSha256Hex('wrong', canonicalString('GET', '/p', 1, ''));
    expect(a).not.toBe(b);
  });
});
