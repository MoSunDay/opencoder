// DAG event normalization, snapshot projection and graph layout contracts.
import { describe, expect, it } from 'vitest';
import { NODE_W } from './dag/dagLayout.js';
import { dropCycleEdges, frameToEvent, graphFromSpec, nodeBadgeText, projectStepStatuses, specSteps } from './dagProjection.js';

const SPEC = {
  name: 'etl',
  steps: [
    { name: 'fetch', kind: { type: 'binary', resource: 'tool' } },
    { name: 'transform', depends_on: ['fetch'], kind: { type: 'binary', resource: 'tool.binary --load' } },
    { name: 'review', depends_on: ['transform'], kind: { type: 'agent', prompt: 'review' } },
    { name: 'publish', depends_on: ['review'], kind: { type: 'agent', prompt: 'publish' } },
  ],
};

describe('frameToEvent', () => {
  it('normalizes an sse.js frame and prefers data.seq over the transport id', () => {
    const e = frameToEvent({ event: 'step_done', data: { seq: 7, kind: 'step_done', step: 'a', payload: { ok: true }, at_ms: 123 }, seq: 99 });
    expect(e).toEqual({ seq: 7, kind: 'step_done', step: 'a', payload: { ok: true }, at_ms: 123 });
  });

  it('falls back to the SSE id line seq and to a null seq for live frames', () => {
    expect(frameToEvent({ event: 'run_started', data: { kind: 'run_started', payload: {} }, seq: 4 }).seq).toBe(4);
    expect(frameToEvent({ event: 'step_started', data: { kind: 'step_started', step: 'a' } }).seq).toBeNull();
  });

  it('rejects unknown kinds and step events without a step name, defaults payload', () => {
    expect(frameToEvent({ event: 'message', data: { raw: 'x' } })).toBeNull();
    expect(frameToEvent({ event: 'step_done', data: { kind: 'step_done' } })).toBeNull();
    const e = frameToEvent({ event: 'step_started', data: { kind: 'step_started', step: 'a' } });
    expect(e.payload).toEqual({});
    expect(e.at_ms).toBeNull();
  });
});

describe('projectStepStatuses', () => {
  it('pending without a snapshot state, known statuses win', () => {
    const states = new Map([['fetch', { status: 'done' }]]);
    const p = projectStepStatuses(SPEC, states);
    expect(p.get('fetch')).toBe('done');
    expect(p.get('transform')).toBe('pending');
  });

  it('skips steps whose deps failed, transitively', () => {
    const states = new Map([['fetch', { status: 'error', error: 'net' }]]);
    const p = projectStepStatuses(SPEC, states);
    expect(p.get('fetch')).toBe('error');
    expect(p.get('transform')).toBe('skipped');
    expect(p.get('review')).toBe('skipped'); // transitively via transform
    expect(p.get('publish')).toBe('skipped');
  });

  it('does not skip a RUNNING step whose sibling dep failed, and done deps never skip', () => {
    const spec = {
      name: 'mixed',
      steps: [
        { name: 'a', kind: { type: 'binary', resource: 'tool' } },
        { name: 'b', kind: { type: 'binary', resource: 'tool' } },
        { name: 'c', depends_on: ['a', 'b'], kind: { type: 'binary', resource: 'tool' } },
        { name: 'd', depends_on: ['c'], kind: { type: 'binary', resource: 'tool' } },
      ],
    };
    const states = new Map([
      ['a', { status: 'done' }],
      ['b', { status: 'error', error: 'x' }],
      ['c', { status: 'running' }],
    ]);
    const p = projectStepStatuses(spec, states);
    expect(p.get('c')).toBe('running'); // already started: never re-projected
    expect(p.get('d')).toBe('pending'); // dep c is not (yet) error/skipped
  });

  it('unknown depends_on names never block or crash the fixpoint', () => {
    const spec = { name: 'x', steps: [{ name: 'a', depends_on: ['ghost'], kind: { type: 'binary', resource: 'tool' } }] };
    expect(projectStepStatuses(spec, new Map()).get('a')).toBe('pending');
  });
});

