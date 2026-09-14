import repairExample from '../../../../../../../examples/brain/repair-loop.json';
import { newId } from '../../../fleet/model.js';

export function emptyPlan() {
  return { schema_version: 1, title: '', objective: '', inputs: {}, steps: [], deliverables: {}, references: [], flow: { entry: '', max_visits_per_action: 20, transitions: [] } };
}
export function createDraft(version) {
  return { version: version || { id: newId('plan'), version: 1, plan: emptyPlan() }, selected: version?.plan.steps[0]?.id || null, positions: {}, viewport: null, raw: {}, metadata: { title: version?.plan.title || '', summary: version?.plan.objective || '' } };
}
export function bindEntity(step, entity) {
  return { ...step, capability_id: entity.id, action: { ...step.action, kind: entity.kind, target: entity.target, definition: entity.definition, agent_manifests: {} } };
}
export function appendAction(plan, entity, id) {
  const step = bindEntity({ id, label: '新 Action', purpose: '', action: { prompt: '', output_mode: 'text', max_attempts: 1 }, inputs: {}, output: { type: 'string' }, acceptance: '', depends_on: [], resources: [] }, entity);
  return { ...plan, steps: [...plan.steps, step], flow: plan.flow && { ...plan.flow, entry: plan.flow.entry || id, transitions: [...plan.flow.transitions, { from: id, to: null, label: '完成计划' }] } };
}
export function removeAction(plan, id) {
  return { ...plan, steps: plan.steps.filter((s) => s.id !== id), flow: plan.flow && { ...plan.flow, entry: plan.flow.entry === id ? '' : plan.flow.entry, transitions: plan.flow.transitions.filter((e) => e.from !== id && e.to !== id) } };
}
export function repairPlan(capabilities) {
  const plan = structuredClone(repairExample);
  const entity = capabilities.find((c) => c.kind === 'agent' && c.target === 'act') || capabilities.find((c) => c.kind === 'agent' && c.target);
  if (entity) plan.steps = plan.steps.map((step) => bindEntity(step, entity));
  plan.title = ''; plan.objective = '';
  return plan;
}
export function outputFields(schema, prefix = '') {
  return [{ value: prefix, label: prefix || '完整输出', schema }, ...Object.entries(schema?.properties || {}).flatMap(([name, child]) => outputFields(child, `${prefix}/${name.replaceAll('~', '~0').replaceAll('/', '~1')}`))];
}
export function submission(draft, capabilities) {
  const { title, summary } = draft.metadata;
  if (!title.trim() || !summary.trim()) throw new Error('请填写名称和一句话概述');
  if (Object.values(draft.raw).some((field) => field.error)) throw new Error('请先修正契约格式错误');
  const plan = { ...draft.version.plan, title: title.trim(), objective: summary.trim() };
  if (!plan.steps.length) throw new Error('请从能力库实体添加 Action');
  for (const step of plan.steps) {
    if (!capabilities.some((c) => c.id === step.capability_id && c.kind === step.action.kind && c.target === step.action.target)) throw new Error(`请为 ${step.label} 选择能力库中的执行实体`);
  }
  return { ...draft.version, plan, tags: draft.version.tags || [], confidence: draft.version.confidence || { level: 'unverified', reason: '', evidence: [] }, changelog: summary.trim(), author: 'user', created_at: Date.now() };
}
export function checkDraftPlan(plan) {
  if (!plan || !Array.isArray(plan.steps) || !plan.inputs || !plan.deliverables || plan.steps.some((s) => typeof s?.id !== 'string' || typeof s.label !== 'string' || typeof s.action?.kind !== 'string' || typeof s.action?.prompt !== 'string' || !s.inputs || !s.output?.type || !Array.isArray(s.resources))) throw new Error('计划需要有效的 Action、输入和交付物对象');
  for (const step of plan.steps) { checkPorts(step.inputs, true); checkSchema(step.output); }
  checkPorts(plan.inputs); checkPorts(plan.deliverables);
  if (plan.flow && (!Array.isArray(plan.flow.transitions) || typeof plan.flow.entry !== 'string' || !Number.isInteger(plan.flow.max_visits_per_action) || plan.flow.max_visits_per_action < 1 || plan.flow.max_visits_per_action > 100 || plan.flow.transitions.some((e) => typeof e.from !== 'string' || typeof e.label !== 'string' || !Object.hasOwn(e, 'to') || (e.to !== null && typeof e.to !== 'string') || (e.when && (!e.when.value || typeof e.when.value.source !== 'string' || !Object.hasOwn(e.when, 'equals')))))) throw new Error('流转需要起点、现象和目标');
  return plan;
}
export function checkSchema(schema) {
  if (!schema || !['string', 'number', 'integer', 'boolean', 'object', 'array', 'null'].includes(schema.type)) throw new Error('类型约定无效');
  if (schema.properties) { if (Array.isArray(schema.properties) || typeof schema.properties !== 'object') throw new Error('对象字段需要类型约定'); Object.values(schema.properties).forEach(checkSchema); }
  if (schema.items) checkSchema(schema.items);
  return schema;
}
export function checkPorts(ports, bindings = false) {
  if (!ports || Array.isArray(ports) || typeof ports !== 'object') throw new Error('端口需要对象');
  for (const port of Object.values(ports)) { checkSchema(port?.schema); if (bindings && (!port?.binding || !['input', 'output', 'literal', 'item'].includes(port.binding.source))) throw new Error('输入需要来源绑定'); }
  return ports;
}
