import { describe, expect, it } from 'vitest';
import { encodeUpload, MAX_BINARY_BYTES, parseResource, resourceToken, validateUpload, versionOptions } from './model.js';

function executable(machine = 62) {
  const bytes = new Uint8Array(120);
  bytes.set([127, 69, 76, 70, 2, 1, 1]);
  const view = new DataView(bytes.buffer);
  view.setUint16(16, 2, true); view.setUint16(18, machine, true);
  return bytes;
}

describe('native resource inputs', () => {
  it('uses canonical resource names and immutable u32 version tokens', () => {
    expect(parseResource('_tool@v4294967295')).toEqual({ name: '_tool', version: 4294967295 });
    expect(parseResource('tool')).toEqual({ name: 'tool', version: null });
    for (const value of ['../tool', '.tool', 'tool@3', 'tool@v0', 'tool@v01', 'tool@v4294967296', 'a'.repeat(49)]) expect(parseResource(value)).toBeNull();
    expect(resourceToken('tool', 3)).toBe('tool@v3');
    expect(resourceToken('tool', 0)).toBe('tool');
  });
  it('keeps current-at-admission distinct from an explicit version', () => {
    expect(versionOptions({ current: 3, history: [{ version: 1 }, null, { version: 3 }] })).toEqual([
      { value: 0, label: '当前版本 v3（受理时固定）' }, { value: 1, label: '固定 v1' }, { value: 3, label: '固定 v3' },
    ]);
  });
  it('accepts Linux ELF64 architectures and preserves uploaded bytes', () => {
    for (const machine of [62, 183]) {
      const bytes = executable(machine);
      expect(validateUpload(bytes)).toBe(bytes);
      expect(Uint8Array.from(atob(encodeUpload(bytes.buffer)), (value) => value.charCodeAt(0))).toEqual(bytes);
    }
  });
  it('rejects empty, oversized, non-ELF and incompatible executables', () => {
    for (const bytes of [new Uint8Array(), new Uint8Array(MAX_BINARY_BYTES + 1), new Uint8Array(80), executable(3)]) expect(() => validateUpload(bytes)).toThrow();
    const bytes = executable(); bytes[4] = 1;
    expect(() => encodeUpload(bytes.buffer)).toThrow('ELF64');
  });
});
