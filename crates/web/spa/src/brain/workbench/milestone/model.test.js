import { describe, expect, it } from 'vitest';
import { connect, groups, milestone, moveMilestone, removeMilestone, validateGraph, visits } from './model.js';
const cap = { id: 'a' };
const node = (id, layer) => ({ ...milestone(id, layer), title: id, objective: 'do work', success_criteria: 'verified', capability_ids: ['a'] });
const plan = () => ({ schema_version: 5, nodes: [node('code', 1), node('docs', 1), node('test', 2)], edges: [] });
describe('milestone canvas methodology', () => {
  it('preserves parallel layers with a cyclic reflection path', () => {
    const p = connect(plan(), 'test', 'code', 'tests failed');
    expect(groups(validateGraph(p, [cap])).map((g) => g.length)).toEqual([2, 1]);
    expect(connect(p, 'code', 'code').edges).toHaveLength(2);
    expect(() => connect(p, 'code', 'test')).toThrow('回退线');
  });
  it('removes attached edges and compacts empty layers', () => {
    const p = connect(plan(), 'test', 'code');
    const next = removeMilestone(removeMilestone(p, 'code'), 'docs');
    expect(next.edges).toEqual([]); expect(next.nodes[0].layer).toBe(1);
  });
  it('blocks an edit that changes an existing return into a forward edge', () => {
    expect(() => moveMilestone(connect(plan(), 'test', 'code'), 'code', 3)).toThrow('移动');
  });
  it('locates invalid milestones rather than allowing an empty dispatch', () => {
    const p = plan(); p.nodes[1].capability_ids = [];
    try { validateGraph(p, [cap]); throw new Error('expected validation error'); }
    catch (e) { expect(e.nodeId).toBe('docs'); }
  });
  it('separates repeated visits and multiple capabilities on the same milestone', () => {
    const view = { events: [{ event_type: 'layer_started', round: 1, layer: 1, activation: 1 }, { event_type: 'layer_started', round: 2, layer: 1, activation: 3 }], operations: [{ node_id: 'code', activation: 1 }, { node_id: 'code', activation: 3 }, { node_id: 'code', activation: 3 }] };
    expect(visits(view).map((v) => v.operations.length)).toEqual([1, 2]);
  });
});
