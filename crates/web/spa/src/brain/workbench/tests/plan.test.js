import { describe, expect, it } from 'vitest';
import { newVersion, planLayers, validatePlan, launchBody, convertPlan } from '../scheduler/model.js';
import { createDraft, readDraft } from '../scheduler/draft.js';
const capability = { id: 'agent', kind: 'agent', target: 'act', input_desc: 'task', output_desc: 'result', definition: {}, version: '1' };
const oldPlan = { schema_version: 4, title: '交付', objective: '验证', inputs: {}, nodes: [{ node_id: 'a', title: '实现', capability_id: 'agent' }, { node_id: 'b', title: '验证', capability_id: 'agent' }], edges: [{ from: 'a', to: 'b' }] };
describe('milestone plan versions', () => {
  it('PC 问题入口保留文本和图片引用，拒绝空问题', () => {
    const saved = { id: 'pc-issue', version: 1, plan: { nodes: [{ capability_id: 'pc-issue-impact' }] } };
    const images = [{ id: 'image-x', sha256: 'a'.repeat(64) }];
    const values = { node: 'node-a', problemText: ' 原始问题 ', problemImages: images, engineering: [] };
    expect(launchBody(values, 'brain-x', saved).inputs.problem).toEqual({ text: '原始问题', images });
    expect(() => launchBody({ ...values, problemText: ' ' }, 'brain-x', saved)).toThrow('问题描述');
  });

  it('explicit conversion preserves original version and requires success criteria', () => {
    const before = JSON.stringify(oldPlan); const converted = newVersion({ id: 'p', version: 1, plan: oldPlan });
    expect(converted.version).toBe(2); expect(converted.plan.max_rounds).toBe(5); expect(converted.plan.transitions).toHaveLength(1);
    expect(converted.plan.layers.map((layer) => layer.layer_id)).toEqual(['layer-1', 'layer-2']); expect(JSON.stringify(oldPlan)).toBe(before);
    expect(validatePlan(converted.plan, [capability])).toEqual(converted.plan);
    const ready = { ...converted.plan, layers: converted.plan.layers.map((layer) => ({ ...layer, success_criteria: '通过检查' })) };
    expect(validatePlan(ready, [capability])).toEqual(ready);
  });
  it('retains historical DAG drawing and rejects unsupported conversion', () => {
    expect(planLayers(oldPlan)).toEqual([['a'], ['b']]);
    expect(() => convertPlan({ schema_version: 3 })).toThrow('转换');
  });
  it('creates new drafts with five rounds and explicitly rejects corrupt drafts', () => {
    expect(createDraft().version.plan.schema_version).toBe(7);expect(createDraft().version.plan.max_rounds).toBe(5);
    expect(() => readDraft('k', null, { getItem: () => '{bad' })).toThrow('损坏');
  });
  it('launches a fixed version without mutating its saved inputs', () => {
    const saved = { id: 'p', version: 2, plan: { ...newVersion().plan, inputs: { repo: 'old' } } };
    const request = launchBody({ node: 'n', engineering: [{ key: 'repo', value: '"new"' }] }, 'brain-x', saved);
    expect(request).toEqual({ schema_version: 7, id: 'brain-x', node_id: 'n', plan: { id: 'p', version: 2 }, inputs: { repo: 'new' } });
    expect(saved.plan.inputs.repo).toBe('old');
  });
});
