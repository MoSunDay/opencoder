// specValidate.test.js — client-side DagSpec draft validation (mirror of
// crates/dag/src/spec.rs rules) plus the 400 problem-list extraction.
import { describe, expect, it } from 'vitest';
import { findCycle, parseSpecDraft, problemsFromApiError, validateSpec } from './specValidate.js';

const GOOD = {
  name: 'etl',
  description: ' nightly etl ',
  steps: [
    { name: 'fetch', kind: { type: 'wasm', command: 'tool.wasm' } },
    { name: 'review', depends_on: ['fetch'], kind: { type: 'agent', prompt: 'review it', agent: 'explore', model: 'gpt' } },
    { name: 'boxed', depends_on: ['fetch'], kind: { type: 'wasm', command: 'tool.wasm --flag', sandbox: 'runc' }, timeout_secs: 120 },
  ],
};

describe('parseSpecDraft', () => {
  it('parses a JSON object draft', () => {
    const r = parseSpecDraft(JSON.stringify(GOOD));
    expect(r.spec).toEqual(GOOD);
  });

  it('rejects empty / malformed / non-object drafts with readable messages', () => {
    expect(parseSpecDraft('')).toEqual({ error: '请输入工作流 JSON' });
    expect(parseSpecDraft('   ').error).toContain('请输入');
    expect(parseSpecDraft('{oops').error).toContain('JSON 解析失败');
    expect(parseSpecDraft('[1,2]').error).toContain('对象');
  });
});

describe('validateSpec', () => {
  it('accepts a representative spec (agent + wasm + runc sandbox)', () => {
    expect(validateSpec(GOOD)).toEqual([]);
  });

  it('flags name/description/steps shape problems', () => {
    expect(validateSpec({})).toContain('spec.name 必须是非空字符串');
    expect(validateSpec({ name: 'x' })).toContain('spec.steps 必须是非空数组');
    expect(validateSpec({ name: 'x', steps: [] })).toContain('spec.steps 必须是非空数组');
    expect(validateSpec({ name: 'x', description: 3, steps: GOOD.steps })).toContain(
      'spec.description 只能是字符串',
    );
  });

  it('enforces the step slug charset and uniqueness', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 'Bad_Name', kind: { type: 'wasm', command: 'x' } },
        { name: 'dup', kind: { type: 'wasm', command: 'x' } },
        { name: 'dup', kind: { type: 'wasm', command: 'x' } },
      ],
    };
    const p = validateSpec(spec);
    expect(p.some((s) => s.includes('steps[0].name 必须匹配'))).toBe(true);
    expect(p.some((s) => s.includes('steps[2].name 重复'))).toBe(true);
  });

  it('validates step kind payloads per type', () => {
    const mk = (kind) => ({ name: 'a', kind });
    expect(validateSpec({ name: 'x', steps: [mk({ type: 'shell', cmd: 'ls' })] })[0]).toContain(
      'kind.type 必须是 agent | wasm | runner',
    );
    expect(validateSpec({ name: 'x', steps: [mk({ type: 'agent' })] })[0]).toContain('kind.prompt');
    expect(validateSpec({ name: 'x', steps: [mk({ type: 'wasm' })] })[0]).toContain('kind.command');
    expect(
      validateSpec({ name: 'x', steps: [mk({ type: 'wasm', command: 'x', sandbox: 'jail' })] })[0],
    ).toContain('sandbox');
    // the removed python kind falls into the unknown-type branch
    expect(validateSpec({ name: 'x', steps: [mk({ type: 'python', code: 'x' })] })[0]).toContain(
      'kind.type 必须是 agent | wasm | runner',
    );
    expect(validateSpec({ name: 'x', steps: [{ name: 'a', kind: null }] })[0]).toContain('kind 必须是对象');
  });

  it('validates agent how_append: optional string bounded by 8192 bytes', () => {
    const mk = (howAppend) => ({
      name: 'a',
      kind: { type: 'agent', prompt: 'p', how_append: howAppend },
    });
    // absent / short values pass
    expect(validateSpec({ name: 'x', steps: [{ name: 'a', kind: { type: 'agent', prompt: 'p' } }] })).toEqual([]);
    expect(validateSpec({ name: 'x', steps: [mk('一行经验记录')] })).toEqual([]);
    // exactly 8192 bytes (UTF-8) is the inclusive upper bound
    expect(validateSpec({ name: 'x', steps: [mk('x'.repeat(8192))] })).toEqual([]);
    expect(validateSpec({ name: 'x', steps: [mk('x'.repeat(8193))] })).toEqual([
      'steps[0] (agent) kind.how_append 超过 8192 字节上限',
    ]);
    // the bound is bytes, not chars: 3000 CJK chars ≈ 9000 UTF-8 bytes
    expect(validateSpec({ name: 'x', steps: [mk('中'.repeat(3000))] })).toEqual([
      'steps[0] (agent) kind.how_append 超过 8192 字节上限',
    ]);
    // non-string payloads are rejected
    expect(validateSpec({ name: 'x', steps: [mk(123)] })).toEqual([
      'steps[0].kind.how_append 只能是字符串',
    ]);
  });

  it('flags depends_on problems: unknown refs, self-deps, duplicates', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 'a', depends_on: ['ghost', 'a'], kind: { type: 'wasm', command: 'tool.wasm' } },
        { name: 'b', depends_on: ['a', 'a'], kind: { type: 'wasm', command: 'tool.wasm' } },
      ],
    };
    const p = validateSpec(spec);
    expect(p.some((s) => s.includes('未定义步骤: ghost'))).toBe(true);
    expect(p.some((s) => s.includes('不能包含自身'))).toBe(true);
    expect(p.some((s) => s.includes('重复项'))).toBe(true);
  });

  it('rejects dependency cycles with the offending path', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 'a', depends_on: ['c'], kind: { type: 'wasm', command: 'tool.wasm' } },
        { name: 'b', depends_on: ['a'], kind: { type: 'wasm', command: 'tool.wasm' } },
        { name: 'c', depends_on: ['b'], kind: { type: 'wasm', command: 'tool.wasm' } },
      ],
    };
    const p = validateSpec(spec);
    expect(p.some((s) => s.startsWith('依赖存在环:'))).toBe(true);
  });

  it('flags a non-positive timeout_secs', () => {
    const spec = { name: 'x', steps: [{ name: 'a', timeout_secs: 0, kind: { type: 'wasm', command: 'tool.wasm' } }] };
    expect(validateSpec(spec)[0]).toContain('timeout_secs');
  });
});

