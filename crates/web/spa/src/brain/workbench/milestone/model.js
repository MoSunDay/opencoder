// Canvas state is pure; positions never define execution order.
import { KIND_LABELS } from '../../../fleet/model.js';

export const SCHEMA = 7;
export const capabilityName = (capability) => {
  const id = capability?.capability_id || capability?.id || '';
  if (capability?.name?.trim()) return capability.name.trim();
  if (capability?.version === 'stored' && capability?.summary?.trim()) return capability.summary.trim();
  if (id.startsWith('pc-issue-')) return id;
  return capability?.target?.trim() || id;
};
export const capabilityLabel = (capability) => `${KIND_LABELS[capability.kind] || capability.kind} · ${capabilityName(capability)}`;
export const layerMilestone = (layer_id) => ({ layer_id, title: '', task: '', objective: '', success_criteria: '' });
export const executionNode = (node_id, layer_id) => ({ node_id, layer_id, title: '', objective: '', capability_id: '' });
export const capabilityTask = (capability) => capability?.summary?.trim() || `输入：${capability?.input_desc || ''}；输出：${capability?.output_desc || ''}`;
export const bindCapability = (node, capability) => ({ ...node, capability_id: capability.capability_id || capability.id,
  title: capabilityName(capability).slice(0, 120), objective: capabilityTask(capability).slice(0, 4096) });
export const groups = (plan) => (plan.layers || []).map((layer) => (plan.nodes || []).filter((node) => node.layer_id === layer.layer_id));

export function addLayer(plan, layer_id) {
  return { ...plan, layers: [...plan.layers, layerMilestone(layer_id)] };
}
export function removeLayer(plan, layer_id) {
  return { ...plan, layers: plan.layers.filter((layer) => layer.layer_id !== layer_id),
    nodes: plan.nodes.filter((node) => node.layer_id !== layer_id) };
}
export const removeNode = (plan, node_id) => ({ ...plan, nodes: plan.nodes.filter((node) => node.node_id !== node_id) });
export function moveNode(plan, node_id, layer_id) {
  if (!plan.layers.some((layer) => layer.layer_id === layer_id)) throw new Error('目标里程碑不存在');
  return { ...plan, nodes: plan.nodes.map((node) => node.node_id === node_id ? { ...node, layer_id } : node) };
}
export function validateGraph(plan, capabilities) {
  if (plan.schema_version !== SCHEMA) throw new Error('请转换为新版里程碑计划');
  if (!Array.isArray(plan.layers) || !plan.layers.length || plan.layers.length > 32) throw new Error('计划需要 1–32 个里程碑');
  if (!Array.isArray(plan.nodes) || plan.nodes.length > 256 || plan.edges?.length) throw new Error('计划结构无效');
  const ids = plan.layers.map((layer) => layer.layer_id);
  if (new Set(ids).size !== ids.length || ids.some((id) => !id || id.length > 64)) throw new Error('里程碑 ID 必须唯一');
  for (const layer of plan.layers) {
    const fail = (message) => { const error = new Error(`${layer.title || '未命名里程碑'}：${message}`); error.layerId = layer.layer_id; throw error; };
    if (!layer.title?.trim() || layer.title.length > 120) fail('名称需要 1–120 字');
    if (!layer.task?.trim() || layer.task.length > 4096) fail('请填写要做什么（不超过 4096 字）');
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
  return plan;
}
export const visits = (view) => (view.events || []).filter((event) => event.event_type === 'layer_started').map((event) => ({
  ...event, operations: (view.operations || []).filter((operation) => operation.activation === event.activation),
}));
