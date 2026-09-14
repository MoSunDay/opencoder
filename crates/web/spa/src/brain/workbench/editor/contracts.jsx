import { Button, Checkbox, Collapse, Form, Input, Select, Space } from 'antd';
import { JsonField, SchemaField } from './fields.jsx';
import { outputFields } from './model.js';

const rename = (object, name, next) => next !== name && Object.hasOwn(object, next) ? object : Object.fromEntries(Object.entries(object).map(([key, value]) => [key === name ? next : key, value]));
const unused = (object, prefix) => { let key = prefix; while (Object.hasOwn(object, key)) key += '_'; return key; };

export function PlanContracts({ plan, raw, onRaw, onChange }) {
  const inputs = plan.inputs; const deliveries = plan.deliverables;
  const input = (key, port) => onChange({ ...plan, inputs: { ...inputs, [key]: port } });
  const delivery = (key, port) => onChange({ ...plan, deliverables: { ...deliveries, [key]: port } });
  return <>
    <h3>计划输入</h3>
    {Object.entries(inputs).map(([name, port], index) => <div className="brain-port" key={index}>
      <Form.Item label="输入名称"><Input aria-label={`计划输入 ${index + 1} 名称`} value={name} onChange={(e) => onChange({ ...plan, inputs: rename(inputs, name, e.target.value) })} /></Form.Item>
      <Form.Item label="输入说明"><Input aria-label={`计划输入 ${index + 1} 说明`} value={port.description} onChange={(e) => input(name, { ...port, description: e.target.value })} /></Form.Item>
      <SchemaField label={`计划输入 ${index + 1} 类型`} schema={port.schema} onChange={(schema) => input(name, { ...port, schema })} />
      <Space><Checkbox checked={port.required !== false} onChange={(e) => input(name, { ...port, required: e.target.checked })}>必需</Checkbox><Button size="small" danger onClick={() => onChange({ ...plan, inputs: Object.fromEntries(Object.entries(inputs).filter(([key]) => key !== name)) })}>删除输入</Button></Space>
    </div>)}
    <Button onClick={() => input(unused(inputs, 'input'), { description: '', schema: { type: 'string' }, required: true })}>添加计划输入</Button>
    <h3>交付物</h3>
    {Object.entries(deliveries).map(([name, port], index) => <div className="brain-port" key={index}>
      <Form.Item label="交付物名称"><Input aria-label={`交付物 ${index + 1} 名称`} value={name} onChange={(e) => onChange({ ...plan, deliverables: rename(deliveries, name, e.target.value) })} /></Form.Item>
      <Form.Item label="交付物说明"><Input value={port.description} onChange={(e) => delivery(name, { ...port, description: e.target.value })} /></Form.Item>
      <Form.Item label="来源 Action"><Select aria-label={`交付物 ${index + 1} 来源`} value={port.source.step} options={plan.steps.map((s) => ({ value: s.id, label: s.label }))} onChange={(step) => delivery(name, { ...port, source: { source: 'output', step, path: '' }, schema: plan.steps.find((s) => s.id === step).output, expected: undefined })} /></Form.Item>
      <Form.Item label="输出字段"><Select aria-label={`交付物 ${index + 1} 字段`} value={port.source.path || ''} options={outputFields(plan.steps.find((s) => s.id === port.source.step)?.output).map(({ value, label }) => ({ value, label }))} onChange={(path) => delivery(name, { ...port, source: { ...port.source, path }, schema: outputFields(plan.steps.find((s) => s.id === port.source.step)?.output).find((p) => p.value === path).schema, expected: undefined })} /></Form.Item>
      {port.schema.type === 'boolean' && <Form.Item label="验证条件"><Select value={port.expected ?? ''} options={[{ value: '', label: '只校验输出类型' }, { value: true, label: '必须为 true' }, { value: false, label: '必须为 false' }]} onChange={(expected) => delivery(name, { ...port, expected: expected === '' ? undefined : expected })} /></Form.Item>}
      <Button size="small" danger onClick={() => onChange({ ...plan, deliverables: Object.fromEntries(Object.entries(deliveries).filter(([key]) => key !== name)) })}>删除交付物</Button>
    </div>)}
    <Button disabled={!plan.steps.length} onClick={() => { const step = plan.steps.at(-1); delivery(unused(deliveries, 'result'), { description: '', source: { source: 'output', step: step.id, path: '' }, schema: step.output }); }}>添加交付物</Button>
    <Collapse style={{ marginTop: 20 }} items={[{ key: 'advanced', label: '高级输入与交付契约', children: <>
      <JsonField label="计划输入定义" field="plan:inputs" value={inputs} raw={raw['plan:inputs']} onRaw={onRaw} onChange={(value) => { if (!value || Array.isArray(value) || Object.values(value).some((p) => !p?.schema?.type)) throw new Error('输入需要含 schema 的对象'); onChange({ ...plan, inputs: value }); }} />
      <JsonField label="交付物定义" field="plan:deliverables" value={deliveries} raw={raw['plan:deliverables']} onRaw={onRaw} onChange={(value) => { if (!value || Array.isArray(value) || Object.values(value).some((p) => !p?.schema?.type || !p?.source)) throw new Error('交付物需要含来源与 schema 的对象'); onChange({ ...plan, deliverables: value }); }} />
    </> }]} />
  </>;
}
