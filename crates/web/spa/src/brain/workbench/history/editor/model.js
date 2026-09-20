import repairExample from '../../../../../../../../examples/brain/repair-loop.json';
import { newId } from '../../../../fleet/model.js';
export const documentPort = () => ({ description: '本次任务的具名 Markdown 文档', source: { kind: 'external' }, schema: { type: 'object', properties: { name: { type: 'string' }, markdown: { type: 'string' } }, required: ['name', 'markdown'] }, required: true });
export function emptyPlan() { return { schema_version: 2, title: '', objective: '', inputs: {}, instances: [], outputs: {}, routes: [], entry: [] }; }
export function createDraft(version) {
  return { version: version || { id: newId('plan'), version: 1, created_at: Date.now(), plan: emptyPlan() }, selected: version?.plan.instances?.[0]?.id || null, positions: {}, viewport: null, raw: {}, metadata: { title: version?.plan.title || '', summary: version?.plan.objective || '' } };
}
export function bindEntity(instance, entity) { return { ...instance, capability_id: entity.id, action: { ...instance.action, kind: entity.kind, target: entity.target, definition: entity.definition, agent_manifests: {} } }; }
export function appendAction(plan, entity, id) {
  const output = `${id}-result`;
  const instance = bindEntity({ id, description: entity.summary, action: { prompt: entity.summary, output_mode: 'json', max_attempts: 1 }, inputs: ['document'], outputs: [output], max_visits: 20, resources: [] }, entity);
  return { ...plan, inputs: { document: documentPort(), ...plan.inputs }, instances: [...plan.instances, instance], outputs: { ...plan.outputs, [output]: { description: '执行结果与完成、验证依据' } }, entry: [...plan.entry, id], routes: [...plan.routes, { id: `${id}-route`, description: '结果满足交付条件时结束，否则阻塞并说明原因', outputs: [output], targets: [], exits: [{ id: 'done', description: '交付已验证结果', deliverables: [output], require_completed: true, require_verified: true }] }] };
}
export function removeAction(plan, id) {
  const removed = plan.instances.find((i) => i.id === id)?.outputs || [];
  const instances = plan.instances.filter((i) => i.id !== id);
  const used = new Set(instances.flatMap((i) => i.inputs));
  return { ...plan, instances, entry: plan.entry.filter((i) => i !== id), inputs: Object.fromEntries(Object.entries(plan.inputs).filter(([key]) => used.has(key))), outputs: Object.fromEntries(Object.entries(plan.outputs).filter(([key]) => !removed.includes(key))), routes: plan.routes.map((r) => ({ ...r, outputs: r.outputs.filter((o) => !removed.includes(o)), targets: r.targets.filter((t) => t.instance !== id).map((t) => ({ ...t, bindings: Object.fromEntries(Object.entries(t.bindings).filter(([, o]) => !removed.includes(o))) })), exits: r.exits.map((e) => ({ ...e, deliverables: e.deliverables.filter((o) => !removed.includes(o)) })) })).filter((r) => r.outputs.length) };
}
export function repairPlan(capabilities) {
  const entity = capabilities.find((c) => c.kind === 'agent' && c.target === 'act' && c.summary);
  if (!entity) throw new Error('示例需要已注册且有描述的 act 能力');
  const plan = structuredClone(repairExample); plan.instances = plan.instances.map((i) => bindEntity(i, entity)); return plan;
}
export function removeInput(plan, instanceId, input) {
  const instances = plan.instances.map((i) => i.id === instanceId ? { ...i, inputs: i.inputs.filter((id) => id !== input) } : i);
  return { ...plan, instances, inputs: Object.fromEntries(Object.entries(plan.inputs).filter(([id]) => id !== input || instances.some((i) => i.inputs.includes(id)))), routes: plan.routes.map((r) => ({ ...r, targets: r.targets.map((t) => t.instance === instanceId ? { ...t, bindings: Object.fromEntries(Object.entries(t.bindings).filter(([id]) => id !== input)) } : t) })) };
}
export function removeOutput(plan, instanceId, output) {
  return { ...plan, instances: plan.instances.map((i) => i.id === instanceId ? { ...i, outputs: i.outputs.filter((id) => id !== output) } : i), outputs: Object.fromEntries(Object.entries(plan.outputs).filter(([id]) => id !== output)), routes: plan.routes.map((r) => ({ ...r, outputs: r.outputs.filter((id) => id !== output), targets: r.targets.map((t) => ({ ...t, bindings: Object.fromEntries(Object.entries(t.bindings).filter(([, id]) => id !== output)) })), exits: r.exits.map((e) => ({ ...e, deliverables: e.deliverables.filter((id) => id !== output) })) })) };
}
export function submission(draft, capabilities) {
  const { title, summary } = draft.metadata;
  if (!title.trim() || !summary.trim()) throw new Error('请填写名称和一句话概述');
  if (Object.values(draft.raw).some((f) => f.error)) throw new Error('请先修正契约格式错误');
  const plan = checkDraftPlan({ ...draft.version.plan, title: title.trim(), objective: summary.trim() });
  if (!plan.instances.length) throw new Error('请从能力库添加实例');
  for (const i of plan.instances) if (!capabilities.some((c) => c.id === i.capability_id && c.kind === i.action.kind && c.target === i.action.target && c.summary?.trim())) throw new Error(`实例 ${i.id} 的注册能力不可用或缺少描述`);
  return { ...draft.version, plan, tags: draft.version.tags || [], confidence: draft.version.confidence || { level: 'unverified', reason: '', evidence: [] }, changelog: summary.trim(), author: 'user', created_at: draft.version.created_at || Date.now() };
}
export function checkDraftPlan(plan) {
  if (plan?.schema_version !== 2) throw new Error('旧计划只读，请显式创建 schema_version: 2 计划');
  if (!Array.isArray(plan.instances) || !Array.isArray(plan.routes) || !Array.isArray(plan.entry) || !plan.inputs || !plan.outputs) throw new Error('计划需要 input、实例、output 和路由');
  if (plan.instances.some((i) => !i.id || !i.action || !Array.isArray(i.inputs) || !Array.isArray(i.outputs)) || plan.routes.some((r) => !r.id || !Array.isArray(r.outputs) || !Array.isArray(r.targets) || !Array.isArray(r.exits))) throw new Error('实例或路由结构无效');
  if (Object.values(plan.inputs).some((p) => !p?.schema?.type || !['external', 'routed'].includes(p.source?.kind)) || plan.instances.some((i) => i.inputs.some((id) => !plan.inputs[id]) || i.outputs.some((id) => !plan.outputs[id])) || plan.routes.some((r) => r.targets.some((t) => !t.bindings || !plan.instances.some((i) => i.id === t.instance)) || r.exits.some((e) => !Array.isArray(e.deliverables)))) throw new Error('输入来源、输出引用或路由连接无效');
  return plan;
}
