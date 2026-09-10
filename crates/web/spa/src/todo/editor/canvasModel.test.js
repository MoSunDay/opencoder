// canvasModel.test.js — pure model tests for the WorkflowSpec ↔ canvas
// conversions (specToCanvas / canvasToSpec roundtrip, ghost deps, kept
// cycle edges) and the edit-time predicates (canConnect / renameTodo /
// uniqueSlug / newTodo / specProblemIndex / specLevelProblems). No React,
// no jsdom — same node-side style as specValidate.test.js.
import { describe, expect, it } from 'vitest';
import {
  canConnect,
  canvasToSpec,
  newTodo,
  renameTodo,
  specLevelProblems,
  specProblemIndex,
  specToCanvas,
  uniqueSlug,
} from './canvasModel.js';

const TODO = (id, deps, over) => ({
  id,
  title: '任务 ' + id,
  requirement_background: '需求背景',
  instructions: '执行说明',
  depends_on: deps || [],
  agent: 'act',
  max_attempts: 3,
  acceptance: { criteria: '验收标准', required_tool_calls: [] },
  metadata: {},
  ...(over || {}),
});

const SPEC = {
  schema_version: 1,
  id: 'wf-1',
  name: '发布会筹备',
  objective: '按时完成发布',
  constraints: ['不许延期'],
  todos: [TODO('a'), TODO('b', ['a']), TODO('c', ['a', 'b'], { agent: 'plan' })],
  metadata: { owner: 'ops' },
};

describe('canvasModel specToCanvas', () => {
  it('节点按 todos 顺序生成并携带私有 todo 拷贝', () => {
    const { nodes } = specToCanvas(SPEC);
    expect(nodes.map((n) => n.id)).toEqual(['a', 'b', 'c']);
    expect(nodes[0].type).toBe('todoEdit');
    expect(nodes[0].position).toEqual({ x: 0, y: 0 });
    expect(nodes[0].data.todo).toEqual(SPEC.todos[0]);
    expect(nodes[0].data.todo).not.toBe(SPEC.todos[0]);
  });

  it('depends_on 生成边（id 为 e-<dep>-<id>）', () => {
    expect(specToCanvas(SPEC).edges).toEqual([
      { id: 'e-a-b', source: 'a', target: 'b' },
      { id: 'e-a-c', source: 'a', target: 'c' },
      { id: 'e-b-c', source: 'b', target: 'c' },
    ]);
  });

  it('自依赖 / 未知依赖 / 非字符串依赖不生成边，重复引用去重', () => {
    const bad = { ...SPEC, todos: [TODO('a', ['a', 'ghost', 5]), TODO('b')] };
    expect(specToCanvas(bad).edges).toEqual([]);
    const dup = { ...SPEC, todos: [TODO('a'), TODO('b', ['a', 'a'])] };
    expect(specToCanvas(dup).edges).toEqual([{ id: 'e-a-b', source: 'a', target: 'b' }]);
  });

  it('环边保留（画布要渲染并标红）', () => {
    const spec = { ...SPEC, todos: [TODO('a', ['b']), TODO('b', ['a'])] };
    expect(specToCanvas(spec).edges).toEqual([
      { id: 'e-b-a', source: 'b', target: 'a' },
      { id: 'e-a-b', source: 'a', target: 'b' },
    ]);
  });

  it('无字符串 id 的 todo 不生成节点', () => {
    const spec = { ...SPEC, todos: [TODO('a'), { title: 'x' }, null] };
    expect(specToCanvas(spec).nodes.map((n) => n.id)).toEqual(['a']);
  });
});

describe('canvasModel canvasToSpec', () => {
  it('specToCanvas → canvasToSpec 无损还原', () => {
    expect(canvasToSpec(specToCanvas(SPEC), SPEC)).toEqual(SPEC);
  });

  it('depends_on 由入边重建且顺序 = source 节点序', () => {
    const { nodes, edges } = specToCanvas(SPEC);
    const canvas = { nodes: [nodes[2], nodes[1], nodes[0]], edges: [edges[2], edges[1], edges[0]] };
    const back = canvasToSpec(canvas, SPEC);
    expect(back.todos.map((t) => t.id)).toEqual(['c', 'b', 'a']);
    expect(back.todos.find((t) => t.id === 'c').depends_on).toEqual(['b', 'a']);
  });

  it('ghost 依赖保留、去重且保持原顺序（不静默丢弃）', () => {
    const { nodes, edges } = specToCanvas(SPEC);
    nodes[2].data.todo.depends_on = ['ghost1', 'a', 'ghost1', 'ghost2'];
    const back = canvasToSpec({ nodes, edges }, SPEC);
    expect(back.todos.find((t) => t.id === 'c').depends_on).toEqual(['a', 'b', 'ghost1', 'ghost2']);
  });

  it('无边时画布内 id 的依赖随边消失，ghost 依赖仍在', () => {
    const { nodes } = specToCanvas(SPEC);
    nodes[1].data.todo.depends_on = ['a', 'ghost'];
    const back = canvasToSpec({ nodes, edges: [] }, SPEC);
    expect(back.todos.find((t) => t.id === 'b').depends_on).toEqual(['ghost']);
  });

  it('spec 级字段透传，schema_version 缺省 1', () => {
    const base = { id: 'wf-9', name: 'n', objective: 'o', constraints: ['c1'], metadata: { k: 1 } };
    const back = canvasToSpec(specToCanvas(SPEC), base);
    expect(back.schema_version).toBe(1);
    expect(back.id).toBe('wf-9');
    expect(back.name).toBe('n');
    expect(back.objective).toBe('o');
    expect(back.constraints).toEqual(['c1']);
    expect(back.constraints).not.toBe(base.constraints);
    expect(back.metadata).toEqual({ k: 1 });
    expect(canvasToSpec({ nodes: [], edges: [] }, {})).toEqual({
      schema_version: 1,
      id: '',
      name: '',
      objective: '',
      constraints: [],
      metadata: {},
      todos: [],
    });
  });
});

