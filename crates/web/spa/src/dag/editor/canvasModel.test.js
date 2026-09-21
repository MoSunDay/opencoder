// canvasModel.test.js — pure model tests for the DAG spec ↔ canvas
// conversions (specToCanvas / canvasToSpec roundtrip losslessness, ghost
// deps, self/duplicate dep handling, kept cycle edges) and the edit-time
// predicates (canConnect / renameStep / uniqueSlug / newStep /
// changeStepKind / specProblemIndex). Same node-side style as
// ../specValidate.test.js — no React, no jsdom.

import { describe, expect, it } from 'vitest';
import { EDIT_NODE_W } from './canvasLayout.js';
import {
  canConnect,
  canvasToSpec,
  editNodeBox,
  changeStepKind,
  newStep,
  renameStep,
  specLevelProblems,
  specProblemIndex,
  specToCanvas,
  uniqueSlug,
} from './canvasModel.js';

const SPEC = {
  name: 'etl',
  description: 'demo',
  steps: [
    { name: 'fetch', kind: { type: 'wasm', command: 'tool.wasm', sandbox: 'runc' }, timeout_secs: 120 },
    { name: 'review', kind: { type: 'agent', prompt: 'review it', agent: 'reviewer', model: 'gpt' } },
    { name: 'load', depends_on: ['fetch', 'review'], kind: { type: 'wasm', command: 'tool.wasm' } },
  ],
};

const roundtrip = (spec) => {
  const canvas = specToCanvas(spec);
  return { canvas, spec: canvasToSpec(canvas, spec) };
};

describe('canvasModel roundtrip', () => {
  it('specToCanvas → canvasToSpec 无损还原代表 spec（wasm 沙箱/超时 + agent 字段 + 依赖）', () => {
    const { spec } = roundtrip(SPEC);
    expect(JSON.parse(JSON.stringify(spec))).toEqual(SPEC);
  });

  it('roundtrip 保持节点顺序（4 个步骤按原顺序输出）', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 's1', kind: { type: 'wasm', command: 'tool.wasm' } },
        { name: 's2', depends_on: ['s1'], kind: { type: 'wasm', command: 'tool.wasm' } },
        { name: 's3', depends_on: ['s1'], kind: { type: 'agent', prompt: 'c' } },
        { name: 's4', depends_on: ['s2', 's3'], kind: { type: 'agent', prompt: 'd' } },
      ],
    };
    const { spec: back } = roundtrip(spec);
    expect(back.steps.map((s) => s.name)).toEqual(['s1', 's2', 's3', 's4']);
    expect(back.steps[3].depends_on).toEqual(['s2', 's3']);
  });

  it('幽灵依赖（未定义步骤）不生成连线但 roundtrip 后保留', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 'a', kind: { type: 'wasm', command: 'tool.wasm' } },
        { name: 'b', depends_on: ['ghost'], kind: { type: 'wasm', command: 'tool.wasm' } },
      ],
    };
    const { canvas, spec: back } = roundtrip(spec);
    expect(canvas.edges).toHaveLength(0);
    expect(back.steps[1].depends_on).toEqual(['ghost']);
  });

  it('自身依赖被丢弃（无连线，roundtrip 后 depends_on 整体省略）', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 'a', kind: { type: 'wasm', command: 'tool.wasm' } },
        { name: 'b', depends_on: ['b'], kind: { type: 'wasm', command: 'tool.wasm' } },
      ],
    };
    const { canvas, spec: back } = roundtrip(spec);
    expect(canvas.edges).toHaveLength(0);
    expect(back.steps[1].depends_on).toBeUndefined();
  });

  it('roundtrip 保留顶层 max_concurrency（画布结构编辑不丢并发配置）', () => {
    const { spec } = roundtrip({ ...SPEC, max_concurrency: 8 });
    expect(spec.max_concurrency).toBe(8);
  });

  it('roundtrip 省略未设置 / undefined 的 max_concurrency（落库走服务端默认 4）', () => {
    const { spec } = roundtrip(SPEC);
    expect('max_concurrency' in spec).toBe(false);
    const { spec: back } = roundtrip({ ...SPEC, max_concurrency: undefined });
    expect('max_concurrency' in back).toBe(false);
  });

  it('重复依赖去重为单条连线', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 'a', kind: { type: 'wasm', command: 'tool.wasm' } },
        { name: 'b', depends_on: ['a', 'a'], kind: { type: 'wasm', command: 'tool.wasm' } },
      ],
    };
    const { canvas, spec: back } = roundtrip(spec);
    expect(canvas.edges).toHaveLength(1);
    expect(back.steps[1].depends_on).toEqual(['a']);
  });

  it('节点声明固定 width 与左右 handles 且不声明 height（边首帧即渲染、卡片高度归 RO）', () => {
    const { nodes } = specToCanvas(SPEC);
    expect(nodes.length).toBeGreaterThan(0);
    nodes.forEach((n, i) => {
      expect(n.width).toBe(EDIT_NODE_W);
      // height MUST stay undeclared: a declared height bakes a permanent
      // inline height onto the wrapper, clamping auto-height cards and
      // pinning handle centers + fitView measurement.
      expect(n.height).toBeUndefined();
      expect(n.handles).toHaveLength(2);
      expect(n.handles.find((h) => h.type === 'target')).toMatchObject({ position: 'left', width: 10, height: 10 });
      expect(n.handles.find((h) => h.type === 'source')).toMatchObject({ position: 'right', width: 10, height: 10 });
      // fresh per node: React Flow mutates handle entries in place
      expect(n.handles).not.toBe(nodes[(i + 1) % nodes.length].handles);
    });
    expect(editNodeBox()).toEqual(editNodeBox()); // stable shape
    expect(editNodeBox().handles).not.toBe(editNodeBox().handles); // fresh objects
  });

  it('连字符命名不撞边 id（a→b-c 与 a-b→c 不再折叠成同一条边）', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 'a', kind: { type: 'agent', prompt: 'p' } },
        { name: 'a-b', kind: { type: 'agent', prompt: 'p' } },
        { name: 'b-c', depends_on: ['a'], kind: { type: 'agent', prompt: 'p' } },
        { name: 'c', depends_on: ['a-b'], kind: { type: 'agent', prompt: 'p' } },
      ],
    };
    const { edges } = specToCanvas(spec);
    expect(edges).toHaveLength(2);
    const ids = edges.map((e) => e.id);
    expect(new Set(ids).size).toBe(2);
    expect(ids.sort()).toEqual(['e-a-b>c', 'e-a>b-c']);
  });

  it('循环依赖的两条连线都被保留（编辑器要能渲染并标红）', () => {
    const spec = {
      name: 'x',
      steps: [
        { name: 'a', depends_on: ['b'], kind: { type: 'wasm', command: 'tool.wasm' } },
        { name: 'b', depends_on: ['a'], kind: { type: 'wasm', command: 'tool.wasm' } },
      ],
    };
    const { canvas, spec: back } = roundtrip(spec);
    expect(canvas.edges.map((e) => e.id).sort()).toEqual(['e-a>b', 'e-b>a']);
    expect(JSON.parse(JSON.stringify(back))).toEqual(spec);
  });
});

