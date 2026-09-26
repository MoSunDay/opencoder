import { Alert, Button, Drawer, Form, Input, InputNumber, Space, Typography } from 'antd';
import { forwardRef, useImperativeHandle, useMemo, useState } from 'react';
import { apiPost } from '../../../api.js';
import { newId } from '../../../fleet/model.js';
import { LayerCanvas } from '../layered/canvas.jsx';
import { MilestoneCanvas } from '../milestone/canvas.jsx';
import { MilestoneInspector } from '../milestone/inspector.jsx';
import { addLayer, connect, executionNode, moveNode, removeLayer, removeNode, validateGraph } from '../milestone/model.js';
import { useDraft } from './draft.js';
import { EngineeringFields } from './fields.jsx';
import { available, convertPlan, engineeringInputs, planLayers, validatePlan } from './model.js';

export function PlanPreview({ plan, capabilities }) {
  try {
    if (plan.schema_version >= 5) return <div className="brain-milestone-preview"><MilestoneCanvas plan={plan.schema_version === 7 ? plan : convertPlan(plan)} capabilities={capabilities} /></div>;
    return <LayerCanvas view={{ plan, layers: planLayers(plan), operations: [], run: {} }} />;
  } catch (error) { return <Alert type="error" title={error.message} />; }
}
export const PlanEditor = forwardRef(function PlanEditor({ version, cacheKey, capabilities, onSaved, onClose }, ref) {
  const { draft, setDraft, error: cacheError, persist, clear, discard } = useDraft(cacheKey, version);
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState(null); const [submitOpen, setSubmitOpen] = useState(false);
  const [form] = Form.useForm();
  const close = () => { if (!busy && (!draft || persist())) onClose(); };
  useImperativeHandle(ref, () => ({ close }));
  if (!draft) return <Alert type="error" title="无法读取浏览器草稿" description={cacheError} action={<Space><Button onClick={discard}>备份草稿并重新开始</Button><Button onClick={close}>关闭画布</Button></Space>} />;
  const plan = draft.version.plan; const caps = useMemo(() => capabilities.filter(available), [capabilities]);
  const update = (next) => setDraft((old) => ({ ...old, version: { ...old.version, plan: next } }));
  const attempt = (action) => { try { action(); setError(''); } catch (e) { setError(e.message); if (e.nodeId) setSelected({ type: 'node', id: e.nodeId }); else if (e.layerId) setSelected({ type: 'layer', id: e.layerId }); } };
  const addMilestone = () => attempt(() => { if (plan.layers.length >= 32) throw new Error('最多 32 层'); const id = newId('layer'); update(addLayer(plan, id)); setSelected({ type: 'layer', id }); });
  const addExecution = (layerId) => attempt(() => { if (plan.nodes.length >= 256 || plan.nodes.filter((n) => n.layer_id === layerId).length >= 32) throw new Error('最多 256 个执行节点，每层最多 32 个'); const node = executionNode(newId('step'), layerId); update({ ...plan, nodes: [...plan.nodes, node] }); setSelected({ type: 'node', id: node.node_id }); });
  const change = (key, field, patch) => update({ ...plan, [key]: plan[key].map((item) => item[field] === selected.id ? { ...item, ...patch } : item) });
  const positions = (layout) => setDraft((old) => ({ ...old, layout }));
  const next = () => attempt(() => { validateGraph(plan, caps); form.setFieldsValue({ ...plan, engineering: draft.engineering }); setSubmitOpen(true); });
  const save = async (values) => {
    if (busy) return; setBusy(true); setError('');
    try {
      if (!persist()) return;
      const nextPlan = validatePlan({ ...plan, title: values.title.trim(), objective: values.objective.trim(), max_rounds: values.max_rounds, inputs: engineeringInputs(values.engineering) }, caps);
      await apiPost('/api/brain/plan-defs/validate', nextPlan);
      const result = await apiPost('/api/brain/plan-defs', { ...draft.version, plan: nextPlan });
      await onSaved(result); clear();
    } catch (e) { setError(e.message); } finally { setBusy(false); }
  };
  return <div className="brain-method-editor">
    <div className="brain-method-toolbar"><Space><Button disabled={busy} onClick={close}>关闭画布</Button><Typography.Text strong>配置里程碑与能力</Typography.Text><Typography.Text type="secondary">草稿自动保存 · v{draft.version.version}</Typography.Text></Space><Button type="primary" disabled={busy || !!cacheError} onClick={next}>下一步：计划信息</Button></div>
    {(error || cacheError) && <Alert type="error" showIcon title={cacheError || error} />}
    <div className="brain-method-workspace">
      <MilestoneCanvas plan={plan} capabilities={caps} selection={selected} onSelect={setSelected} positions={draft.layout || {}} onPositions={positions}
        onAddLayer={addMilestone} onAddNode={addExecution} onConnect={(from, to) => attempt(() => { update(connect(plan, from, to)); setSelected({ type: 'transition', id: `${from}:${to}` }); })} />
      <MilestoneInspector plan={plan} selection={selected} capabilities={caps}
        onLayerChange={(patch) => change('layers', 'layer_id', patch)} onNodeChange={(patch) => change('nodes', 'node_id', patch)}
        onTransitionChange={(patch) => update({ ...plan, transitions: plan.transitions.map((item) => `${item.from}:${item.to}` === selected.id ? { ...item, ...patch } : item) })}
        onMoveNode={(layerId) => attempt(() => { update(moveNode(plan, selected.id, layerId)); positions({}); })}
        onDelete={() => { update(selected.type === 'layer' ? removeLayer(plan, selected.id) : selected.type === 'node' ? removeNode(plan, selected.id) : { ...plan, transitions: plan.transitions.filter((item) => `${item.from}:${item.to}` !== selected.id) }); setSelected(null); positions({}); }} />
    </div>
    <Drawer open={submitOpen} onClose={() => !busy && setSubmitOpen(false)} title="计划信息与提交" size={480}>
      {(error || cacheError) && <Alert type="error" title={cacheError || error} />}
      <Form form={form} layout="vertical" disabled={busy} onFinish={save} onValuesChange={(_, values) => setDraft((old) => ({ ...old, engineering: values.engineering || [], version: { ...old.version, plan: { ...old.version.plan, title: values.title, objective: values.objective, max_rounds: values.max_rounds } } }))}>
        <Form.Item label="计划名称" name="title" rules={[{ required: true, whitespace: true }]}><Input maxLength={120} /></Form.Item>
        <Form.Item label="整体目标与交付物" name="objective" rules={[{ required: true, whitespace: true }]}><Input.TextArea rows={4} maxLength={4096} /></Form.Item>
        <EngineeringFields />
        <Form.Item label="最多反思轮数（含首轮）" name="max_rounds" rules={[{ required: true }]} extra="正常逐层推进不增加轮数；回退才开启下一轮。耗尽后阻塞，可调整预算后恢复。"><InputNumber min={1} max={32} precision={0} /></Form.Item>
        <Space><Button disabled={busy} onClick={() => setSubmitOpen(false)}>返回画布</Button><Button type="primary" htmlType="submit" loading={busy} disabled={!!cacheError}>保存计划版本</Button></Space>
      </Form>
    </Drawer>
  </div>;
});
