// Pure methodology edits; positions never determine runtime ordering.
export const SCHEMA = 5;
export function groups(plan) {
  const total = Math.max(0, ...plan.nodes.map((n) => n.layer));
  return Array.from({ length: total }, (_, i) => plan.nodes.filter((n) => n.layer === i + 1));
}
export function milestone(id, layer) {
  return { node_id: id, layer, title: '', objective: '', success_criteria: '', capability_ids: [] };
}
export function removeMilestone(plan, id) {
  const nodes = plan.nodes.filter((n) => n.node_id !== id);
  const occupied = [...new Set(nodes.map((n) => n.layer))].sort((a, b) => a - b);
  return { ...plan, nodes: nodes.map((n) => ({ ...n, layer: occupied.indexOf(n.layer) + 1 })), edges: plan.edges.filter((e) => e.from !== id && e.to !== id) };
}
export function connect(plan, from, to, condition = '结果未达标，需要整改') {
  const source = plan.nodes.find((n) => n.node_id === from); const target = plan.nodes.find((n) => n.node_id === to);
  if (!source || !target) throw new Error('连线节点不存在');
  if (target.layer > source.layer) throw new Error('正常推进按层顺序进行；回退线只能返回当前或之前的层');
  if (plan.edges.some((e) => e.from === from && e.to === to)) throw new Error('回退线已存在');
  return { ...plan, edges: [...plan.edges, { from, to, condition }] };
}
export function moveMilestone(plan, id, layer) {
  if (!Number.isInteger(layer) || layer < 1 || layer > groups(plan).length + 1) throw new Error('无效层级');
  const nodes = plan.nodes.map((n) => n.node_id === id ? { ...n, layer } : n);
  const occupied = [...new Set(nodes.map((n) => n.layer))].sort((a, b) => a - b);
  const next = { ...plan, nodes: nodes.map((n) => ({ ...n, layer: occupied.indexOf(n.layer) + 1 })) };
  for (const edge of next.edges) if (next.nodes.find((n) => n.node_id === edge.to).layer > next.nodes.find((n) => n.node_id === edge.from).layer) throw new Error('移动会使已有回退线指向后续层，请先调整连线');
  return next;
}
export function validateGraph(plan, capabilities) {
  if (plan.schema_version !== SCHEMA) throw new Error('请显式转换为里程碑计划');
  if (!plan.nodes.length || plan.nodes.length > 256) throw new Error('计划需要 1–256 个里程碑');
  if (plan.nodes.some((n) => !Number.isInteger(n.layer) || n.layer < 1 || n.layer > 32)) throw new Error('里程碑层级必须为 1–32 的整数');
  const levels = groups(plan);
  if (levels.length > 32 || levels.some((g) => !g.length || g.length > 32)) throw new Error('层级必须连续，每层最多 32 个里程碑，共最多 32 层');
  if (new Set(plan.nodes.map((n) => n.node_id)).size !== plan.nodes.length) throw new Error('里程碑 ID 重复');
  for (const node of plan.nodes) {
    const fail = (message) => { const error = new Error(`${node.title || '未命名里程碑'}：${message}`); error.nodeId = node.node_id; throw error; };
    if (!node.title.trim() || node.title.length > 120) fail('名称需要 1–120 字');
    if (!node.objective.trim() || !node.success_criteria.trim()) fail('请填写目标和达成标准');
    if (node.objective.length > 4096 || node.success_criteria.length > 4096) fail('目标和达成标准各不超过 4096 字');
    if (!node.capability_ids.length || node.capability_ids.length > 32) fail('请选择 1–32 个能力');
    if (new Set(node.capability_ids).size !== node.capability_ids.length) fail('能力重复');
    for (const id of node.capability_ids) if (!capabilities.some((c) => (c.capability_id || c.id) === id)) fail(`能力不可用：${id}`);
  }
  const checked = { ...plan, edges: [] };
  for (const edge of plan.edges) {
    if (!edge.condition.trim() || edge.condition.length > 1024) throw new Error('回退线需要 1–1024 字的适用情形');
    checked.edges = connect(checked, edge.from, edge.to, edge.condition).edges;
  }
  return plan;
}
export function visits(view) {
  return (view.events || []).filter((e) => e.event_type === 'layer_started').map((e) => ({
    ...e, operations: (view.operations || []).filter((op) => op.activation === e.activation),
  }));
}
