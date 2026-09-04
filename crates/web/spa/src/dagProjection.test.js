// dagProjection.test.js — pure fold/projection contract for the DAG run
// view. The wire shapes mirror crates/dag/src/protocol.rs (DagEventView /
// DagRunView / DagSpec); the projection rules are the ones runDetail.jsx
// renders (step colors, skipped deps, feed text).
import { describe, expect, it } from 'vitest';
import {
  dropCycleEdges,
  foldStepStates,
  frameToEvent,
  graphFromSpec,
  nodeBadgeText,
  outputPreview,
  projectStepStatuses,
  runStatusLabel,
  runStatusTag,
  specSteps,
} from './dagProjection.js';

const ev = (seq, kind, step, payload, at_ms = seq) => ({ seq, kind, step, payload, at_ms });

const SPEC = {
  name: 'etl',
  steps: [
    { name: 'fetch', kind: { type: 'python', code: 'print(1)' } },
    { name: 'transform', depends_on: ['fetch'], kind: { type: 'python', code: 'print(2)' } },
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

describe('foldStepStates', () => {
  it('folds started→done(ok) with the output snapshot', () => {
    const m = foldStepStates([
      ev(1, 'step_started', 'fetch'),
      ev(2, 'step_done', 'fetch', { ok: true, output: 'hello\nworld' }),
    ]);
    expect(m.get('fetch')).toEqual({ status: 'done', output: 'hello\nworld', error: '', at_ms: 2 });
  });

  it('folds a failed step_done into error with the error text', () => {
    const m = foldStepStates([ev(3, 'step_done', 'x', { ok: false, error: 'boom', output: 'partial' })]);
    expect(m.get('x')).toEqual({ status: 'error', output: 'partial', error: 'boom', at_ms: 3 });
  });

  it('treats a missing ok as success (payload {} → done)', () => {
    expect(foldStepStates([ev(1, 'step_done', 'x', {})]).get('x').status).toBe('done');
  });

  it('sorts by seq so a late-arriving replay still folds in order', () => {
    const m = foldStepStates([
      ev(2, 'step_done', 'a', { ok: true, output: 'out' }),
      ev(1, 'step_started', 'a'),
    ]);
    expect(m.get('a').status).toBe('done');
    expect(m.get('a').output).toBe('out');
  });

  it('collapses duplicate seqs (replay + live double-delivery is idempotent)', () => {
    const m = foldStepStates([
      ev(1, 'step_started', 'a'),
      ev(1, 'step_started', 'a'),
      ev(2, 'step_done', 'a', { ok: true }),
      ev(2, 'step_done', 'a', { ok: true }),
    ]);
    expect(m.get('a').status).toBe('done');
    expect(m.size).toBe(1);
  });

  it('keeps the latest step_started as running and ignores run-level kinds', () => {
    const m = foldStepStates([
      ev(1, 'run_started', null, { node_id: 'n1' }),
      ev(2, 'step_started', 'a'),
      ev(3, 'run_finished', null, { status: 'done' }),
    ]);
    expect(m.size).toBe(1);
    expect(m.get('a')).toMatchObject({ status: 'running', at_ms: 2 });
  });

  it('degrades on garbage input instead of throwing', () => {
    expect(foldStepStates(null)).toEqual(new Map());
    expect(foldStepStates([null, 3, { kind: 'step_started' }]).size).toBe(0);
  });
});

describe('projectStepStatuses', () => {
  it('pending without events, folded statuses win', () => {
    const states = foldStepStates([ev(1, 'step_done', 'fetch', { ok: true })]);
    const p = projectStepStatuses(SPEC, states);
    expect(p.get('fetch')).toBe('done');
    expect(p.get('transform')).toBe('pending');
  });

  it('skips steps whose deps failed, transitively', () => {
    const states = foldStepStates([ev(1, 'step_done', 'fetch', { ok: false, error: 'net' })]);
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
        { name: 'a', kind: { type: 'python', code: 'x' } },
        { name: 'b', kind: { type: 'python', code: 'x' } },
        { name: 'c', depends_on: ['a', 'b'], kind: { type: 'python', code: 'x' } },
        { name: 'd', depends_on: ['c'], kind: { type: 'python', code: 'x' } },
      ],
    };
    const states = foldStepStates([
      ev(1, 'step_done', 'a', { ok: true }),
      ev(2, 'step_done', 'b', { ok: false, error: 'x' }),
      ev(3, 'step_started', 'c'),
    ]);
    const p = projectStepStatuses(spec, states);
    expect(p.get('c')).toBe('running'); // already started: never re-projected
    expect(p.get('d')).toBe('pending'); // dep c is not (yet) error/skipped
  });

  it('unknown depends_on names never block or crash the fixpoint', () => {
    const spec = { name: 'x', steps: [{ name: 'a', depends_on: ['ghost'], kind: { type: 'python', code: 'x' } }] };
    expect(projectStepStatuses(spec, new Map()).get('a')).toBe('pending');
  });
});

