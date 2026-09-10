// specValidate.test.js — client-side WorkflowSpec draft validation (mirror
// of crates/todos/src/domain.rs validate_spec rules): structured
// {path, message} problems, todo pinning via `todos[id]`, and cycle
// detection with the iterative DFS (long chains must not blow the stack).
import { describe, expect, it } from 'vitest';
import { BUILTIN_AGENTS, findCycle, parseSpecDraft, validateSpec } from './specValidate.js';

const GOOD_TODO = (id, deps, agent) => ({
  id,
  title: '任务 ' + id,
  requirement_background: '需求背景',
  instructions: '执行说明',
  depends_on: deps || [],
  agent: agent === undefined ? 'act' : agent,
  max_attempts: 3,
  acceptance: { criteria: '验收标准', required_tool_calls: [] },
  metadata: {},
});

const GOOD = {
  schema_version: 1,
  id: 'wf-1',
  name: '发布会筹备',
  objective: '按时完成发布',
  constraints: ['不许延期'],
  todos: [GOOD_TODO('a'), GOOD_TODO('b', ['a']), GOOD_TODO('c', ['a', 'b'], 'plan')],
  metadata: { owner: 'ops' },
};

describe('parseSpecDraft', () => {
  it('解析 JSON 对象草稿', () => {
    expect(parseSpecDraft(JSON.stringify(GOOD)).spec).toEqual(GOOD);
  });

  it('空文本 / 坏 JSON / 非对象给出中文错误', () => {
    expect(parseSpecDraft('')).toEqual({ error: '请输入工作流 JSON' });
    expect(parseSpecDraft('   ').error).toContain('请输入');
    expect(parseSpecDraft('{oops').error).toContain('JSON 解析失败');
    expect(parseSpecDraft('[1,2]').error).toContain('对象');
  });
});

