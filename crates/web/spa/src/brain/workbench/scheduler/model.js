import { newId } from '../../../fleet/model.js';
export const capabilityId = (capability) => capability.capability_id || capability.id;
export function available(capability) {
  return ['agent', 'team', 'dag', 'todos', 'operator'].includes(capability.kind)
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
  return version ? { ...version, version: version.version + 1, created_at: Date.now(), changelog: '更新调度计划' }
    : { id: newId('plan'), version: 1, created_at: Date.now(), changelog: '创建调度计划', tags: [], plan: { schema_version: 3, title: '', objective: '', inputs: {}, capability_ids: [], max_rounds: 32 } };
}
export function validatePlan(plan, capabilities) {
  if (!plan.title.trim()) throw new Error('请输入计划名称');
  if (!plan.objective.trim()) throw new Error('请输入目标和交付物');
  if (!plan.capability_ids.length) throw new Error('请至少选择一项能力');
  const missing = plan.capability_ids.filter((id) => !capabilities.some((cap) => capabilityId(cap) === id && available(cap)));
  if (missing.length) throw new Error(`所选能力不可用：${missing.join('、')}`);
  if (!Number.isInteger(plan.max_rounds) || plan.max_rounds < 1 || plan.max_rounds > 256) throw new Error('轮次上限必须为 1–256');
  return plan;
}
export function launchBody(values, id, plan) {
  const base = { schema_version: 3, id, node_id: values.node, inputs: engineeringInputs(values.engineering) };
  return plan ? { ...base, plan: { id: plan.id, version: plan.version } }
    : { ...base, objective: values.objective.trim(), capability_ids: values.capability_ids, max_rounds: values.max_rounds ?? 32 };
}
