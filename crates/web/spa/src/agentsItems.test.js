// Agent resource encoding, references and NFS hints.

import { describe, expect, it } from 'vitest';
import { REF_FIELDS, b64EncodeText, mountHint, resolvedNames } from './agentsItems.js';

describe('b64 codecs', () => {
  it('round-trips ASCII and CJK text (UTF-8, not Latin-1)', () => {
    const samples = ['SOUL TEXT', '你是最强的 Rust 工程师', 'a\nb\tc'];
    samples.forEach((s) => {
      expect(Buffer.from(b64EncodeText(s), 'base64').toString('utf8')).toBe(s);
    });
  });

  it('encodes missing text as empty', () => {
    expect(b64EncodeText('')).toBe('');
    expect(b64EncodeText(null)).toBe('');
    expect(b64EncodeText(undefined)).toBe('');
  });
});

describe('resolvedNames', () => {
  it('reads prompt_files for the prompt field, arrays for skills/tools', () => {
    const refs = { prompt_files: ['soul', 'how'], skills: ['ssh'], tools: ['bash'], memory: true };
    expect(resolvedNames(refs, 'prompt')).toEqual(['soul', 'how']);
    expect(resolvedNames(refs, 'skills')).toEqual(['ssh']);
    expect(resolvedNames(refs, 'tools')).toEqual(['bash']);
  });

  it('flattens the memory boolean to one line, empty when absent', () => {
    expect(resolvedNames({ memory: true }, 'memory')).toEqual(['memory 已接入']);
    expect(resolvedNames({ memory: false }, 'memory')).toEqual([]);
    expect(resolvedNames(null, 'tools')).toEqual([]);
  });
});

describe('mountHint', () => {
  it('fills host/port into the mount(8) line', () => {
    expect(mountHint({ host: '127.0.0.1', port: 2049 }))
      .toBe('mount -t nfs -o ro,vers=3,tcp,port=2049,mountport=2049,nolock 127.0.0.1:/ <dir>');
  });

  it('keeps placeholders for a missing snapshot', () => {
    expect(mountHint(null)).toContain('port=PORT');
    expect(mountHint({})).toContain('HOST:/');
  });
});

describe('REF_FIELDS', () => {
  it('keeps the reference display order', () => {
    expect(REF_FIELDS).toEqual(['prompt', 'skills', 'tools', 'memory']);
  });
});
