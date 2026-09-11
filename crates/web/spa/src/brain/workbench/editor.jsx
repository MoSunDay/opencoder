import { Alert, Button, Collapse, Form, Input, Select, Space, Tabs, Typography } from 'antd';
import { createContext, useContext, useEffect, useId, useRef, useState } from 'react';
import { apiPost } from '../../api.js';
import { PlanCanvas } from './canvas.jsx';
import { KINDS, newStep } from './model.js';
const JsonValidation = createContext(() => {});
function JsonField({ label, value, onChange }) {
  const report = useContext(JsonValidation); const id = useId();
  const serialized = JSON.stringify(value, null, 2); const previous = useRef(serialized);
  const [text, setText] = useState(serialized); const [error, setError] = useState('');
  useEffect(() => { if (previous.current !== serialized) { previous.current = serialized; setText(serialized); } }, [serialized]);
  useEffect(() => { report((errors) => ({ ...errors, [id]: error })); return () => report((errors) => { const next = { ...errors }; delete next[id]; return next; }); }, [error, id, report]);
  return <Form.Item label={label} validateStatus={error ? 'error' : ''} help={error}><Input.TextArea aria-label={label} value={text} autoSize={{ minRows: 2, maxRows: 12 }} spellCheck={false} onChange={(e) => {
    setText(e.target.value);
    try {
      const next = JSON.parse(e.target.value);
      if (label === '本体定义' && (!next || !Array.isArray(next.steps) || !next.inputs || !next.deliverables || next.steps.some((step) => !step || typeof step.id !== 'string' || !step.action || typeof step.action.kind !== 'string'))) throw new Error('完整计划需要 steps 数组、inputs 和 deliverables 对象');
      previous.current = JSON.stringify(next, null, 2); onChange(next); setError('');
    } catch (error) { setError(error.message); }
  }} /></Form.Item>;
}
function StepEditor({ step, plan, capabilities, onChange }) {
  const set = (key, value) => onChange({ ...step, [key]: value });
  const action = (key, value) => set('action', { ...step.action, [key]: value });
  const choices = capabilities.filter((c) => c.kind === step.action.kind && c.target);
  return <Form layout="vertical" size="small"><Form.Item label="步骤名称"><Input value={step.label} onChange={(e) => set('label', e.target.value)} /></Form.Item>
    <Form.Item label="步骤作用"><Input.TextArea value={step.purpose} onChange={(e) => set('purpose', e.target.value)} /></Form.Item>
    <Space.Compact style={{ width: '100%', marginBottom: 12 }}><Select value={step.action.kind} options={KINDS} onChange={(kind) => onChange({ ...step, capability_id: undefined, action: { ...step.action, kind, target: kind === 'agent' ? 'act' : '', definition: undefined, agent_manifests: {} } })} /><Select style={{ flex: 1 }} showSearch value={step.action.target} options={choices.map((c) => ({ value: c.target, label: c.summary || c.target }))} onChange={(target) => { const capability = choices.find((c) => c.target === target); onChange({ ...step, capability_id: capability?.id, action: { ...step.action, target, definition: capability?.definition, agent_manifests: {} } }); }} /></Space.Compact>
    <Form.Item label="动作说明"><Input.TextArea rows={3} value={step.action.prompt} onChange={(e) => action('prompt', e.target.value)} /></Form.Item>
    <Form.Item label="验收标准"><Input.TextArea value={step.acceptance} onChange={(e) => set('acceptance', e.target.value)} /></Form.Item>
    <Form.Item label="前置依赖"><Select mode="multiple" value={step.depends_on} options={plan.steps.filter((s) => s.id !== step.id).map((s) => ({ value: s.id, label: s.label }))} onChange={(value) => set('depends_on', value)} /></Form.Item>
    <Form.Item label="输出方式"><Select value={step.action.output_mode} options={[{ value: 'text', label: '文本' }, { value: 'json', label: '结构化 JSON' }]} onChange={(value) => action('output_mode', value)} /></Form.Item>
    <Form.Item label="输出字段（JSON pointer）"><Input value={step.action.output_pointer || ''} placeholder="空值表示全部输出" onChange={(e) => action('output_pointer', e.target.value)} /></Form.Item>
    <JsonField label="输入端口与来源绑定" value={step.inputs} onChange={(value) => set('inputs', value)} />
    <Typography.Paragraph type="secondary">来源 source：input（用户输入）、output（上游 step）、item（批量项）、literal（常量）。使用 schema 描述类型，path 指定字段。</Typography.Paragraph>
    <JsonField label="输出类型约定" value={step.output} onChange={(value) => set('output', value)} />
    <Collapse items={[{ key: 'advanced', label: '条件、批量与共享资源', children: <>
      <JsonField label="条件 when（null 为无条件）" value={step.when || null} onChange={(value) => set('when', value)} />
      <JsonField label="批量 foreach（null 为单次）" value={step.foreach || null} onChange={(value) => set('foreach', value)} />
      <JsonField label="共享资源（key、mode: read / write）" value={step.resources} onChange={(value) => set('resources', value)} />
    </> }]} />
  </Form>;
}
export function PlanEditor({ version, capabilities, onSaved, onClose }) {
  const [plan, setPlan] = useState(version.plan); const [selected, setSelected] = useState(plan.steps[0]?.id); const [changelog, setChangelog] = useState(''); const [tags, setTags] = useState(version.tags || []); const [confidence, setConfidence] = useState(version.confidence || { level: 'unverified', reason: '', evidence: [] }); const [error, setError] = useState(''); const [busy, setBusy] = useState(false);
  const [jsonErrors, setJsonErrors] = useState({}); const invalid = Object.values(jsonErrors).some(Boolean); const [tab, setTab] = useState('steps');
  const step = plan.steps.find((s) => s.id === selected);
  const validate = async () => { if (invalid) { setError('请先修正 JSON 格式错误'); return false; } setError(''); try { await apiPost('/api/brain/plan-defs/validate', plan); return true; } catch (e) { setError(e.message); return false; } };
  const save = async () => { if (!changelog.trim()) { setError('请填写本次变更说明'); return; } setBusy(true); try { if (!await validate()) return; const result = await apiPost('/api/brain/plan-defs', { ...version, plan, changelog, tags, confidence, author: 'user', created_at: Date.now() }); onSaved(result); } catch (e) { setError(e.message); } finally { setBusy(false); } };
  return <JsonValidation.Provider value={setJsonErrors}><div><Space><Button onClick={onClose}>返回计划库</Button><Typography.Title level={4} style={{ margin: 0 }}>编辑 {version.id} · v{version.version}</Typography.Title><Button disabled={invalid} onClick={validate}>校验计划</Button><Button type="primary" loading={busy} disabled={invalid} onClick={save}>保存新版本</Button></Space>
    <Typography.Paragraph type="secondary">已发布版本不可覆盖。保存生成新版本，运行中的计划保持固定。</Typography.Paragraph>{error && <Alert type="error" showIcon title={error} />}
    <Form layout="vertical"><Form.Item label="计划名称"><Input value={plan.title} onChange={(e) => setPlan({ ...plan, title: e.target.value })} /></Form.Item><Form.Item label="目标"><Input.TextArea value={plan.objective} onChange={(e) => setPlan({ ...plan, objective: e.target.value })} /></Form.Item>
      <Space align="start" wrap><Form.Item label="变更说明（必填）"><Input style={{ width: 320 }} value={changelog} onChange={(e) => setChangelog(e.target.value)} /></Form.Item><Form.Item label="标签"><Select mode="tags" style={{ width: 220 }} value={tags} onChange={setTags} /></Form.Item><Form.Item label="置信程度"><Select style={{ width: 140 }} value={confidence.level} onChange={(level) => setConfidence({ ...confidence, level })} options={['unverified', 'low', 'medium', 'high'].map((value, i) => ({ value, label: ['未验证', '低', '中', '高'][i] }))} /></Form.Item><Form.Item label="置信依据"><Input value={confidence.reason} onChange={(e) => setConfidence({ ...confidence, reason: e.target.value })} /></Form.Item></Space>
    </Form>
    <Tabs activeKey={tab} onChange={(key) => { if (!invalid) setTab(key); else setError('请先修正 JSON 格式错误'); }} items={[{ key: 'steps', label: '步骤与关系', children: <><Space><Button disabled={invalid} onClick={() => { const step = newStep(`step-${crypto.randomUUID().slice(0, 8)}`); setPlan({ ...plan, steps: [...plan.steps, step] }); setSelected(step.id); }}>添加步骤</Button><Button danger disabled={!step || invalid} onClick={() => { setPlan({ ...plan, steps: plan.steps.filter((s) => s.id !== selected) }); setSelected(null); }}>移除所选步骤</Button></Space><div className="brain-editor-workspace"><PlanCanvas plan={plan} mode="ontology" selected={selected} onSelect={(id) => { if (!invalid) setSelected(id); else setError('请先修正 JSON 格式错误'); }} /><div className="brain-inspector">{step ? <StepEditor key={step.id} step={step} plan={plan} capabilities={capabilities} onChange={(next) => setPlan({ ...plan, steps: plan.steps.map((s) => s.id === selected ? next : s) })} /> : '请选择步骤'}</div></div></> },
      { key: 'contracts', label: '目标输入与交付物', children: <Form layout="vertical"><JsonField label="用户输入定义" value={plan.inputs} onChange={(inputs) => setPlan({ ...plan, inputs })} /><JsonField label="交付物与验证条件" value={plan.deliverables} onChange={(deliverables) => setPlan({ ...plan, deliverables })} /></Form> },
      { key: 'source', label: '完整计划 JSON', children: <JsonField label="本体定义" value={plan} onChange={setPlan} /> },
    ]} />
  </div></JsonValidation.Provider>;
}
