// Canvas state is pure; positions never define execution order.
export const SCHEMA = 7;
export const layerMilestone = (layer_id) => ({ layer_id, title: '', objective: '', success_criteria: '' });
export const executionNode = (node_id, layer_id) => ({ node_id, layer_id, title: '', objective: '', capability_id: '' });
export const groups = (plan) => (plan.layers || []).map((layer) => (plan.nodes || []).filter((node) => node.layer_id === layer.layer_id));

export function addLayer(plan, layer_id) {
  const previous = plan.layers[plan.layers.length - 1];
  return { ...plan, layers: [...plan.layers, layerMilestone(layer_id)],
    transitions: previous ? [...plan.transitions, { from: previous.layer_id, to: layer_id, condition: '本层达标后进入下一里程碑' }] : plan.transitions };
}
export function removeLayer(plan, layer_id) {
  const index = plan.layers.findIndex((layer) => layer.layer_id === layer_id);
  const before = plan.layers[index - 1], after = plan.layers[index + 1];
  const transitions = plan.transitions.filter((edge) => edge.from !== layer_id && edge.to !== layer_id);
  if (before && after) transitions.push({ from: before.layer_id, to: after.layer_id, condition: '本层达标后进入下一里程碑' });
  return { ...plan, layers: plan.layers.filter((layer) => layer.layer_id !== layer_id),
    nodes: plan.nodes.filter((node) => node.layer_id !== layer_id), transitions };
}
export const removeNode = (plan, node_id) => ({ ...plan, nodes: plan.nodes.filter((node) => node.node_id !== node_id) });
export function moveNode(plan, node_id, layer_id) {
  if (!plan.layers.some((layer) => layer.layer_id === layer_id)) throw new Error('目标里程碑不存在');
  return { ...plan, nodes: plan.nodes.map((node) => node.node_id === node_id ? { ...node, layer_id } : node) };
}
export function connect(plan, from, to) {
  const source = plan.layers.findIndex((layer) => layer.layer_id === from);
  const target = plan.layers.findIndex((layer) => layer.layer_id === to);
  if (source < 0 || target < 0 || target > source + 1) throw new Error('前进只能连接下一里程碑，回退可连接本层或已执行层');
  if (plan.transitions.some((edge) => edge.from === from && edge.to === to)) throw new Error('这条里程碑连线已存在');
  return { ...plan, transitions: [...plan.transitions, { from, to, condition: target > source ? '本层达标后进入下一里程碑' : '' }] };
}
export function validateGraph(plan, capabilities) {
  if (plan.schema_version !== SCHEMA) throw new Error('请转换为新版里程碑计划');
  if (!Array.isArray(plan.layers) || !plan.layers.length || plan.layers.length > 32) throw new Error('计划需要 1–32 个里程碑');
  if (!Array.isArray(plan.nodes) || plan.nodes.length > 256 || !Array.isArray(plan.transitions) || plan.edges?.length) throw new Error('计划结构无效');
  const ids = plan.layers.map((layer) => layer.layer_id);
  if (new Set(ids).size !== ids.length || ids.some((id) => !id || id.length > 64)) throw new Error('里程碑 ID 必须唯一');
  for (const layer of plan.layers) {
    const fail = (message) => { const error = new Error(`${layer.title || '未命名里程碑'}：${message}`); error.layerId = layer.layer_id; throw error; };
    if (!layer.title?.trim() || layer.title.length > 120) fail('名称需要 1–120 字');
    if (!layer.objective?.trim() || !layer.success_criteria?.trim()) fail('请填写目标和达成标准');
    if (layer.objective.length > 4096 || layer.success_criteria.length > 4096) fail('目标和达成标准各不超过 4096 字');
    const nodes = plan.nodes.filter((node) => node.layer_id === layer.layer_id);
    if (!nodes.length || nodes.length > 32) fail('每层需要 1–32 个并行执行节点');
  }
  if (new Set(plan.nodes.map((node) => node.node_id)).size !== plan.nodes.length) throw new Error('执行节点 ID 重复');
  for (const node of plan.nodes) {
    const fail = (message) => { const error = new Error(`${node.title || '未命名执行节点'}：${message}`); error.nodeId = node.node_id; throw error; };
    if (!ids.includes(node.layer_id)) fail('所属里程碑不存在');
    if (!node.title?.trim() || node.title.length > 120) fail('名称需要 1–120 字');
    if (!node.objective?.trim() || node.objective.length > 4096) fail('请填写执行任务');
    if (!node.capability_id || !capabilities.some((cap) => (cap.capability_id || cap.id) === node.capability_id)) fail('请选择一个可用能力');
  }
  const pairs = new Set();
  for (const edge of plan.transitions) {
    const from = ids.indexOf(edge.from), to = ids.indexOf(edge.to);
    if (from < 0 || to < 0 || to > from + 1) throw new Error('连线只能前进到下一层，或回到本层及先前层');
    const pair = `${edge.from}\0${edge.to}`;
    if (pairs.has(pair)) throw new Error('里程碑连线重复');
    pairs.add(pair);
    if (!edge.condition?.trim() || edge.condition.length > 1024) throw new Error('请填写连线的扭转条件（不超过 1024 字）');
  }
  for (let i = 1; i < ids.length; i += 1) if (!pairs.has(`${ids[i - 1]}\0${ids[i]}`)) throw new Error('相邻里程碑需要前进连线');
  return plan;
}
export const visits = (view) => (view.events || []).filter((event) => event.event_type === 'layer_started').map((event) => ({
  ...event, operations: (view.operations || []).filter((operation) => operation.activation === event.activation),
}));
