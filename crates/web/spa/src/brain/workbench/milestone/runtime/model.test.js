import { expect, it } from 'vitest';
import { layerExecutions, latestExecutions, selectExecution } from './model.js';
const plan = { layers: [{ layer_id: 'code' }, { layer_id: 'test' }], nodes: [
  { node_id: 'developer', layer_id: 'code' }, { node_id: 'tester', layer_id: 'test' },
] };
const operations = [
  { node_id: 'developer', activation: 3, execution_id: 'dev-new', status: 'done' },
  { node_id: 'developer', activation: 1, execution_id: 'dev-old', status: 'error' },
  { node_id: 'tester', activation: 4, execution_id: 'test-new', status: 'done' },
];
it('opens the latest execution of an earlier layer independent of event ordering', () => {
  expect(selectExecution(plan, operations, { type: 'layer', id: 'code' })).toEqual({ layerId: 'code', executionId: 'dev-new' });
  expect(selectExecution(plan, operations, { type: 'node', id: 'developer' })).toEqual({ layerId: 'code', executionId: 'dev-new' });
  expect(layerExecutions(plan, operations, 'code').map((row) => row.execution_id)).toEqual(['dev-new', 'dev-old']);
  expect(latestExecutions(operations)).toHaveLength(2);
});
it('does not open an older result while the new execution is being created', () => {
  const rows = [...operations, { node_id: 'developer', activation: 5, status: 'creating', execution_id: 'reserved' }];
  expect(selectExecution(plan, rows, { type: 'layer', id: 'code' }).executionId).toBeNull();
  expect(selectExecution(plan, [], { type: 'node', id: 'tester' }).executionId).toBeNull();
});
