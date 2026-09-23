import { describe, expect, it } from 'vitest';
import { groups, milestone, moveMilestone, removeMilestone, validateGraph, visits } from './model.js';
const cap = { id: 'a' };
const node = (id, layer) => ({ ...milestone(id, layer), title: id, objective: 'do work', success_criteria: 'verified', capability_ids: ['a'] });
const plan = () => ({ schema_version: 6, nodes: [node('code', 1), node('docs', 1), node('test', 2)], edges: [] });
describe('milestone canvas methodology', () => {
  it('keeps parallel layers without configured return edges', () => {
    expect(groups(validateGraph(plan(), [cap])).map((g) => g.length)).toEqual([2, 1]);
    const legacy = { ...plan(), edges: [{ from: 'test', to: 'code', condition: 'failure' }] };
    expect(() => validateGraph(legacy, [cap])).toThrow('转换旧回退线');
  });
  it('removes attached edges and compacts empty layers', () => {
    const next = removeMilestone(removeMilestone(plan(), 'code'), 'docs');
    expect(next.edges).toEqual([]); expect(next.nodes[0].layer).toBe(1);
  });
  it('moves milestones independently of return paths', () => {
    expect(moveMilestone(plan(), 'code', 3).nodes.find((n) => n.node_id === 'code').layer).toBe(3);
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
