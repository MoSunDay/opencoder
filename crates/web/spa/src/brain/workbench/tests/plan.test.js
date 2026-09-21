import { describe, expect, it } from 'vitest';
import { newVersion, planLayers, validatePlan, removeNode, launchBody } from '../scheduler/model.js';
import { createDraft, readDraft } from '../scheduler/draft.js';
const capability = { id: 'agent', kind: 'agent', target: 'act', input_desc: 'task', output_desc: 'result', definition: {}, version: '1' };
const plan = () => ({ ...newVersion().plan, title: '交付', objective: '交付并验证', nodes: [
  { node_id: 'finish', title: '验证结果', capability_id: 'agent' },
  { node_id: 'first', title: '实现功能', capability_id: 'agent' },
  { node_id: 'parallel', title: '准备测试', capability_id: 'agent' },
], edges: [{ from: 'first', to: 'finish' }, { from: 'parallel', to: 'finish' }] });
describe('step 计划', () => {
  it('按依赖分层，不依赖数组顺序；同层全部完成后才进入下层', () => {
    expect(planLayers(plan())).toEqual([['first', 'parallel'], ['finish']]);
    expect(validatePlan(plan(), [capability])).toEqual(plan());
  });
  it('删除 step 同时移除所有关联连线', () => {
    const next = removeNode(plan(), 'finish');
    expect(next.nodes).toHaveLength(2); expect(next.edges).toEqual([]);
  });
  it('拒绝环、悬空连线、重复节点和重复连线', () => {
    const p = plan();
    expect(() => planLayers({ ...p, edges: [...p.edges, { from: 'finish', to: 'first' }] })).toThrow('循环');
    expect(() => planLayers({ ...p, edges: [{ from: 'missing', to: 'first' }] })).toThrow('不存在');
    expect(() => planLayers({ ...p, nodes: [...p.nodes, p.nodes[0]] })).toThrow('重复');
    expect(() => validatePlan({ ...p, edges: [...p.edges, p.edges[0]] }, [capability])).toThrow('重复');
  });
  it('能力必须可用，计划能力也能绑定；尝试次数有界', () => {
    expect(() => validatePlan(plan(), [])).toThrow('不可用');
    const p = plan(); p.nodes[0].retry = { max_attempts: 6 };
    expect(() => validatePlan(p, [capability])).toThrow('1–5');
    expect(validatePlan(plan(), [{ ...capability, kind: 'brain' }])).toEqual(plan());
  });
  it('保存浏览器新结构，拒绝旧草稿', () => {
    expect(createDraft().version.plan.schema_version).toBe(4);
    expect(() => readDraft('draft', null, { getItem: () => JSON.stringify({ version: { id: 'p', plan: { schema_version: 3 } } }) })).toThrow('格式无效');
  });
  it('执行固定计划版本，输入只影响本次运行', () => {
    const saved = { id: 'plan-x', version: 3, plan: plan() };
    const request = launchBody({ node: 'node-a', engineering: [{ key: 'repo', value: '"workspace"' }] }, 'brain-x', saved);
    expect(request).toEqual({ id: 'brain-x', schema_version: 4, node_id: 'node-a', plan: { id: 'plan-x', version: 3 }, inputs: { repo: 'workspace' } });
    expect(saved.plan.inputs).toEqual({});
  });
});
