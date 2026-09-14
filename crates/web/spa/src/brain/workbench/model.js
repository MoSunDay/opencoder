import dagre from '@dagrejs/dagre';
export const KINDS = [{ value: 'agent', label: 'Agent' }, { value: 'dag', label: 'DAG' }, { value: 'todos', label: 'TODO' }, { value: 'team', label: 'Team' }];
export const PHASES = { planning: '规划中', running: '执行中', paused: '已暂停', waiting_input: '等待输入', cancelling: '取消中', completed: '已完成', failed: '失败', cancelled: '已取消' };
export const STATES = { waiting: '等待依赖', ready: '就绪', queued: '已排队', running: '执行中', verifying: '验证中', succeeded: '已通过', failed: '失败', skipped: '条件跳过', cancelled: '已取消' };
export const COLORS = { planning: 'purple', running: 'blue', ready: 'cyan', queued: 'gold', succeeded: 'green', completed: 'green', failed: 'red', waiting_input: 'orange', paused: 'gold', cancelled: 'default' };
export const terminal = (phase) => ['completed', 'failed', 'cancelled'].includes(phase);
export function dependencies(step) {
  return [...new Set([...(step.depends_on || []), ...Object.values(step.inputs || {}).map((i) => i.binding), step.when?.value, step.foreach?.items].filter(Boolean).map((b) => typeof b === 'string' ? b : b.source === 'output' ? b.step : null).filter(Boolean))];
}
export function statusOf(group) {
  const counts = group?.counts || {};
  return ['failed', 'running', 'verifying', 'queued', 'ready', 'waiting', 'cancelled', 'succeeded', 'skipped'].find((s) => counts[s]) || (group?.sealed ? 'skipped' : 'waiting');
}
export function graph(plan, groups = [], ontology = false, capabilities = []) {
  const nodes = (plan?.steps || []).map((step) => ({ id: step.id, type: 'brainStep', data: { step, label: step.label, kind: ontology ? 'Action' : step.action.kind, group: groups.find((g) => g.id === step.id), ontology } }));
  const edges = plan?.flow ? plan.flow.transitions.filter((e) => e.to || ontology).map((edge, index) => ({ id: `flow:${index}`, source: edge.from, target: edge.to || 'flow:finish', type: 'smoothstep', label: edge.label, markerEnd: { type: 'arrowclosed' }, style: { stroke: edge.when?.equals === false ? '#d46b08' : '#1677ff', strokeWidth: 2, ...(edge.when ? { strokeDasharray: '6 3' } : {}) } })) : (plan?.steps || []).flatMap((s) => dependencies(s).map((dep) => ({ id: `${dep}->${s.id}`, source: dep, target: s.id, type: 'smoothstep', style: s.when ? { strokeDasharray: '5 4' } : {}, label: s.when ? '条件' : s.foreach ? '展开' : '' })));
  if (ontology) {
    if (plan?.flow) nodes.push({ id: 'flow:finish', type: 'brainStep', data: { kind: '结束', label: '验证交付物', ontology } });
    for (const step of plan?.steps || []) {
      if (!step.capability_id) continue;
      const id = `entity:${step.capability_id}`;
      const entity = capabilities.find((c) => c.id === step.capability_id);
      if (!nodes.some((n) => n.id === id)) nodes.push({ id, type: 'brainStep', data: { kind: '实体', label: entity?.summary || step.action.target, entity: true, description: `${step.action.kind} · ${step.action.target}`, ontology } });
      edges.push({ id: `${id}->${step.id}`, source: id, target: step.id, type: 'smoothstep', label: '执行', style: { stroke: '#8c8c8c', strokeDasharray: '3 4' } });
    }
    for (const [id, port] of Object.entries(plan?.inputs || {})) {
      nodes.push({ id: `input:${id}`, type: 'brainStep', data: { label: id, kind: '输入', port, ontology } });
      for (const step of plan.steps || []) {
        const bindings = [...Object.values(step.inputs || {}).map((i) => i.binding), step.when?.value, step.foreach?.items];
        if (bindings.some((b) => b?.source === 'input' && b.name === id)) edges.push({ id: `input:${id}->${step.id}`, source: `input:${id}`, target: step.id, type: 'smoothstep' });
      }
    }
    for (const [id, port] of Object.entries(plan?.deliverables || {})) {
      nodes.push({ id: `result:${id}`, type: 'brainStep', data: { label: id, kind: '交付物', port, ontology } });
      const source = port.source?.source === 'output' ? port.source.step : port.source?.source === 'input' ? `input:${port.source.name}` : null;
      if (source) edges.push({ id: `${source}->result:${id}`, source, target: `result:${id}`, type: 'smoothstep' });
    }
  }
  const layout = new dagre.graphlib.Graph(); layout.setGraph({ rankdir: 'LR', nodesep: 32, ranksep: 70 }); layout.setDefaultEdgeLabel(() => ({}));
  nodes.forEach((n) => layout.setNode(n.id, { width: 224, height: ontology ? 128 : 104 })); edges.filter((e) => nodes.some((n) => n.id === e.source) && nodes.some((n) => n.id === e.target)).forEach((e) => layout.setEdge(e.source, e.target)); dagre.layout(layout);
  return { nodes: nodes.map((n) => ({ ...n, position: { x: layout.node(n.id).x - 112, y: layout.node(n.id).y - 52 } })), edges: edges.filter((e) => nodes.some((n) => n.id === e.source) && nodes.some((n) => n.id === e.target)) };
}
export function newStep(id = 'step-1') {
  return { id, label: '新步骤', purpose: '说明此步骤的作用', action: { kind: 'agent', target: 'act', prompt: '执行任务并返回结果', output_mode: 'text', max_attempts: 1 }, inputs: {}, output: { type: 'string' }, acceptance: '结果满足任务要求', depends_on: [], resources: [] };
}
export function newPlan() {
  return { schema_version: 1, title: '新计划', objective: '', inputs: {}, steps: [newStep()], deliverables: { result: { description: '最终结果', source: { source: 'output', step: 'step-1' }, schema: { type: 'string' } } }, references: [] };
}
export function launchBody(values, id) {
  const reference = (raw) => { const [name, version] = raw.split('@'); return { id: name, version: Number(version) }; };
  return { id, mode: values.mode, node_id: values.node, objective: values.objective.trim(), inputs: JSON.parse(values.inputs || '{}'), plan: values.mode === 'fixed' ? reference(values.plan) : null, references: values.mode === 'dynamic' ? (values.references || []).map(reference) : [] };
}