describe('findCycle', () => {
  it('returns null for a DAG and the cycle path for a cyclic one', () => {
    expect(findCycle(GOOD)).toBeNull();
    const cyc = {
      steps: [
        { name: 'a', depends_on: ['b'] },
        { name: 'b', depends_on: ['c'] },
        { name: 'c', depends_on: ['a'] },
      ],
    };
    const path = findCycle(cyc);
    expect(path[0]).toBe(path[path.length - 1]);
    expect(new Set(path).size).toBe(3);
  });
});

describe('problemsFromApiError', () => {
  it('prefers the server 400 problem list, degrades to error/message', () => {
    expect(problemsFromApiError({ status: 400, body: { problems: ['bad name', 'dup step'] } })).toEqual([
      'bad name',
      'dup step',
    ]);
    expect(problemsFromApiError({ status: 400, body: { error: 'invalid spec' } })).toEqual(['invalid spec']);
    expect(problemsFromApiError(new Error('网络错误: x'))).toEqual(['网络错误: x']);
    expect(problemsFromApiError(null)).toEqual(['提交失败']);
  });
});

it('validates registered runner bindings', () => {
  const spec = { name: 'business', steps: [{ name: 'workflow', kind: { type: 'runner', runner: 'eval-diagnose', agent: 'eval-diagnose' } }] };
  expect(validateSpec(spec)).toEqual([]);
  spec.steps[0].kind.runner = '../bin';
  expect(validateSpec(spec).join(' ')).toContain('kind.runner');
  spec.steps[0].kind.agent = '';
  expect(validateSpec(spec).join(' ')).toContain('kind.agent');
});
