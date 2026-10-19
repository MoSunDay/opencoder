import { expect, it } from 'vitest';
import { assessmentFor, roundRows } from './model.js';
it('返工只使目标层及后续层的历史通过结论失效', () => {
  const layers = [{ layer_id: 'a' }, { layer_id: 'b' }];
  const view = { plan: { layers }, events: [
    { activation: 1, assessments: { a: { met: true, reason: '通过' } } },
    { activation: 2, assessments: { b: { met: true, reason: '通过' } } },
    { activation: 3, event_type: 'layer_started', layer: 2 },
  ] };
  expect(assessmentFor(view, layers[0], 1).label).toBe('已达标');
  expect(assessmentFor(view, layers[1], 2).label).toBe('历史达标，返工后已失效');
  expect(assessmentFor(view, layers[1], 3).label).toBe('尚未评估');
  view.events.push({ activation: 3, assessments: { b: { met: false, reason: '结果未达标' } } });
  expect(assessmentFor(view, layers[1], 3)).toMatchObject({ label: '未达标', reason: '结果未达标' });
});
it('轮次倒序汇总全部并行操作，保留未创建执行的派发', () => {
  const rows = roundRows([{ round: 1, layer: 1, operations: ['a', 'b'] }, { round: 2, layer: 1, operations: [] }, { round: 2, layer: 2, operations: ['c'] }]);
  expect(rows.map((row) => row.round)).toEqual([2, 1]);
  expect(rows[0]).toMatchObject({ layerCount: 2, operations: ['c'] });
  expect(rows[0].visits).toHaveLength(2);
});
