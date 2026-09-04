// specValidate.test.js — client-side DagSpec draft validation (mirror of
// crates/dag/src/spec.rs rules) plus the 400 problem-list extraction.
import { describe, expect, it } from 'vitest';
import { findCycle, parseSpecDraft, problemsFromApiError, validateSpec } from './specValidate.js';

const GOOD = {
  name: 'etl',
  description: ' nightly etl ',
  steps: [
    { name: 'fetch', kind: { type: 'python', code: 'print(1)' } },
    { name: 'review', depends_on: ['fetch'], kind: { type: 'agent', prompt: 'review it', agent: 'explore', model: 'gpt' } },
    { name: 'boxed', depends_on: ['fetch'], kind: { type: 'python', code: 'print(2)', sandbox: 'runc' }, timeout_secs: 120 },
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
  it('accepts a representative spec (agent + python + runc sandbox)', () => {
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
        { name: 'Bad_Name', kind: { type: 'python', code: 'x' } },
        { name: 'dup', kind: { type: 'python', code: 'x' } },
        { name: 'dup', kind: { type: 'python', code: 'x' } },
      ],
    };
    const p = validateSpec(spec);
    expect(p.some((s) => s.includes('steps[0].name 必须匹配'))).toBe(true);
    expect(p.some((s) => s.includes('steps[2].name 重复'))).toBe(true);
  });

  it('validates step kind payloads per type', () => {
    const mk = (kind) => ({ name: 'a', kind });
    expect(validateSpec({ name: 'x', steps: [mk({ type: 'shell', cmd: 'ls' })] })[0]).toContain(
      'kind.type 必须是 agent | python',
    );
    expect(validateSpec({ name: 'x', steps: [mk({ type: 'agent' })] })[0]).toContain('kind.prompt');
    expect(validateSpec({ name: 'x', steps: [mk({ type: 'python' })] })[0]).toContain('kind.code');
    expect(
      validateSpec({ name: 'x', steps: [mk({ type: 'python', code: 'x', sandbox: 'jail' })] })[0],
    ).toContain('sandbox');
    expect(validateSpec({ name: 'x', steps: [{ name: 'a', kind: null }] })[0]).toContain('kind 必须是对象');
  });

  it('flags depends_on problems: unknown refs, self-deps, duplicates', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 'a', depends_on: ['ghost', 'a'], kind: { type: 'python', code: 'x' } },
        { name: 'b', depends_on: ['a', 'a'], kind: { type: 'python', code: 'x' } },
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
        { name: 'a', depends_on: ['c'], kind: { type: 'python', code: 'x' } },
        { name: 'b', depends_on: ['a'], kind: { type: 'python', code: 'x' } },
        { name: 'c', depends_on: ['b'], kind: { type: 'python', code: 'x' } },
      ],
    };
    const p = validateSpec(spec);
    expect(p.some((s) => s.startsWith('依赖存在环:'))).toBe(true);
  });

  it('flags a non-positive timeout_secs', () => {
    const spec = { name: 'x', steps: [{ name: 'a', timeout_secs: 0, kind: { type: 'python', code: 'x' } }] };
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
