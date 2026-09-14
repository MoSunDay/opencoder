import { Alert, Button, Collapse, Empty, Form, InputNumber, Select, Space, Tabs, Typography } from 'antd';
import { forwardRef, useImperativeHandle, useState } from 'react';
import { apiPost } from '../../api.js';
import { PlanCanvas } from './canvas.jsx';
import { useDraft } from './editor/draft.js';
import { appendAction, checkDraftPlan, removeAction, repairPlan, submission } from './editor/model.js';
import { ActionEditor } from './editor/action.jsx';
import { JsonField } from './editor/fields.jsx';
import { PublishDialog } from './editor/publish.jsx';
import { PlanContracts } from './editor/contracts.jsx';

export const PlanEditor = forwardRef(function PlanEditor({ version, cacheKey = `oc:brain:editor:${version?.id || 'new'}`, capabilities, onSaved, onClose }, ref) {
  const { draft, setDraft, error: cacheError, persist, retry, clear } = useDraft(cacheKey, version);
  const [error, setError] = useState(''); const [publishing, setPublishing] = useState(false); const [busy, setBusy] = useState(false); const [tab, setTab] = useState('canvas');
  const close = () => { if (!busy && (!draft || persist())) onClose(); };
  useImperativeHandle(ref, () => ({ close }));
  if (!draft) return <Alert type="error" showIcon title="无法读取浏览器草稿" description={cacheError} action={<Space><Button onClick={retry}>重试读取</Button><Button onClick={onClose}>关闭</Button></Space>} />;
  const plan = draft.version.plan;
  const step = plan.steps.find((s) => s.id === draft.selected);
  const invalid = Object.values(draft.raw).some((field) => field.error);
  const setPlan = (next) => setDraft((d) => ({ ...d, version: { ...d.version, plan: next } }));
  const onRaw = (field, value) => setDraft((d) => ({ ...d, raw: { ...d.raw, [field]: value } }));
  const editStep = (next) => setDraft((d) => {
    const old = d.version.plan.steps.find((s) => s.id === next.id);
    const raw = { ...d.raw };
    for (const key of ['inputs', 'output', 'resources', 'when', 'foreach']) if (JSON.stringify(old[key]) !== JSON.stringify(next[key])) delete raw[`${next.id}:${key}`];
    return { ...d, raw, version: { ...d.version, plan: { ...d.version.plan, steps: d.version.plan.steps.map((s) => s.id === next.id ? next : s) } } };
  });
  const json = (key, label, value, change) => <JsonField label={label} field={key} value={value} raw={draft.raw[key]} onRaw={onRaw} onChange={change} />;
  const save = async (metadata) => {
    setBusy(true); setError('');
    try {
      if (!persist()) return;
      const value = submission({ ...draft, metadata }, capabilities);
      await apiPost('/api/brain/plan-defs/validate', value.plan);
      const result = await apiPost('/api/brain/plan-defs', value);
      clear(); onSaved(result);
    } catch (e) { setError(e.message); }
    finally { setBusy(false); }
  };
  return <div className="brain-plan-editor">
    <div className="brain-editor-toolbar"><Space><Button onClick={close} disabled={busy}>关闭画布</Button><Typography.Text type={cacheError ? 'danger' : 'secondary'}>{cacheError ? '草稿缓存失败' : '草稿已缓存在此浏览器'}</Typography.Text></Space><Space>
      <Button disabled={busy || plan.steps.length > 0} onClick={() => setDraft((d) => ({ ...d, version: { ...d.version, plan: repairPlan(capabilities) }, selected: 'fix', positions: {}, viewport: null, raw: {} }))}>修复—复测—发布示例</Button>
      <Button type="primary" disabled={invalid || busy || !!cacheError} onClick={() => { setError(''); setPublishing(true); }}>提交计划</Button>
    </Space></div>
    {cacheError && <Alert type="error" showIcon title={cacheError} action={<Button onClick={persist}>重试缓存</Button>} />}
    {error && !publishing && <Alert type="error" showIcon title={error} />}
    <Tabs activeKey={tab} onChange={setTab} items={[
      { key: 'canvas', label: '实体 · Action · 条件流转', children: <div className="brain-plan-workspace">
        <aside className="brain-entity-library"><Typography.Text strong>实体（能力库对象）</Typography.Text><p>选择实体添加 Action，再定义输入、动作和后续现象。</p>
          {!capabilities.some((c) => c.target) && <Empty description="请先在能力库添加实体" />}
          {capabilities.filter((c) => c.target).map((entity) => <div className="brain-entity-card" key={entity.id}><strong>{entity.summary || entity.target}</strong><small>{entity.kind} · {entity.target}</small><Button size="small" disabled={invalid} onClick={() => { const id = `action-${crypto.randomUUID().slice(0, 8)}`; setDraft((d) => ({ ...d, selected: id, version: { ...d.version, plan: appendAction(d.version.plan, entity, id) } })); }}>添加 Action</Button></div>)}
        </aside>
        <PlanCanvas plan={plan} capabilities={capabilities} mode="ontology" selected={draft.selected} positions={draft.positions} viewport={draft.viewport}
          onPositions={(positions) => setDraft((d) => ({ ...d, positions: { ...d.positions, ...positions } }))}
          onViewport={(viewport) => setDraft((d) => ({ ...d, viewport }))}
          onSelect={(selected) => setDraft((d) => ({ ...d, selected }))}
          onConnect={plan.flow && !invalid ? ({ source, target }) => setPlan({ ...plan, flow: { ...plan.flow, transitions: [...plan.flow.transitions, { from: source, to: target, label: '继续执行' }] } }) : undefined} />
        <aside className="brain-inspector">
          {step ? <><Space style={{ marginBottom: 12 }}><Typography.Text strong>Action 定义</Typography.Text><Button size="small" danger onClick={() => setDraft((d) => ({ ...d, selected: null, raw: Object.fromEntries(Object.entries(d.raw).filter(([key]) => !key.startsWith(`${step.id}:`))), version: { ...d.version, plan: removeAction(d.version.plan, step.id) } }))}>删除 Action</Button></Space>
            <ActionEditor step={step} plan={plan} capabilities={capabilities} raw={draft.raw} onRaw={onRaw} onChange={editStep} onFlow={(flow) => setPlan({ ...plan, flow })} /></> : <><Typography.Paragraph>选中 Action 编辑执行实体、前置输入、要做什么与条件流转。</Typography.Paragraph><Button onClick={() => setTab('contracts')}>定义计划输入与交付物</Button></>}
        </aside>
      </div> },
      { key: 'contracts', label: '计划输入与交付物', children: <Form layout="vertical" className="brain-plan-contracts">
        {plan.flow && <><Form.Item label="起始 Action"><Select aria-label="起始 Action" value={plan.flow.entry} options={plan.steps.map((s) => ({ value: s.id, label: s.label }))} onChange={(entry) => setPlan({ ...plan, flow: { ...plan.flow, entry } })} /></Form.Item><Form.Item label="单个 Action 最多执行轮次"><InputNumber min={1} max={100} value={plan.flow.max_visits_per_action} onChange={(max_visits_per_action) => setPlan({ ...plan, flow: { ...plan.flow, max_visits_per_action } })} /></Form.Item></>}
        <PlanContracts plan={plan} raw={draft.raw} onRaw={onRaw} onChange={setPlan} />
      </Form> },
      { key: 'source', label: '本体定义', children: <Collapse defaultActiveKey={['source']} items={[{ key: 'source', label: '完整契约', children: json('plan:source', '本体定义', plan, (value) => setPlan(checkDraftPlan(value))) }]} /> },
    ]} />
    {publishing && <PublishDialog metadata={draft.metadata} busy={busy} error={error || cacheError} onChange={(metadata) => setDraft((d) => ({ ...d, metadata }))} onSubmit={save} onClose={() => { if (!busy) setPublishing(false); }} />}
  </div>;
});
