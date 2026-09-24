import { SCHEMA, validateGraph } from '../milestone/model.js';
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
    : { id: newId('plan'), version: 1, created_at: Date.now(), changelog: '创建计划', tags: [], plan: { schema_version: SCHEMA, title: '', objective: '', inputs: {}, nodes: [], layers: [], transitions: [], max_rounds: 5 } };
}
export function planLayers(plan) {
  if (plan.schema_version >= 5) return Array.from({ length: Math.max(0, ...plan.nodes.map((n) => n.layer)) }, (_, i) => plan.nodes.filter((n) => n.layer === i + 1).map((n) => n.node_id));
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
  if (plan.plan?.nodes?.some((node) => node.capability_id?.startsWith('pc-issue-'))) {
    const text = String(values.problemText || '').trim();
    if (!text) throw new Error('请输入问题描述');
    inputs.problem = { text, images: values.problemImages || [] };
    inputs.settings = { ...plan.plan.inputs?.settings, ...values.settings };
  }
  return { schema_version: SCHEMA, id, node_id: values.node, inputs, plan: { id: plan.id, version: plan.version } };
}
export function removeNode(plan, id) {
  return { ...plan, nodes: plan.nodes.filter((n) => n.node_id !== id), edges: plan.edges.filter((e) => e.from !== id && e.to !== id) };
}

export function convertPlan(plan) {
  if (plan.schema_version === SCHEMA) return structuredClone(plan);
  if (![4, 5, 6].includes(plan.schema_version)) throw new Error('不支持此计划的转换');
  const levels = planLayers(plan);
  const layers = levels.map((ids, index) => ({ layer_id: `layer-${index + 1}`, title: plan.nodes.find((n) => n.node_id === ids[0])?.title || `里程碑 ${index + 1}`,
    objective: plan.nodes.filter((n) => ids.includes(n.node_id)).map((n) => n.objective || n.title).join('；'),
    success_criteria: plan.nodes.filter((n) => ids.includes(n.node_id)).map((n) => n.success_criteria).filter(Boolean).join('；') || '本层执行项全部达标' }));
  const nodes = plan.nodes.flatMap((node) => (node.capability_ids?.length ? node.capability_ids : [node.capability_id]).map((capability_id, index) => ({
    node_id: index ? `${node.node_id}-${index + 1}` : node.node_id, layer_id: layers[levels.findIndex((group) => group.includes(node.node_id))].layer_id,
    title: index ? `${node.title} ${index + 1}` : node.title, objective: node.objective || node.title, capability_id,
  })));
  const transitions = layers.slice(1).map((layer, index) => ({ from: layers[index].layer_id, to: layer.layer_id, condition: '本层达标后进入下一里程碑' }));
  if (plan.schema_version === 5) for (const edge of plan.edges || []) {
    const from = levels.findIndex((group) => group.includes(edge.from)); const to = levels.findIndex((group) => group.includes(edge.to));
    if (from >= 0 && to >= 0 && to <= from && !transitions.some((item) => item.from === layers[from].layer_id && item.to === layers[to].layer_id)) transitions.push({ from: layers[from].layer_id, to: layers[to].layer_id, condition: edge.condition || '需要整改' });
  }
  return { schema_version: SCHEMA, title: plan.title, objective: plan.objective, inputs: structuredClone(plan.inputs || {}), todo: plan.todo, nodes, layers, transitions, max_rounds: plan.max_rounds || 5 };
}
