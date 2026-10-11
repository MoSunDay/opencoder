import { EditButton } from '../../../ui/permissions.jsx';
import { Alert, Button, Drawer, Form, Input, InputNumber, Modal, Space, Typography } from 'antd';
import { forwardRef, useImperativeHandle, useMemo, useState } from 'react';
import { apiPost } from '../../../api.js';
import { newId } from '../../../fleet/model.js';
import { LayerCanvas } from '../layered/canvas.jsx';
import { MilestoneCanvas } from '../milestone/canvas.jsx';
import { MilestoneInspector } from '../milestone/inspector.jsx';
import { validateGraph } from '../milestone/model.js';
import { useDraft } from './draft.js';
import { changeDraft } from './changes.js';
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
  const [deleting, setDeleting] = useState(null);
  const [form] = Form.useForm();
  const caps = useMemo(() => capabilities.filter(available), [capabilities]);
  const close = () => { if (!busy && (!draft || persist())) onClose(); };
  useImperativeHandle(ref, () => ({ close }));
  if (!draft) return <Alert type="error" title="无法读取浏览器草稿" description={cacheError} action={<Space><Button onClick={discard}>备份草稿并重新开始</Button><Button onClick={close}>关闭画布</Button></Space>} />;
  const plan = draft.version.plan;
  const update = (action) => setDraft((old) => changeDraft(old, action));
  const attempt = (action) => { try { action(); setError(''); } catch (e) { setError(e.message); if (e.nodeId) setSelected({ type: 'node', id: e.nodeId }); else if (e.layerId) setSelected({ type: 'layer', id: e.layerId }); } };
  const addMilestone = () => attempt(() => { const id = newId('layer'); update({ type: 'add_layer', id }); setSelected({ type: 'layer', id }); });
  const addExecution = (layerId) => attempt(() => { const id = newId('step'); update({ type: 'add_node', id, layerId }); setSelected({ type: 'node', id }); });
  const positions = (positions) => update({ type: 'positions', positions });
  const requestDelete = (items) => { if (!busy) setDeleting((pending) => pending || items); };
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
      <MilestoneCanvas plan={plan} capabilities={caps} selection={selected} onSelect={setSelected} positions={draft.layout} onPositions={positions}
        onResetLayout={() => update({ type: 'reset_layout' })} onDelete={requestDelete} disabled={busy || !!deleting}
        onAddLayer={addMilestone} onAddNode={addExecution} />
      <MilestoneInspector plan={plan} selection={selected} capabilities={caps} disabled={busy || !!deleting}
        onLayerChange={(patch) => update({ type: 'layer', id: selected.id, patch })} onNodeChange={(capability) => update({ type: 'capability', id: selected.id, capability })}
        onMoveNode={(layerId) => attempt(() => update({ type: 'move', id: selected.id, layerId }))}
        onDelete={() => requestDelete([selected])} />
    </div>
    {deleting && <Modal open title="删除选中的里程碑或执行节点？" okText="删除" cancelText="取消" okButtonProps={{ danger: true }}
      onCancel={() => setDeleting(null)} onOk={() => { update({ type: 'remove', items: deleting }); setDeleting(null); setSelected(null); }}>
      删除里程碑时会同时删除其中的执行节点。确认后自动保存到当前草稿。
    </Modal>}
    <Drawer open={submitOpen} onClose={() => !busy && setSubmitOpen(false)} title="计划信息与提交" size={480}>
      {(error || cacheError) && <Alert type="error" title={cacheError || error} />}
      <Form form={form} layout="vertical" disabled={busy} onFinish={save} onValuesChange={(_, values) => update({ type: 'metadata', values })}>
        <Form.Item label="计划名称" name="title" rules={[{ required: true, whitespace: true }]}><Input maxLength={120} /></Form.Item>
        <Form.Item label="整体目标与交付物" name="objective" rules={[{ required: true, whitespace: true }]}><Input.TextArea rows={4} maxLength={4096} /></Form.Item>
        <EngineeringFields />
        <Form.Item label="最多反思轮数（含首轮）" name="max_rounds" rules={[{ required: true }]} extra="正常逐层推进不增加轮数；回退才开启下一轮。耗尽后阻塞，可调整预算后恢复。"><InputNumber min={1} max={32} precision={0} /></Form.Item>
        <Space><Button disabled={busy} onClick={() => setSubmitOpen(false)}>返回画布</Button><EditButton type="primary" htmlType="submit" loading={busy} disabled={!!cacheError}>保存计划版本</EditButton></Space>
      </Form>
    </Drawer>
  </div>;
});
