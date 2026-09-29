import { describe, expect, it } from 'vitest';
import { addLayer, executionNode, groups, moveNode, removeLayer, removeNode, validateGraph, visits } from './model.js';
const cap = { id: 'a' };
const layer = (id) => ({ layer_id: id, title: id, task: '执行里程碑工作', objective: '目标', success_criteria: '通过证据验收' });
const node = (id, layer_id) => ({ ...executionNode(id, layer_id), title: id, objective: '执行任务', capability_id: 'a' });
const plan = () => ({ schema_version: 7, layers: [layer('code'), layer('test')], nodes: [node('code-a', 'code'), node('code-b', 'code'), node('test-a', 'test')] });
describe('milestone canvas methodology', () => {
  it('keeps parallel nodes in a milestone without configured routes', () => {
    expect(groups(plan()).map((group) => group.length)).toEqual([2, 1]);
    expect(validateGraph(plan(), [cap])).toMatchObject({ layers: [layer('code'), layer('test')] });
  });
  it('removes a layer and its nodes while preserving order', () => {
    const p = addLayer(plan(), 'release');
    const next = removeLayer(p, 'test');
    expect(next.layers.map((item) => item.layer_id)).toEqual(['code', 'release']);
    expect(next.nodes.map((item) => item.node_id)).toEqual(['code-a', 'code-b']);
    expect(next).not.toHaveProperty('transitions');
  });
  it('moves and removes execution nodes', () => {
    const moved = moveNode(plan(), 'code-b', 'test');
    expect(moved.nodes.find((item) => item.node_id === 'code-b').layer_id).toBe('test');
    expect(removeNode(moved, 'code-b').nodes).toHaveLength(2);
  });
  it('locates an invalid execution node', () => {
    const p = plan(); p.nodes[1].capability_id = '';
    try { validateGraph(p, [cap]); throw new Error('expected validation error'); }
    catch (error) { expect(error.nodeId).toBe('code-b'); }
  });
  it('requires each milestone task independently of its goal', () => {
    const p = plan(); p.layers[0].task = '';
    expect(() => validateGraph(p, [cap])).toThrow('要做什么');
  });
  it('separates repeated visits and concurrent operations', () => {
    const view = { events: [{ event_type: 'layer_started', activation: 1 }, { event_type: 'layer_started', activation: 3 }], operations: [{ activation: 1 }, { activation: 3 }, { activation: 3 }] };
    expect(visits(view).map((entry) => entry.operations.length)).toEqual([1, 2]);
  });
});
