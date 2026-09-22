import { Alert, Button, Collapse, Form, Input, InputNumber, Select, Space, Typography } from 'antd';
import { forwardRef, useImperativeHandle, useState } from 'react';
import { apiPost } from '../../../api.js';
import { newId } from '../../../fleet/model.js';
import { LayerCanvas } from '../layered/canvas.jsx';
import { useDraft } from './draft.js';
import { EngineeringFields } from './fields.jsx';
import { available, capabilityId, engineeringInputs, planLayers, removeNode, validatePlan } from './model.js';

export function PlanPreview({ plan }) {
  try { return <LayerCanvas view={{ plan, layers: planLayers(plan), operations: [], run: {} }} />; }
  catch (error) { return <Alert type="error" title={error.message} />; }
}

export const PlanEditor = forwardRef(function PlanEditor({ version, cacheKey, capabilities, onSaved, onClose }, ref) {
  const { draft, setDraft, error: cacheError, persist, clear, discard } = useDraft(cacheKey, version);
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  const [from, setFrom] = useState(null); const [to, setTo] = useState(null);
  const close = () => { if (!busy && (!draft || persist())) onClose(); };
  useImperativeHandle(ref, () => ({ close }));
  if (!draft) return <Alert type="error" title="无法读取浏览器草稿" description={cacheError} action={<Space><Button onClick={discard}>备份草稿并重新开始</Button><Button onClick={close}>关闭</Button></Space>} />;
  const plan = draft.version.plan;
  const update = (next) => setDraft((old) => ({ ...old, version: { ...old.version, plan: next } }));
  const changeNode = (id, patch) => update({ ...plan, nodes: plan.nodes.map((n) => n.node_id === id ? { ...n, ...patch } : n) });
  const addEdge = () => {
    try { const next = { ...plan, edges: [...plan.edges, { from, to }] }; planLayers(next);
      if (plan.edges.some((e) => e.from === from && e.to === to)) throw new Error('连线已存在');
      update(next); setError('');
    } catch (error) { setError(error.message); }
  };
  const save = async (values) => {
    setBusy(true); setError('');
    try {
      if (!persist()) return;
      const next = validatePlan({ ...plan, title: values.title.trim(), objective: values.objective.trim(), inputs: engineeringInputs(values.engineering) }, capabilities);
      await apiPost('/api/brain/plan-defs/validate', next);
      const result = await apiPost('/api/brain/plan-defs', { ...draft.version, plan: next });
      clear(); await onSaved(result);
    } catch (error) { setError(error.message); } finally { setBusy(false); }
  };
  const options = capabilities.filter(available).map((cap) => ({ value: capabilityId(cap), label: `${cap.kind === 'brain' ? '计划' : cap.kind} · ${cap.summary || cap.target} · ${cap.version}` }));
  const steps = plan.nodes.map((n) => ({ value: n.node_id, label: n.title || n.node_id }));
  return <div className="brain-scheduler-editor">
    <div className="brain-editor-toolbar"><Button onClick={close}>关闭画布</Button><Typography.Text type="secondary">草稿自动保存在此浏览器 · v{draft.version.version}</Typography.Text></div>
    {(error || cacheError) && <Alert type="error" title={cacheError || error} />}
    <PlanPreview plan={plan} />
    <Form layout="vertical" disabled={busy} initialValues={{ ...plan, engineering: draft.engineering }} onValuesChange={(_, values) => setDraft((old) => ({ ...old, engineering: values.engineering || [], version: { ...old.version, plan: { ...old.version.plan, title: values.title, objective: values.objective, max_rounds: values.max_rounds } } }))} onFinish={save}>
      <Form.Item label="计划名称" name="title" rules={[{ required: true, whitespace: true }]}><Input /></Form.Item>
      <Form.Item label="目标和交付物" name="objective" rules={[{ required: true, whitespace: true }]}><Input.TextArea rows={3} /></Form.Item>
      <EngineeringFields />
      {plan.nodes.map((node, index) => <section className="brain-operation-card" key={node.node_id}>
        <Typography.Title level={5}>Step {index + 1}</Typography.Title>
        <Input aria-label={`Step ${index + 1} 描述`} placeholder="一句话描述这个 step" value={node.title} onChange={(e) => changeNode(node.node_id, { title: e.target.value })} />
        <Select aria-label={`Step ${index + 1} 能力`} style={{ width: '100%', margin: '8px 0' }} showSearch optionFilterProp="label" options={options} value={node.capability_id || undefined} placeholder="关联一个能力或可执行计划" onChange={(capability_id) => changeNode(node.node_id, { capability_id })} />
        <Collapse ghost items={[{ key: 'retry', label: '高级设置', children: <Space>最多尝试次数<InputNumber aria-label={`Step ${index + 1} 尝试上限`} min={1} max={5} precision={0} value={node.retry?.max_attempts ?? 2} onChange={(max_attempts) => changeNode(node.node_id, { retry: { max_attempts } })} /></Space> }]} />
        <Button danger onClick={() => update(removeNode(plan, node.node_id))}>删除 Step {index + 1}</Button>
      </section>)}
      <Button onClick={() => update({ ...plan, nodes: [...plan.nodes, { node_id: newId('step'), title: '', capability_id: '', retry: { max_attempts: 2 } }] })}>添加 step</Button>
      <Typography.Title level={5}>连线</Typography.Title>
      <Space wrap><Select aria-label="上游 step" placeholder="上游 step" style={{ width: 240 }} options={steps} value={from} onChange={setFrom} /><Select aria-label="下游 step" placeholder="下游 step" style={{ width: 240 }} options={steps} value={to} onChange={setTo} /><Button disabled={!from || !to} onClick={addEdge}>添加连线</Button></Space>
      {plan.edges.map((edge, i) => <div key={`${edge.from}:${edge.to}`}><Space>{steps.find((n) => n.value === edge.from)?.label} → {steps.find((n) => n.value === edge.to)?.label}<Button onClick={() => update({ ...plan, edges: plan.edges.filter((_, index) => index !== i) })}>删除连线</Button></Space></div>)}
      <Form.Item label="最多调度轮次" name="max_rounds" rules={[{ required: true }]}><InputNumber min={1} max={32} precision={0} /></Form.Item>
      <Button type="primary" htmlType="submit" loading={busy} disabled={!!cacheError}>保存计划</Button>
    </Form>
  </div>;
});