describe('canvasModel uniqueSlug / newTodo', () => {
  it('uniqueSlug 空闲返回原名，冲突时递增 -2 / -3', () => {
    expect(uniqueSlug('x', [])).toBe('x');
    expect(uniqueSlug('x', ['x'])).toBe('x-2');
    expect(uniqueSlug('x', ['x', 'x-2'])).toBe('x-3');
    expect(uniqueSlug('x', new Set(['x']))).toBe('x-2');
  });

  it('newTodo 生成编辑器默认值与唯一 id', () => {
    expect(newTodo([])).toEqual({
      id: 'todo',
      title: '',
      agent: 'act',
      depends_on: [],
      max_attempts: 3,
      requirement_background: '',
      instructions: '',
      acceptance: { criteria: '' },
    });
    expect(newTodo(['todo', 'todo-2']).id).toBe('todo-3');
  });
});

describe('canvasModel canConnect', () => {
  it('自连 / 重复 / 成环分别给出中文理由，其余可连', () => {
    expect(canConnect([], 'a', 'a')).toBe('不能连接到自身');
    const edges = [
      { id: 'e-a-b', source: 'a', target: 'b' },
      { id: 'e-b-c', source: 'b', target: 'c' },
    ];
    expect(canConnect(edges, 'a', 'b')).toBe('依赖已存在');
    expect(canConnect([{ id: 'e-b-a', source: 'b', target: 'a' }], 'a', 'b')).toBe('不能形成循环依赖');
    expect(canConnect(edges, 'a', 'c')).toBeNull();
  });
});

describe('canvasModel renameTodo', () => {
  it('成功重命名同步节点 id、data.todo.id、边与 depends_on 引用', () => {
    const canvas = specToCanvas(SPEC);
    const r = renameTodo(canvas, 'a', 'z');
    expect(r.ok).toBe(true);
    expect(r.canvas.nodes.map((n) => n.id)).toEqual(['z', 'b', 'c']);
    expect(r.canvas.nodes[0].data.todo.id).toBe('z');
    expect(r.canvas.edges).toEqual([
      { id: 'e-z-b', source: 'z', target: 'b' },
      { id: 'e-z-c', source: 'z', target: 'c' },
      { id: 'e-b-c', source: 'b', target: 'c' },
    ]);
    expect(r.canvas.nodes[1].data.todo.depends_on).toEqual(['z']);
    expect(r.canvas.nodes[2].data.todo.depends_on).toEqual(['z', 'b']);
  });

  it('depends_on 中未连边的引用（ghost 引用）也同步', () => {
    const { nodes } = specToCanvas(SPEC);
    const r = renameTodo({ nodes, edges: [] }, 'a', 'z');
    expect(r.canvas.nodes[2].data.todo.depends_on).toEqual(['z', 'b']);
  });

  it('空白 / 重复 id 拒绝，候选 id 会 trim', () => {
    const canvas = specToCanvas(SPEC);
    expect(renameTodo(canvas, 'a', '  ').error).toContain('不能为空');
    const dup = renameTodo(canvas, 'a', 'b');
    expect(dup.error).toContain('已存在');
    expect(dup.canvas).toBeUndefined();
    const t = renameTodo(canvas, 'a', ' z ');
    expect(t.ok).toBe(true);
    expect(t.canvas.nodes[0].id).toBe('z');
  });

  it('不修改输入画布（不可变风格）', () => {
    const canvas = specToCanvas(SPEC);
    const before = JSON.parse(JSON.stringify(canvas));
    renameTodo(canvas, 'a', 'z');
    expect(canvas).toEqual(before);
  });
});

describe('canvasModel specProblemIndex / specLevelProblems', () => {
  const PROBLEMS = [
    { path: 'todos[a]', message: 'TODO a title 不能为空' },
    { path: 'workflow', message: 'workflow name 不能为空' },
    { path: 'todos[b]', message: 'TODO b agent 不能为空' },
    { path: 'todos[a]', message: 'TODO a max_attempts 必须为正整数' },
    { message: '裸问题' },
  ];

  it('todos[id] 条目按 id 分组收集 message', () => {
    const idx = specProblemIndex(PROBLEMS);
    expect(idx.get('a')).toEqual(['TODO a title 不能为空', 'TODO a max_attempts 必须为正整数']);
    expect(idx.get('b')).toEqual(['TODO b agent 不能为空']);
    expect(idx.size).toBe(2);
    expect(specProblemIndex([]).size).toBe(0);
  });

  it('workflow 级与无法定位的条目进入 specLevelProblems', () => {
    expect(specLevelProblems(PROBLEMS)).toEqual(['workflow name 不能为空', '裸问题']);
    expect(specLevelProblems([])).toEqual([]);
  });
});