describe('canvasModel canConnect', () => {
  it('拒绝自身 / 重复 / 成环连接并给出中文原因', () => {
    expect(canConnect([], 'x', 'x')).toContain('自身');
    expect(canConnect([{ source: 'a', target: 'b' }], 'a', 'b')).toContain('依赖已存在');
    expect(canConnect([{ source: 'a', target: 'b' }], 'b', 'a')).toContain('循环');
  });

  it('三节点路径上反向成环也被拦截；合法连接返回 null', () => {
    const edges = [
      { id: 'e-a>b', source: 'a', target: 'b' },
      { id: 'e-b>c', source: 'b', target: 'c' },
    ];
    expect(canConnect(edges, 'c', 'a')).toContain('循环');
    expect(canConnect(edges, 'b', 'd')).toBeNull();
  });
});

describe('canvasModel renameStep / uniqueSlug / newStep', () => {
  it('renameStep 校验 slug 字符集、长度与重名', () => {
    expect(renameStep('Bad_Name', [])).toContain('slug');
    expect(renameStep('UPPER', [])).toContain('slug');
    expect(renameStep('a'.repeat(65), [])).toContain('slug');
    expect(renameStep('a'.repeat(64), [])).toBeNull();
    expect(renameStep('good-1', [])).toBeNull();
    expect(renameStep('a', ['a', 'b'])).toContain('已存在');
  });

  it('uniqueSlug 空闲返回原名，冲突时递增 -2 / -3', () => {
    expect(uniqueSlug('x', [])).toBe('x');
    expect(uniqueSlug('x', ['x'])).toBe('x-2');
    expect(uniqueSlug('x', ['x', 'x-2'])).toBe('x-3');
  });

  it('newStep 生成唯一 slug 名与对应类型的空负载', () => {
    expect(newStep('agent', [])).toEqual({ name: 'step', kind: { type: 'agent', prompt: '' } });
    const py = newStep('wasm', ['step']);
    expect(py.name).toBe('step-2');
    expect(py.kind).toEqual({ type: 'wasm', command: '' });
  });
});

describe('canvasModel changeStepKind', () => {
  it('agent → wasm 重置负载但保留 name 与 timeout_secs', () => {
    const step = { name: 'a', timeout_secs: 60, kind: { type: 'agent', prompt: 'p', agent: 'g', model: 'm' } };
    expect(changeStepKind(step, 'wasm')).toEqual({
      name: 'a',
      timeout_secs: 60,
      kind: { type: 'wasm', command: '' },
    });
  });

  it('wasm → agent 同样重置负载并保留 name', () => {
    const step = { name: 'b', kind: { type: 'wasm', command: 'tool.wasm', sandbox: 'runc' } };
    expect(changeStepKind(step, 'agent')).toEqual({ name: 'b', kind: { type: 'agent', prompt: '' } });
  });
});

describe('canvasModel specProblemIndex', () => {
  const PROBLEMS = [
    'steps[0].kind.type 必须是 agent | wasm',
    'spec.name 必须是非空字符串',
    'steps[1].depends_on 存在重复项',
  ];
  const NODES = [{ id: 'a' }, { id: 'b' }];

  it('steps[N] 前缀的问题按节点序号挂到对应节点 id 上', () => {
    const idx = specProblemIndex(PROBLEMS, NODES);
    expect(idx.get('a')).toEqual(['steps[0].kind.type 必须是 agent | wasm']);
    expect(idx.get('b')).toEqual(['steps[1].depends_on 存在重复项']);
    expect(idx.size).toBe(2);
  });

  it('specLevelProblems 只返回非 steps[N] 的 spec 级问题', () => {
    expect(specLevelProblems(PROBLEMS)).toEqual(['spec.name 必须是非空字符串']);
    expect(specLevelProblems([])).toEqual([]);
  });
});
