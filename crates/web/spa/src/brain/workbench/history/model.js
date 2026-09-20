import dagre from '@dagrejs/dagre';
export const KINDS = [{ value: 'agent', label: 'Agent' }, { value: 'dag', label: 'DAG' }, { value: 'todos', label: 'TODO' }, { value: 'team', label: 'Team' }, { value: 'operator', label: 'Operator' }];
export const PHASES = { blocked: '已阻塞', planning: '规划中', running: '执行中', paused: '已暂停', waiting_input: '等待输入', cancelling: '取消中', completed: '已完成', failed: '失败', cancelled: '已取消' };
export const STATES = { inactive: '未激活', waiting: '等待输入 / 汇合', ready: '就绪', queued: '已排队', running: '执行中', verifying: '验证中', succeeded: '执行结束', failed: '失败', skipped: '条件跳过', cancelled: '已取消' };
export const COLORS = { blocked: 'red', planning: 'purple', running: 'blue', ready: 'cyan', queued: 'gold', succeeded: 'green', completed: 'green', failed: 'red', waiting_input: 'orange', paused: 'gold', cancelled: 'default' };
export const terminal = (phase) => ['completed', 'failed', 'cancelled'].includes(phase);
export function dependencies(step) {
  return [...new Set([...(step.depends_on || []), ...Object.values(step.inputs || {}).map((i) => i.binding), step.when?.value, step.foreach?.items].filter(Boolean).map((b) => typeof b === 'string' ? b : b.source === 'output' ? b.step : null).filter(Boolean))];
}
export function statusOf(group) {
  if (group?.total === 0) return 'inactive';
  const counts = group?.counts || {};
  return ['failed', 'running', 'verifying', 'queued', 'ready', 'waiting', 'cancelled', 'succeeded', 'skipped'].find((s) => counts[s]) || (group?.sealed ? 'skipped' : 'waiting');
}
export function graph(plan, groups = []) {
  const nodes = []; const edges = [];
  const add = (id, label, kind, extra = {}) => nodes.push({ id, type: 'brainStep', data: { label, kind, ...extra } });
  const edge = (source, target, label = '') => edges.push({ id: `${source}->${target}:${edges.length}`, source, target, label, type: 'smoothstep', markerEnd: { type: 'arrowclosed' } });
  if (plan?.schema_version !== 2) {
    for (const i of plan?.steps || []) add(i.id, i.label, '历史实例', { step: i });
    for (const e of plan?.flow?.transitions || []) if (e.to) edge(e.from, e.to, e.label);
    if (!plan?.flow) for (const i of plan?.steps || []) for (const source of dependencies(i)) edge(source, i.id);
  } else {
    for (const [id, port] of Object.entries(plan.inputs)) add(`input:${id}`, id, 'input', { description: port.description });
    for (const i of plan.instances) {
      add(i.id, i.description || i.id, `实例 · ${i.action.kind}`, { step: i, group: groups.find((g) => g.id === i.id) });
      i.inputs.forEach((id) => edge(`input:${id}`, i.id));
      i.outputs.forEach((id) => edge(i.id, `output:${id}`));
    }
    for (const [id, output] of Object.entries(plan.outputs)) add(`output:${id}`, id, 'output', { description: output.description });
    for (const r of plan.routes) {
      add(`route:${r.id}`, r.id, '路由', { description: r.description });
      r.outputs.forEach((o) => edge(`output:${o}`, `route:${r.id}`));
      for (const t of r.targets) {
        const bindings = Object.keys(t.bindings);
        if (bindings.length) bindings.forEach((input) => edge(`route:${r.id}`, `input:${input}`, `${t.bindings[input]} → ${input}`));
        else edge(`route:${r.id}`, t.instance, '选中后激活');
      }
      for (const e of r.exits) { add(`exit:${r.id}:${e.id}`, e.description, '结束出口'); edge(`route:${r.id}`, `exit:${r.id}:${e.id}`); }
    }
  }
  const layout = new dagre.graphlib.Graph(); layout.setGraph({ rankdir: 'LR', nodesep: 32, ranksep: 70 }); layout.setDefaultEdgeLabel(() => ({}));
  nodes.forEach((n) => layout.setNode(n.id, { width: 224, height: 128 })); edges.forEach((e) => layout.setEdge(e.source, e.target)); dagre.layout(layout);
  return { nodes: nodes.map((n) => ({ ...n, position: { x: layout.node(n.id).x - 112, y: layout.node(n.id).y - 64 } })), edges };
}
export function newPlan() { return { schema_version: 2, title: '新计划', objective: '', inputs: {}, instances: [], outputs: {}, routes: [], entry: [] }; }
export function engineeringInputs(rows) {
  const inputs = {};
  for (const row of rows || []) {
    const key = String(row?.key || '').trim(); if (!key) continue;
    const raw = String(row?.value ?? '').trim();
    let value; try { value = raw === '' ? '' : JSON.parse(raw); } catch { value = raw; }
    inputs[key] = value;
  }
  return inputs;
}
export function launchBody(values, id) {
  const reference = (raw) => { const [name, version] = raw.split('@'); return { id: name, version: Number(version) }; };
  return { id, mode: values.mode, node_id: values.node, objective: values.objective.trim(), inputs: engineeringInputs(values.engineering), plan: values.mode === 'fixed' ? reference(values.plan) : null, references: values.mode === 'dynamic' ? (values.references || []).map(reference) : [] };
}
