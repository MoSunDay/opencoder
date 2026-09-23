import { groups, validateGraph } from '../milestone/model.js';
import { newId } from '../../../fleet/model.js';
export const capabilityId = (capability) => capability.capability_id || capability.id;
export function available(capability) {
  return ['agent', 'team', 'dag', 'todos', 'operator', 'brain'].includes(capability.kind)
    && !!capability.target?.trim() && !!capability.input_desc?.trim()
    && !!capability.output_desc?.trim() && !!capability.version
    && capability.definition !== null && typeof capability.definition === 'object' && !Array.isArray(capability.definition);
}
export function engineeringInputs(rows = []) {
  const inputs = Object.create(null);
  for (const row of rows) {
    const key = String(row.key || '').trim(); const raw = String(row.value ?? '').trim();
    if (!key && !raw) continue;
    if (!key) throw new Error('工程参数名不能为空');
    if (Object.hasOwn(inputs, key)) throw new Error(`工程参数重复：${key}`);
    try { inputs[key] = raw ? JSON.parse(raw) : ''; } catch { inputs[key] = raw; }
  }
  return inputs;
}
export const inputRows = (inputs = {}) => Object.entries(inputs).map(([key, value]) => ({ key, value: JSON.stringify(value) }));
export function newVersion(version) {
  return version ? { ...version, plan: convertPlan(version.plan), version: version.version + 1, created_at: Date.now(), changelog: version.plan.schema_version === 4 ? '转换为里程碑方法论' : '更新计划' }
    : { id: newId('plan'), version: 1, created_at: Date.now(), changelog: '创建计划', tags: [], plan: { schema_version: 5, title: '', objective: '', inputs: {}, nodes: [], edges: [], max_rounds: 5 } };
}
export function planLayers(plan) {
  if (plan.schema_version === 5) return groups(plan).map((g) => g.map((n) => n.node_id));
  const remaining = new Set(plan.nodes.map((n) => n.node_id)); const done = new Set(); const layers = [];
  if (remaining.size !== plan.nodes.length) throw new Error('step ID 重复');
  for (const edge of plan.edges) if (!remaining.has(edge.from) || !remaining.has(edge.to)) throw new Error('连线引用不存在的 step');
  while (remaining.size) {
    const ready = [...remaining].filter((id) => plan.edges.every((e) => e.to !== id || done.has(e.from)));
    if (!ready.length) throw new Error('step 连线不能构成循环');
    if (ready.length > 32) throw new Error('每层最多 32 个 step');
    layers.push(ready); ready.forEach((id) => { remaining.delete(id); done.add(id); });
  }
  if (layers.length > 32) throw new Error('最多 32 层');
  return layers;
}
export function validatePlan(plan, capabilities) {
  validateGraph(plan, capabilities.filter(available));
  if (!plan.title.trim()) throw new Error('请输入计划名称');
  if (!plan.objective.trim()) throw new Error('请输入目标和交付物');
  if (!Number.isInteger(plan.max_rounds) || plan.max_rounds < 1 || plan.max_rounds > 32) throw new Error('轮次上限必须为 1–32');
  return plan;
}
export function launchBody(values, id, plan) {
  if (!plan) throw new Error('请先选择可执行计划');
  const inputs = engineeringInputs(values.engineering);
  if (plan.plan?.nodes?.some((node) => node.capability_ids?.some((id) => id.startsWith('pc-issue-')))) {
    const text = String(values.problemText || '').trim();
    if (!text) throw new Error('请输入问题描述');
    inputs.problem = { text, images: values.problemImages || [] };
    inputs.settings = { ...plan.plan.inputs?.settings, ...values.settings };
  }
  return { schema_version: 5, id, node_id: values.node, inputs, plan: { id: plan.id, version: plan.version } };
}
export function removeNode(plan, id) {
  return { ...plan, nodes: plan.nodes.filter((n) => n.node_id !== id), edges: plan.edges.filter((e) => e.from !== id && e.to !== id) };
}

export function convertPlan(plan) {
  if (plan.schema_version === 5) return structuredClone(plan);
  if (plan.schema_version !== 4) throw new Error('不支持此计划的转换');
  const levels = planLayers(plan);
  return { ...plan, schema_version: 5, max_rounds: 5, edges: [], nodes: plan.nodes.map((n) => ({
    node_id: n.node_id, title: n.title, layer: levels.findIndex((g) => g.includes(n.node_id)) + 1,
    objective: n.title, success_criteria: '', capability_ids: [n.capability_id],
  })) };
}
