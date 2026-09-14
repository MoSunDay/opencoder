import { Button, Checkbox, Form, Input, Select, Space } from 'antd';
import { outputFields } from './model.js';
import { useEffect } from 'react';
export const TYPES = ['string', 'boolean', 'number', 'integer', 'object', 'array'];
export const typeOptions = TYPES.map((value) => ({ value, label: value }));

export function JsonField({ label, field, value, raw, onRaw, onChange }) {
  const serialized = JSON.stringify(value, null, 2);
  useEffect(() => { if (raw && !raw.error) { try { if (JSON.stringify(JSON.parse(raw.text), null, 2) !== serialized) onRaw(field, { text: serialized, error: '' }); } catch { /* invalid text remains in the draft */ } } }, [serialized]);
  const text = raw?.text ?? JSON.stringify(value, null, 2);
  return <Form.Item label={label} validateStatus={raw?.error ? 'error' : ''} help={raw?.error}><Input.TextArea aria-label={label} value={text} autoSize={{ minRows: 2, maxRows: 10 }} spellCheck={false} onChange={(e) => {
    try { const parsed = JSON.parse(e.target.value); onChange(parsed); onRaw(field, { text: e.target.value, error: '' }); }
    catch (error) { onRaw(field, { text: e.target.value, error: error.message }); }
  }} /></Form.Item>;
}
export function SchemaField({ label, schema, onChange }) {
  return <div><Form.Item label={label}><Select aria-label={label} value={schema.type} options={typeOptions} onChange={(type) => onChange(type === 'object' ? { type, properties: {}, required: [] } : type === 'array' ? { type, items: { type: 'string' } } : { type })} /></Form.Item>
    {schema.type === 'object' && <><Space direction="vertical" style={{ width: '100%' }}>{Object.entries(schema.properties || {}).map(([name, child], index) => <Space.Compact key={index} style={{ width: '100%' }}><Input aria-label={`输出字段 ${name}`} value={name} onChange={(e) => { const next = e.target.value; if (next !== name && Object.hasOwn(schema.properties, next)) return; const properties = Object.fromEntries(Object.entries(schema.properties).map(([key, value]) => [key === name ? next : key, value])); onChange({ ...schema, properties, required: (schema.required || []).map((key) => key === name ? next : key) }); }} /><Select aria-label={`${name} 类型`} value={child.type} options={typeOptions.filter((o) => o.value !== 'array' && o.value !== 'object')} onChange={(type) => onChange({ ...schema, properties: { ...schema.properties, [name]: { type } } })} /><Button aria-label={`删除输出 ${name}`} onClick={() => onChange({ ...schema, properties: Object.fromEntries(Object.entries(schema.properties).filter(([key]) => key !== name)), required: (schema.required || []).filter((key) => key !== name) })}>删除</Button></Space.Compact>)}</Space>
      <Button size="small" onClick={() => { let name = 'field'; while (schema.properties?.[name]) name += '_'; onChange({ ...schema, properties: { ...schema.properties, [name]: { type: 'string' } }, required: [...(schema.required || []), name] }); }}>添加输出字段</Button></>}
  </div>;
}
export function InputsField({ step, plan, onChange }) {
  const ports = step.inputs || {};
  const set = (name, port) => onChange({ ...ports, [name]: port });
  return <div className="brain-input-ports"><strong>前置输入</strong>{Object.entries(ports).map(([name, port], index) => <div className="brain-port" key={index}>
    <Space.Compact><Input aria-label={`输入名称 ${name}`} value={name} onChange={(e) => { const next = e.target.value; if (next !== name && Object.hasOwn(ports, next)) return; onChange(Object.fromEntries(Object.entries(ports).map(([key, value]) => [key === name ? next : key, value]))); }} /><Button aria-label={`删除输入 ${name}`} onClick={() => onChange(Object.fromEntries(Object.entries(ports).filter(([key]) => key !== name)))}>删除</Button></Space.Compact>
    <Select aria-label={`${name} 来源类型`} value={port.binding.source} options={[{ value: 'input', label: '计划输入' }, { value: 'output', label: 'Action 输出（最近一轮）' }, { value: 'literal', label: '固定值' }]} onChange={(source) => set(name, { ...port, binding: source === 'literal' ? { source, value: '' } : source === 'input' ? { source, name: '' } : { source, step: '', path: '' } })} />
    {port.binding.source === 'input' && <Select aria-label={`${name} 计划输入`} value={port.binding.name} options={Object.keys(plan.inputs).map((key) => ({ value: key, label: key }))} onChange={(key) => set(name, { ...port, schema: plan.inputs[key].schema, binding: { source: 'input', name: key } })} />}
    {port.binding.source === 'output' && <><Select aria-label={`${name} 来源 Action`} value={port.binding.step} options={plan.steps.map((s) => ({ value: s.id, label: s.label }))} onChange={(id) => set(name, { ...port, schema: plan.steps.find((s) => s.id === id).output, binding: { source: 'output', step: id, path: '' } })} /><Select aria-label={`${name} 输出字段`} value={port.binding.path || ''} options={outputFields(plan.steps.find((s) => s.id === port.binding.step)?.output).map(({ value, label }) => ({ value, label }))} onChange={(path) => set(name, { ...port, schema: outputFields(plan.steps.find((s) => s.id === port.binding.step)?.output).find((p) => p.value === path).schema, binding: { ...port.binding, path } })} /></>}
    {port.binding.source === 'literal' && <Input aria-label={`${name} 固定值`} value={typeof port.binding.value === 'string' ? port.binding.value : JSON.stringify(port.binding.value)} onChange={(e) => set(name, { ...port, schema: { type: 'string' }, binding: { source: 'literal', value: e.target.value } })} />}
    <Checkbox checked={port.required !== false} onChange={(e) => set(name, { ...port, required: e.target.checked })}>必需（首轮可无反馈时取消）</Checkbox>
  </div>)}<Button size="small" onClick={() => { let name = 'input'; while (ports[name]) name += '_'; set(name, { schema: { type: 'string' }, binding: { source: 'literal', value: '' }, required: true }); }}>添加前置输入</Button></div>;
}