describe('validateSpec', () => {
  it('合法完整 spec 返回 []', () => {
    expect(validateSpec(GOOD)).toEqual([]);
  });

  it('非对象 spec 只报一条', () => {
    expect(validateSpec(null)).toEqual([{ path: 'workflow', message: 'spec 必须是 JSON 对象' }]);
    expect(validateSpec([1])).toEqual([{ path: 'workflow', message: 'spec 必须是 JSON 对象' }]);
  });

  it('schema_version 不为 1', () => {
    const ps = validateSpec({ ...GOOD, schema_version: 2 });
    expect(ps[0]).toEqual({ path: 'workflow', message: 'schema_version 必须为 1' });
  });

  it('id / name / objective 非空检查（trim 后）', () => {
    const ps = validateSpec({ ...GOOD, id: '  ', name: '', objective: ' ' });
    expect(ps).toContainEqual({ path: 'workflow', message: 'workflow id 不能为空' });
    expect(ps).toContainEqual({ path: 'workflow', message: 'workflow name 不能为空' });
    expect(ps).toContainEqual({ path: 'workflow', message: 'workflow objective 不能为空' });
  });

  it('todos 缺失或为空数组时只报一条并提前返回', () => {
    expect(validateSpec({ ...GOOD, todos: [] })).toEqual([
      { path: 'workflow', message: 'todos 必须是非空数组' },
    ]);
    expect(validateSpec({ schema_version: 1, todos: 'nope' })).toEqual([
      { path: 'workflow', message: 'workflow id 不能为空' },
      { path: 'workflow', message: 'workflow name 不能为空' },
      { path: 'workflow', message: 'workflow objective 不能为空' },
      { path: 'workflow', message: 'todos 必须是非空数组' },
    ]);
  });

  it('id 重复挂 workflow', () => {
    const ps = validateSpec({ ...GOOD, todos: [GOOD_TODO('a'), GOOD_TODO('a')] });
    expect(ps).toEqual([{ path: 'workflow', message: 'TODO id 重复: a' }]);
  });

  it.each(['title', 'requirement_background', 'instructions'])('空 %s 报 TODO a %s 不能为空', (field) => {
    const spec = { ...GOOD, todos: [GOOD_TODO('a')] };
    spec.todos[0][field] = '   ';
    expect(validateSpec(spec)).toEqual([{ path: 'todos[a]', message: 'TODO a ' + field + ' 不能为空' }]);
  });

  it('acceptance 缺失 / criteria 非字符串 / 空白 都算空', () => {
    const missing = { ...GOOD, todos: [{ ...GOOD_TODO('a'), acceptance: undefined }] };
    expect(validateSpec(missing)).toEqual([
      { path: 'todos[a]', message: 'TODO a acceptance.criteria 不能为空' },
    ]);
    const bad = { ...GOOD, todos: [GOOD_TODO('a')] };
    bad.todos[0].acceptance = { criteria: 5 };
    expect(validateSpec(bad).map((p) => p.message)).toContain('TODO a acceptance.criteria 不能为空');
  });

  it.each([0, -1, 1.5, undefined])('max_attempts %p 必须为正整数', (v) => {
    const spec = { ...GOOD, todos: [GOOD_TODO('a')] };
    spec.todos[0].max_attempts = v;
    expect(validateSpec(spec)).toEqual([
      { path: 'todos[a]', message: 'TODO a max_attempts 必须为正整数' },
    ]);
  });

  it.each(['a/b', 'a\\b', 'a..b', 'a\0b'])('id %j 不安全（/ \\ .. 空字节）', (bad) => {
    const spec = { ...GOOD, todos: [GOOD_TODO(bad)] };
    expect(validateSpec(spec)).toEqual([
      { path: 'todos[' + bad + ']', message: 'TODO ' + bad + ' id 不安全（不能含 / \\ .. 或空字节）' },
    ]);
  });

  it('depends_on 非数组', () => {
    const spec = { ...GOOD, todos: [GOOD_TODO('a', 'oops')] };
    expect(validateSpec(spec)).toEqual([
      { path: 'todos[a]', message: 'TODO a depends_on 必须是字符串数组' },
    ]);
  });

  it('自依赖与未知依赖', () => {
    const spec = { ...GOOD, todos: [GOOD_TODO('a', ['a']), GOOD_TODO('b', ['ghost'])] };
    const ps = validateSpec(spec);
    expect(ps).toContainEqual({ path: 'todos[a]', message: 'TODO a 依赖不能指向自身' });
    expect(ps).toContainEqual({ path: 'todos[b]', message: 'TODO b 依赖了不存在的 TODO: ghost' });
  });

  it('required_tool_calls 非数组', () => {
    const spec = { ...GOOD, todos: [GOOD_TODO('a')] };
    spec.todos[0].acceptance = { criteria: 'ok', required_tool_calls: {} };
    expect(validateSpec(spec)).toEqual([
      { path: 'todos[a]', message: 'TODO a required_tool_calls 必须是数组' },
    ]);
  });

  it.each([
    ['name 为空', { name: '  ', arguments_contains: {}, result_ok: true }],
    ['arguments_contains 是数组', { name: 'tap', arguments_contains: ['x'], result_ok: true }],
    ['arguments_contains 是字符串', { name: 'tap', arguments_contains: 'x', result_ok: true }],
    ['arguments_contains 是 null', { name: 'tap', arguments_contains: null, result_ok: true }],
    ['条目非对象', 'oops'],
  ])('required_tool_calls %s → 条目非法', (_, call) => {
    const spec = { ...GOOD, todos: [GOOD_TODO('a')] };
    spec.todos[0].acceptance = { criteria: 'ok', required_tool_calls: [call] };
    expect(validateSpec(spec)).toEqual([
      {
        path: 'todos[a]',
        message: 'TODO a required_tool_calls 条目非法（name 须非空，arguments_contains 须为对象）',
      },
    ]);
  });

  it('合法 required_tool_calls 通过', () => {
    const spec = { ...GOOD, todos: [GOOD_TODO('a')] };
    spec.todos[0].acceptance = {
      criteria: 'ok',
      required_tool_calls: [{ name: 'mcp__fk__tap', arguments_contains: { repo: 'op' }, result_ok: false }],
    };
    expect(validateSpec(spec)).toEqual([]);
  });

  it('agent 空 / workflow 拒绝，未知 agent 放行', () => {
    expect(validateSpec({ ...GOOD, todos: [GOOD_TODO('a', [], '')] })).toEqual([
      { path: 'todos[a]', message: 'TODO a agent 不能为空' },
    ]);
    expect(validateSpec({ ...GOOD, todos: [GOOD_TODO('a', [], 'workflow')] })).toEqual([
      { path: 'todos[a]', message: 'TODO a 不能使用 workflow agent' },
    ]);
    expect(validateSpec({ ...GOOD, todos: [GOOD_TODO('a', [], 'ghost-agent')] })).toEqual([]);
    expect(BUILTIN_AGENTS).toEqual(['act', 'plan', 'explore', 'build']);
  });

  it('无法定位 id（空白/缺失）的 todo 问题挂 workflow', () => {
    const spec = {
      ...GOOD,
      todos: [GOOD_TODO('a'), { ...GOOD_TODO('b'), id: '   ', title: '' }, { title: '' }],
    };
    const ps = validateSpec(spec);
    expect(ps.every((p) => p.path === 'workflow')).toBe(true);
    expect(ps.map((p) => p.message)).toContain('TODO id 不能为空');
    expect(ps.filter((p) => p.message.includes('title 不能为空')).length).toBe(2);
  });

  it('环问题挂到 todos[id] 路径', () => {
    const ps = validateSpec({ ...GOOD, todos: [GOOD_TODO('a', ['b']), GOOD_TODO('b', ['a'])] });
    expect(ps).toEqual([{ path: 'todos[a]', message: '依赖图存在环（涉及 a）' }]);
  });
});

describe('findCycle', () => {
  it('无环图与空输入返回 null', () => {
    expect(findCycle(GOOD)).toBeNull();
    expect(findCycle(null)).toBeNull();
    expect(findCycle({ todos: [] })).toBeNull();
  });

  it('两节点环 / 三节点环返回环上首个节点', () => {
    const two = { todos: [GOOD_TODO('a', ['b']), GOOD_TODO('b', ['a'])] };
    expect(findCycle(two)).toBe('a');
    const three = {
      todos: [GOOD_TODO('a', ['b']), GOOD_TODO('b', ['c']), GOOD_TODO('c', ['a'])],
    };
    expect(['a', 'b', 'c']).toContain(findCycle(three));
  });

  it('自依赖也算环', () => {
    expect(findCycle({ todos: [GOOD_TODO('a', ['a'])] })).toBe('a');
  });

  it('2000 节点线性链不成环也不爆栈（迭代 DFS）', () => {
    const todos = [];
    for (let i = 0; i < 2000; i += 1) {
      todos.push(GOOD_TODO('t' + i, i ? ['t' + (i - 1)] : []));
    }
    const spec = { ...GOOD, todos };
    expect(findCycle(spec)).toBeNull();
    expect(validateSpec(spec)).toEqual([]);
  });
});
