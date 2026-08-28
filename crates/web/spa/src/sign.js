// sign.js — request signing primitives, pure and dependency-free so vitest can
// exercise them without DOM/network. JS mirror of crates/core/src/auth_sig.rs:
//   canon = "{METHOD}\n{path_and_query}\n{ts_ms}\n{sha256_hex(body)}"
//   X-Sig-Timestamp: ts_ms ; X-Sig: hex(HMAC-SHA256(token, canon))
// Empty bodies (GET/DELETE) hash the empty byte string.

// WebCrypto is only exposed on secure contexts (https / localhost). On a
// plain-HTTP intranet origin crypto.subtle is undefined, so hashing falls
// back to the dependency-free pure-JS mirror in sha256.js.
import { hmacSha256 as hmacSha256Fallback, sha256 as sha256Fallback } from './sha256.js';

const subtle = globalThis.crypto && globalThis.crypto.subtle ? globalThis.crypto.subtle : null;

const encoder = new TextEncoder();

/// Lowercase hex of the empty-byte SHA-256 (known test vector, mirrors the
/// Rust `sha256_hex_matches_known_vectors` EMPTY_SHA constant).
export const EMPTY_BODY_SHA = 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855';

/// Canonical signing string. Method uppercased; newline separators make the
/// concatenation unambiguous (same reasoning as the Rust doc comment).
export function canonicalString(method, pathAndQuery, tsMs, bodySha256Hex) {
  const m = String(method || '').toUpperCase();
  const ts = String(tsMs);
  const hash = bodySha256Hex || EMPTY_BODY_SHA;
  return m + '\n' + String(pathAndQuery) + '\n' + ts + '\n' + hash;
}

export function bytesToHex(u8) {
  let out = '';
  for (let i = 0; i < u8.length; i += 1) {
    out += u8[i].toString(16).padStart(2, '0');
  }
  return out;
}

/// SHA-256 of raw bytes -> lowercase hex. WebCrypto when available,
/// pure-JS fallback over plain-HTTP intranet.
export async function sha256Hex(bytes) {
  const buf = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes || 0);
  if (subtle) {
    return bytesToHex(new Uint8Array(await subtle.digest('SHA-256', buf)));
  }
  return bytesToHex(sha256Fallback(buf));
}

/// Lowercase-hex HMAC-SHA256 of `canonical` keyed by the shared token.
export async function hmacSha256Hex(token, canonical) {
  const keyBytes = encoder.encode(String(token || ''));
  const msgBytes = encoder.encode(canonical);
  if (subtle) {
    const key = await subtle.importKey(
      'raw',
      keyBytes,
      { name: 'HMAC', hash: 'SHA-256' },
      false,
      ['sign'],
    );
    return bytesToHex(new Uint8Array(await subtle.sign('HMAC', key, msgBytes)));
  }
  return bytesToHex(hmacSha256Fallback(keyBytes, msgBytes));
}

/// Full signing step: bytes in, { ts, sig } headers' payload out.
export async function signRequest(token, method, pathAndQuery, bodyBytes, tsMs) {
  const bodySha = await sha256Hex(bodyBytes || new Uint8Array(0));
  const ts = typeof tsMs === 'number' ? tsMs : Date.now();
  const canonical = canonicalString(method, pathAndQuery, ts, bodySha);
  const sig = await hmacSha256Hex(token, canonical);
  return { ts, sig };
}
