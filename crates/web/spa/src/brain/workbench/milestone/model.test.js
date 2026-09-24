import { describe, expect, it } from 'vitest';
import { addLayer, connect, executionNode, groups, moveNode, removeLayer, removeNode, validateGraph, visits } from './model.js';
const cap = { id: 'a' };
const layer = (id) => ({ layer_id: id, title: id, objective: '目标', success_criteria: '通过证据验收' });
const node = (id, layer_id) => ({ ...executionNode(id, layer_id), title: id, objective: '执行任务', capability_id: 'a' });
const plan = () => ({ schema_version: 7, layers: [layer('code'), layer('test')], nodes: [node('code-a', 'code'), node('code-b', 'code'), node('test-a', 'test')], transitions: [{ from: 'code', to: 'test', condition: '编码达标' }] });
describe('milestone canvas methodology', () => {
  it('keeps parallel nodes in a milestone and allows explicit returns', () => {
    const p = connect(plan(), 'test', 'code');
    expect(groups(plan()).map((group) => group.length)).toEqual([2, 1]);
    expect(p.transitions.at(-1)).toEqual({ from: 'test', to: 'code', condition: '' });
    expect(() => validateGraph(p, [cap])).toThrow('扭转条件');
    p.transitions.at(-1).condition = '复测失败';
    expect(validateGraph(p, [cap])).toBe(p);
    expect(() => connect(p, 'code', 'test')).toThrow('已存在');
  });
  it('removes a layer and its nodes while preserving adjacent progression', () => {
    const p = addLayer(plan(), 'release');
    const next = removeLayer(p, 'test');
    expect(next.layers.map((item) => item.layer_id)).toEqual(['code', 'release']);
    expect(next.nodes.map((item) => item.node_id)).toEqual(['code-a', 'code-b']);
    expect(next.transitions).toEqual([{ from: 'code', to: 'release', condition: '本层达标后进入下一里程碑' }]);
  });
  it('moves and removes execution nodes without changing transitions', () => {
    const moved = moveNode(plan(), 'code-b', 'test');
    expect(moved.nodes.find((item) => item.node_id === 'code-b').layer_id).toBe('test');
    expect(removeNode(moved, 'code-b').transitions).toEqual(plan().transitions);
  });
  it('locates an invalid execution node', () => {
    const p = plan(); p.nodes[1].capability_id = '';
    try { validateGraph(p, [cap]); throw new Error('expected validation error'); }
    catch (error) { expect(error.nodeId).toBe('code-b'); }
  });
  it('separates repeated visits and concurrent operations', () => {
    const view = { events: [{ event_type: 'layer_started', activation: 1 }, { event_type: 'layer_started', activation: 3 }], operations: [{ activation: 1 }, { activation: 3 }, { activation: 3 }] };
    expect(visits(view).map((entry) => entry.operations.length)).toEqual([1, 2]);
  });
});
