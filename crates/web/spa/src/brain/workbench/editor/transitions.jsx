import { Button, Form, Input, InputNumber, Select, Space } from 'antd';
import { outputFields } from './model.js';

export function Transitions({ step, plan, onChange }) {
  const edges = plan.flow.transitions;
  const fields = outputFields(step.output).filter((p) => ['string', 'boolean', 'number', 'integer'].includes(p.schema?.type));
  const set = (index, edge) => onChange({ ...plan.flow, transitions: edges.map((e, i) => i === index ? edge : e) });
  const condition = (path) => { const field = fields.find((p) => p.value === path); return { value: { source: 'output', step: step.id, path }, equals: field?.schema.type === 'boolean' ? true : ['integer', 'number'].includes(field?.schema.type) ? 0 : '' }; };
  return <div className="brain-transitions"><strong>现象与后续 Action</strong>{edges.map((edge, index) => edge.from !== step.id ? null : <div className="brain-port" key={index}>
    <Form.Item label="现象／流转说明"><Input aria-label={`流转 ${index + 1} 现象`} value={edge.label} onChange={(e) => set(index, { ...edge, label: e.target.value })} /></Form.Item>
    <Select aria-label={`流转 ${index + 1} 条件`} value={edge.when ? 'condition' : 'default'} options={[{ value: 'default', label: '其他条件未命中时（默认）' }, { value: 'condition', label: '输出符合条件时', disabled: !fields.length }]} onChange={(mode) => set(index, { ...edge, when: mode === 'default' ? undefined : condition(fields[0].value) })} />
    {edge.when && <Space.Compact style={{ width: '100%' }}><Select aria-label={`流转 ${index + 1} 输出字段`} value={edge.when.value.path || ''} options={fields.map(({ value, label }) => ({ value, label }))} onChange={(path) => set(index, { ...edge, when: condition(path) })} />
      {typeof edge.when.equals === 'boolean' ? <Select aria-label={`流转 ${index + 1} 等于`} value={edge.when.equals} options={[{ value: true, label: '是 / true' }, { value: false, label: '否 / false' }]} onChange={(equals) => set(index, { ...edge, when: { ...edge.when, equals } })} /> : typeof edge.when.equals === 'number' ? <InputNumber aria-label={`流转 ${index + 1} 等于`} value={edge.when.equals} onChange={(equals) => set(index, { ...edge, when: { ...edge.when, equals } })} /> : <Input aria-label={`流转 ${index + 1} 等于`} value={edge.when.equals} onChange={(e) => set(index, { ...edge, when: { ...edge.when, equals: e.target.value } })} />}
    </Space.Compact>}
    <Form.Item label="继续／回退到"><Select aria-label={`流转 ${index + 1} 目标`} value={edge.to || ''} options={[{ value: '', label: '结束计划并验证交付物' }, ...plan.steps.map((s) => ({ value: s.id, label: s.label }))]} onChange={(to) => set(index, { ...edge, to: to || null })} /></Form.Item>
    <Button size="small" danger onClick={() => onChange({ ...plan.flow, transitions: edges.filter((_, i) => i !== index) })}>删除流转</Button>
  </div>)}<Button size="small" onClick={() => onChange({ ...plan.flow, transitions: [...edges, { from: step.id, to: null, label: '', when: fields.length ? condition(fields[0].value) : undefined }] })}>添加条件流转</Button></div>;
}
