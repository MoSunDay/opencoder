import { Alert, Button, Form, Input, InputNumber, Space, Typography } from 'antd';
import { forwardRef, useImperativeHandle, useRef, useState } from 'react';
import { apiPost } from '../../../api.js';
import { SummaryCanvas } from '../summaryCanvas.jsx';
import { useDraft } from './draft.js';
import { CapabilityPicker, EngineeringFields } from './fields.jsx';
import { engineeringInputs, validatePlan } from './model.js';

export const PlanEditor = forwardRef(function PlanEditor({ version, cacheKey, capabilities, onSaved, onClose }, ref) {
  const { draft, setDraft, error: cacheError, persist, clear, discard } = useDraft(cacheKey, version);
  const [error, setError] = useState(''); const [busy, setBusy] = useState(false); const picker = useRef(null);
  const close = () => { if (!busy && (!draft || persist())) onClose(); };
  useImperativeHandle(ref, () => ({ close }));
  if (!draft) return <Alert type="error" title="无法读取浏览器草稿" description={cacheError} action={<Space><Button onClick={discard}>备份草稿并重新开始</Button><Button onClick={close}>关闭</Button></Space>} />;
  const plan = draft.version.plan;
  const change = (_, values) => setDraft((old) => ({ ...old, engineering: values.engineering || [], version: { ...old.version, plan: { ...old.version.plan, title: values.title, objective: values.objective, capability_ids: values.capability_ids || [], max_rounds: values.max_rounds } } }));
  const save = async (values) => {
    setBusy(true); setError('');
    try {
      if (!persist()) return;
      const next = validatePlan({ ...plan, title: values.title.trim(), objective: values.objective.trim(), inputs: engineeringInputs(values.engineering) }, capabilities);
      const body = { ...draft.version, plan: next };
      await apiPost('/api/brain/plan-defs/validate', next);
      const result = await apiPost('/api/brain/plan-defs', body);
      clear(); await onSaved(result);
    } catch (error) { setError(error.message); } finally { setBusy(false); }
  };
  return <div className="brain-scheduler-editor">
    <div className="brain-editor-toolbar"><Button onClick={close}>关闭画布</Button><Typography.Text type="secondary">草稿自动保存在此浏览器 · v{draft.version.version}</Typography.Text></div>
    {(error || cacheError) && <Alert type="error" title={cacheError || error} action={cacheError && <Button onClick={persist}>重试缓存</Button>} />}
    <SummaryCanvas view={{ objective: plan.objective, capabilities: capabilities.filter((cap) => plan.capability_ids.includes(cap.id)), capability_ids: plan.capability_ids }} onCapabilities={() => picker.current?.scrollIntoView?.({ behavior: 'smooth' })} />
    <Form layout="vertical" disabled={busy} initialValues={{ ...plan, engineering: draft.engineering }} onValuesChange={change} onFinish={save}>
      <Form.Item label="计划名称" name="title" rules={[{ required: true, whitespace: true, message: '请输入计划名称' }]}><Input /></Form.Item>
      <Form.Item label="目标和交付物" name="objective" rules={[{ required: true, whitespace: true, message: '请输入目标和交付物' }]}><Input.TextArea rows={3} /></Form.Item>
      <EngineeringFields />
      <div ref={picker}><Form.Item label="关联能力库" name="capability_ids" rules={[{ type: 'array', min: 1, required: true, message: '请至少选择一项能力' }]}><CapabilityPicker capabilities={capabilities} /></Form.Item></div>
      <Form.Item label="最多调度轮次" name="max_rounds" rules={[{ required: true }]}><InputNumber min={1} max={256} /></Form.Item>
      <Button type="primary" htmlType="submit" loading={busy} disabled={!!cacheError}>保存计划</Button>
    </Form>
  </div>;
});
