import { expect, it } from 'vitest';
import { changeDraft } from '../scheduler/changes.js';
import { createDraft } from '../scheduler/draft.js';
import { capability, version } from '../milestone/canvas/fixtures.js';

it('successive edits preserve fields, capability bindings, membership and unrelated positions', () => {
  const original = { ...createDraft(version), layout: { coding: { x: 500, y: 90 }, testing: { x: 50, y: 700 } } };
  let draft = changeDraft(original, { type: 'layer', id: 'coding', patch: { task: '最新任务' } });
  draft = changeDraft(draft, { type: 'add_node', layerId: 'coding', id: 'review' });
  draft = changeDraft(draft, { type: 'capability', id: 'review', capability });
  draft = changeDraft(draft, { type: 'move', id: 'review', layerId: 'testing' });
  draft = changeDraft(draft, { type: 'remove', items: [{ type: 'node', id: 'code-a' }] });
  expect(draft.version.plan.layers[0].task).toBe('最新任务');
  expect(draft.version.plan.nodes.find((node) => node.node_id === 'review')).toMatchObject({ layer_id: 'testing', capability_id: 'coding', title: 'Coder' });
  expect(draft.version.plan.nodes.map((node) => node.node_id)).toEqual(['code-b', 'test-a', 'review']);
  expect(draft.layout).toEqual(original.layout);
  expect(original.version.plan.nodes).toHaveLength(3);
});

it('batch deletion cascades once and late position events cannot restore deleted layers', () => {
  let draft = changeDraft(createDraft(version), { type: 'positions', positions: { coding: { x: 10, y: 20 }, testing: { x: 30, y: 40 } } });
  draft = changeDraft(draft, { type: 'remove', items: [{ type: 'layer', id: 'coding' }, { type: 'node', id: 'code-a' }, { type: 'node', id: 'code-b' }] });
  draft = changeDraft(draft, { type: 'positions', positions: { coding: { x: 99, y: 99 } } });
  expect(draft.version.plan.nodes.map((node) => node.node_id)).toEqual(['test-a']);
  expect(draft.version.plan.layers.map((layer) => layer.layer_id)).toEqual(['testing']);
  expect(draft.layout).toEqual({ testing: { x: 30, y: 40 } });
  expect(() => changeDraft(draft, { type: 'add_node', layerId: 'coding', id: 'late' })).toThrow('目标里程碑不存在');
});

it('merges separate position updates and only an explicit reset clears layout', () => {
  let draft = changeDraft(createDraft(version), { type: 'positions', positions: { coding: { x: 10, y: 20 } } });
  draft = changeDraft(draft, { type: 'positions', positions: { testing: { x: 30, y: 40 }, missing: { x: 1, y: 1 } } });
  draft = changeDraft(draft, { type: 'metadata', values: { title: '新版', objective: '交付', max_rounds: 6, engineering: [{ key: 'repo', value: 'src' }] } });
  expect(draft.layout).toEqual({ coding: { x: 10, y: 20 }, testing: { x: 30, y: 40 } });
  expect(draft.version.plan.title).toBe('新版');
  expect(changeDraft(draft, { type: 'reset_layout' }).layout).toEqual({});
});

it('enforces layer and parallel node limits against the current draft', () => {
  let draft = createDraft(version);
  for (let index = 2; index < 32; index++) draft = changeDraft(draft, { type: 'add_node', layerId: 'coding', id: `node-${index}` });
  expect(() => changeDraft(draft, { type: 'add_node', layerId: 'coding', id: 'overflow' })).toThrow('每层最多 32');
  for (let index = 2; index < 32; index++) draft = changeDraft(draft, { type: 'add_layer', id: `layer-${index}` });
  expect(() => changeDraft(draft, { type: 'add_layer', id: 'overflow' })).toThrow('最多 32 层');
});
