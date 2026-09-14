import { Collapse, Form, Input, Select } from 'antd';
import { bindEntity, checkPorts, checkSchema } from './model.js';
import { InputsField, JsonField, SchemaField } from './fields.jsx';
import { Transitions } from './transitions.jsx';

export function ActionEditor({ step, plan, capabilities, raw, onRaw, onChange, onFlow }) {
  const set = (key, value) => onChange({ ...step, [key]: value });
  const json = (key, label, value, change) => <JsonField label={label} field={`${step.id}:${key}`} value={value} raw={raw[`${step.id}:${key}`]} onRaw={onRaw} onChange={change} />;
  return <Form layout="vertical" size="small">
    <Form.Item label="执行实体（能力库对象）"><Select aria-label="执行实体" showSearch optionFilterProp="label" value={step.capability_id} options={capabilities.filter((c) => c.target).map((c) => ({ value: c.id, label: `${c.summary || c.target} · ${c.kind}` }))} onChange={(id) => onChange(bindEntity(step, capabilities.find((c) => c.id === id)))} /></Form.Item>
    <Form.Item label="Action 名称"><Input aria-label="Action 名称" value={step.label} onChange={(e) => set('label', e.target.value)} /></Form.Item>
    <Form.Item label="要做什么"><Input.TextArea aria-label="要做什么" rows={3} value={step.action.prompt} onChange={(e) => onChange({ ...step, purpose: e.target.value, action: { ...step.action, prompt: e.target.value } })} /></Form.Item>
    <InputsField step={step} plan={plan} onChange={(inputs) => set('inputs', inputs)} />
    <SchemaField label="输出类型" schema={step.output} onChange={(output) => onChange({ ...step, output, action: { ...step.action, output_mode: output.type === 'string' ? 'text' : 'json' } })} />
    <Form.Item label="验收标准"><Input.TextArea aria-label="验收标准" value={step.acceptance} onChange={(e) => set('acceptance', e.target.value)} /></Form.Item>
    {plan.flow && <Transitions step={step} plan={plan} onChange={onFlow} />}
    <Collapse items={[{ key: 'contracts', label: '高级契约', children: <>
      {json('inputs', '输入端口与来源绑定', step.inputs, (value) => set('inputs', checkPorts(value, true)))}
      {json('output', '输出类型约定', step.output, (value) => set('output', checkSchema(value)))}
      {json('resources', '共享资源', step.resources, (value) => { if (!Array.isArray(value)) throw new Error('共享资源需要数组'); set('resources', value); })}
      {!plan.flow && <>{json('when', '条件 when', step.when || null, (value) => set('when', value))}{json('foreach', '批量 foreach', step.foreach || null, (value) => set('foreach', value))}</>}
    </> }]} />
  </Form>;
}
