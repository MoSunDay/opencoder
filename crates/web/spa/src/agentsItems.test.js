// agentsItems.test.js — pure-node rules for the Agent 配置 tab (no DOM, no
// JSX). Guards the b64 text codec (UTF-8 roundtrip + dirty input), the
// ref-card → cell mapping, pool/version option shaping and the NFS mount
// hint line that agentsConfig / agentDetail / agentNfsCard render.

import { describe, expect, it } from 'vitest';
import {
  REF_FIELDS, b64DecodeText, b64EncodeText, mountHint, refCells,
  resolvedNames, resourceOptions, versionOptions,
} from './agentsItems.js';

describe('b64 codecs', () => {
  it('round-trips ASCII and CJK text (UTF-8, not Latin-1)', () => {
    const samples = ['SOUL TEXT', '你是最强的 Rust 工程师', 'a\nb\tc'];
    samples.forEach((s) => {
      expect(b64DecodeText(b64EncodeText(s))).toBe(s);
    });
  });

  it('encodes empty like the server expects and decodes empty to empty', () => {
    expect(b64EncodeText('')).toBe('');
    expect(b64DecodeText('')).toBe('');
    expect(b64DecodeText(null)).toBe('');
    expect(b64DecodeText(undefined)).toBe('');
  });

  it('degrades dirty b64 to empty instead of throwing', () => {
    expect(b64DecodeText('!!!not-base64!!!')).toBe('');
    expect(b64DecodeText('AAAAA')).toBe(''); // 长度非法（%4==1）
  });
});

describe('refCells', () => {
  it('maps a card current into four cells, unset ⇒ empty value', () => {
    const cells = refCells({ prompt: 'base', tools: 'std' });
    expect(cells.map((c) => c.field)).toEqual(['prompt', 'skills', 'tools', 'memory']);
    expect(cells[0]).toEqual({ field: 'prompt', label: 'Prompt', value: 'base' });
    expect(cells[1].value).toBe('');
    expect(cells[2].value).toBe('std');
    expect(cells[3].value).toBe('');
  });

  it('tolerates garbage current cards', () => {
    expect(refCells(null).every((c) => c.value === '')).toBe(true);
    expect(refCells('nope').every((c) => c.value === '')).toBe(true);
  });
});

describe('resourceOptions / versionOptions', () => {
  it('labels pool entries with their current version', () => {
    expect(resourceOptions([{ name: 'base', current: 2, versions: [1, 2] }, { name: 'x', current: 0 }]))
      .toEqual([
        { value: 'base', label: 'base · v2' },
        { value: 'x', label: 'x' },
      ]);
    expect(resourceOptions(null)).toEqual([]);
  });

  it('sorts versions descending for the rollback target', () => {
    expect(versionOptions([1, 3, 2])).toEqual([
      { value: 3, label: 'v3' },
      { value: 2, label: 'v2' },
      { value: 1, label: 'v1' },
    ]);
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
      .toBe('mount -t nfs -o vers=3,tcp,port=2049,mountport=2049,nolock 127.0.0.1:/ <dir>');
  });

  it('keeps placeholders for a missing snapshot', () => {
    expect(mountHint(null)).toContain('port=PORT');
    expect(mountHint({})).toContain('HOST:/');
  });
});

describe('REF_FIELDS', () => {
  it('keeps field ↔ pool category aligned (api path segments)', () => {
    expect(REF_FIELDS.map((f) => f.cat)).toEqual(['prompts', 'skills', 'tools', 'memory']);
    expect(REF_FIELDS.map((f) => f.field)).toEqual(['prompt', 'skills', 'tools', 'memory']);
  });
});