describe('graphFromSpec', () => {
  it('builds React-Flow shapes: ids are slugs, edges follow depends_on', () => {
    const states = new Map([
      ['fetch', { status: 'done', output: 'rows=3' }],
      ['transform', { status: 'running' }],
    ]);
    const { nodes, edges } = graphFromSpec(SPEC, states);
    expect(nodes.map((n) => n.id)).toEqual(['fetch', 'transform', 'review', 'publish']);
    expect(nodes[0]).toMatchObject({
      type: 'dagStep',
      data: { label: 'fetch', status: 'done', kindType: 'binary', output: 'rows=3' },
    });
    expect(edges.map((e) => e.source + '>' + e.target)).toEqual([
      'fetch>transform',
      'transform>review',
      'review>publish',
    ]);
  });

  it('assigns finite dagre positions flowing left→right along deps', () => {
    const { nodes } = graphFromSpec(SPEC, new Map());
    const by = new Map(nodes.map((n) => [n.id, n]));
    for (const n of nodes) {
      expect(Number.isFinite(n.position.x)).toBe(true);
      expect(Number.isFinite(n.position.y)).toBe(true);
    }
    expect(by.get('publish').position.x).toBeGreaterThan(by.get('fetch').position.x);
  });

  it('drops unknown dep references, duplicate edges and cycle-closing edges', () => {
    const cyclic = {
      name: 'c',
      steps: [
        { name: 'a', depends_on: ['ghost', 'c', 'c'], kind: { type: 'binary', resource: 'tool' } },
        { name: 'b', depends_on: ['a'], kind: { type: 'binary', resource: 'tool' } },
        { name: 'c', depends_on: ['b'], kind: { type: 'binary', resource: 'tool' } },
      ],
    };
    const { nodes, edges } = graphFromSpec(cyclic, new Map());
    expect(nodes).toHaveLength(3);
    // Spec edge order: c→a (from a's dep), a→b, b→c. Kept in order, the
    // LAST edge that closes the cycle (b→c, target reaches source c→a→b)
    // is the dropped one; earlier edges win.
    const ids = edges.map((e) => e.source + '>' + e.target);
    expect(ids).toEqual(['c>a', 'a>b']);
    expect(new Set(ids).size).toBe(ids.length); // duplicate 'c' dep collapsed
  });

  it('degrades to an empty graph on a malformed spec', () => {
    expect(graphFromSpec(null, new Map())).toEqual({ nodes: [], edges: [] });
    expect(graphFromSpec({ steps: 'nope' }, new Map()).nodes).toEqual([]);
    expect(specSteps(undefined)).toEqual([]);
  });

  it('节点声明固定 width 与左右 handles 且不声明 height（边首帧即渲染、卡片高度归 RO）', () => {
    const { nodes } = graphFromSpec(SPEC, new Map());
    expect(nodes.length).toBeGreaterThan(0);
    nodes.forEach((n, i) => {
      expect(n.width).toBe(NODE_W);
      // height MUST stay undeclared: a declared height bakes a permanent
      // inline height onto the wrapper, clamping auto-height cards (error
      // text grows them) and pinning handle centers + fitView measurement.
      expect(n.height).toBeUndefined();
      expect(n.handles).toHaveLength(2);
      const target = n.handles.find((h) => h.type === 'target');
      const source = n.handles.find((h) => h.type === 'source');
      expect(target).toMatchObject({ position: 'left', width: 6, height: 6 });
      expect(source).toMatchObject({ position: 'right', width: 6, height: 6 });
      // fresh objects per node: React Flow mutates handle entries in place
      expect(n.handles).not.toBe(nodes[(i + 1) % nodes.length].handles);
    });
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
    const { edges } = graphFromSpec(spec, new Map());
    expect(edges).toHaveLength(2);
    const ids = edges.map((e) => e.id);
    expect(new Set(ids).size).toBe(2);
    expect(ids.sort()).toEqual(['e-a-b>c', 'e-a>b-c']);
  });

  it('projected node statuses include skipped (dep failure) without events', () => {
    const states = new Map([['fetch', { status: 'error', error: 'x' }]]);
    const { nodes } = graphFromSpec(SPEC, states);
    const by = new Map(nodes.map((n) => [n.id, n.data.status]));
    expect(by.get('fetch')).toBe('error');
    expect(by.get('transform')).toBe('skipped');
    expect(by.get('publish')).toBe('skipped');
  });
});

describe('dropCycleEdges', () => {
  it('keeps a clean DAG untouched and in input order', () => {
    const edges = [
      { id: '1', source: 'a', target: 'b' },
      { id: '2', source: 'a', target: 'c' },
      { id: '3', source: 'b', target: 'd' },
    ];
    expect(dropCycleEdges(edges)).toEqual(edges);
  });

  it('breaks a two-node cycle by dropping exactly one edge', () => {
    const kept = dropCycleEdges([
      { id: '1', source: 'a', target: 'b' },
      { id: '2', source: 'b', target: 'a' },
    ]);
    expect(kept).toHaveLength(1);
  });
});

// display tokens: runStatusTag/runStatusLabel moved to src/ui/statusTag.jsx
// (console-wide mapping); their contract is covered by statusTag.dom.test.jsx.
describe('display tokens', () => {
  it('nodeBadgeText shows the node id or the unclaimed hint', () => {
    expect(nodeBadgeText('node-42')).toBe('node-42');
    expect(nodeBadgeText(null)).toBe('任意节点排队中');
    expect(nodeBadgeText(undefined)).toBe('任意节点排队中');
    expect(nodeBadgeText('')).toBe('任意节点排队中');
    expect(nodeBadgeText(null, '—')).toBe('—');
  });
});
