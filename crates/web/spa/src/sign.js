// sign.js — request signing primitives, pure and dependency-free so vitest can
// exercise them without DOM/network. JS mirror of crates/core/src/auth_sig.rs:
//   canon = "{METHOD}\n{path_and_query}\n{ts_ms}\n{sha256_hex(body)}"
//   X-Sig-Timestamp: ts_ms ; X-Sig: hex(HMAC-SHA256(token, canon))
// Empty bodies (GET/DELETE) hash the empty byte string.

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

/// SHA-256 of raw bytes via WebCrypto → lowercase hex.
export async function sha256Hex(bytes) {
  const buf = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes || 0);
  const digest = await crypto.subtle.digest('SHA-256', buf);
  return bytesToHex(new Uint8Array(digest));
}

/// Lowercase-hex HMAC-SHA256 of `canonical` keyed by the shared token.
export async function hmacSha256Hex(token, canonical) {
  const key = await crypto.subtle.importKey(
    'raw',
    encoder.encode(String(token || '')),
    { name: 'HMAC', hash: 'SHA-256' },
    false,
    ['sign'],
  );
  const sig = await crypto.subtle.sign('HMAC', key, encoder.encode(canonical));
  return bytesToHex(new Uint8Array(sig));
}

/// Full signing step: bytes in, { ts, sig } headers' payload out.
export async function signRequest(token, method, pathAndQuery, bodyBytes, tsMs) {
  const bodySha = await sha256Hex(bodyBytes || new Uint8Array(0));
  const ts = typeof tsMs === 'number' ? tsMs : Date.now();
  const canonical = canonicalString(method, pathAndQuery, ts, bodySha);
  const sig = await hmacSha256Hex(token, canonical);
  return { ts, sig };
}