describe('graphFromSpec', () => {
  it('builds React-Flow shapes: ids are slugs, edges follow depends_on', () => {
    const states = foldStepStates([
      ev(1, 'step_done', 'fetch', { ok: true, output: 'rows=3' }),
      ev(2, 'step_started', 'transform'),
    ]);
    const { nodes, edges } = graphFromSpec(SPEC, states);
    expect(nodes.map((n) => n.id)).toEqual(['fetch', 'transform', 'review', 'publish']);
    expect(nodes[0]).toMatchObject({
      type: 'dagStep',
      data: { label: 'fetch', status: 'done', kindType: 'python', output: 'rows=3' },
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
        { name: 'a', depends_on: ['ghost', 'c', 'c'], kind: { type: 'python', code: 'x' } },
        { name: 'b', depends_on: ['a'], kind: { type: 'python', code: 'x' } },
        { name: 'c', depends_on: ['b'], kind: { type: 'python', code: 'x' } },
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

  it('projected node statuses include skipped (dep failure) without events', () => {
    const states = foldStepStates([ev(1, 'step_done', 'fetch', { ok: false, error: 'x' })]);
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

describe('display tokens', () => {
  it('runStatusTag maps every DagRunView status to an antd color token', () => {
    expect(runStatusTag('pending')).toBe('default');
    expect(runStatusTag('running')).toBe('processing');
    expect(runStatusTag('cancelling')).toBe('orange');
    expect(runStatusTag('done')).toBe('success');
    expect(runStatusTag('error')).toBe('red');
    expect(runStatusTag('cancelled')).toBe('grey');
    expect(runStatusTag(undefined)).toBe('default');
    expect(runStatusTag('weird')).toBe('default');
  });

  it('runStatusLabel is Chinese with a raw fallback', () => {
    expect(runStatusLabel('running')).toBe('运行中');
    expect(runStatusLabel('cancelled')).toBe('已取消');
    expect(runStatusLabel('nope')).toBe('nope');
  });

  it('nodeBadgeText shows the node id or the unclaimed hint', () => {
    expect(nodeBadgeText('node-42')).toBe('node-42');
    expect(nodeBadgeText(null)).toBe('任意节点排队中');
    expect(nodeBadgeText(undefined)).toBe('任意节点排队中');
    expect(nodeBadgeText('')).toBe('任意节点排队中');
    expect(nodeBadgeText(null, '—')).toBe('—');
  });

  it('outputPreview passes short snapshots through and clips long ones with a marker', () => {
    expect(outputPreview('ok')).toBe('ok');
    expect(outputPreview('', 5)).toBe('');
    expect(outputPreview(null)).toBe('');
    const long = 'x'.repeat(50);
    expect(outputPreview(long, 10)).toBe('xxxxxxxxxx…(已截断)');
  });
});
