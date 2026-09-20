import { Form, Input, Select } from 'antd';
export function SourceFields({ step, allNames, onChange }) {
  const source = step.kind.source || { type: 'input', pointer: '/items' };
  const update = (next) => onChange({ ...step, kind: { ...step.kind, source: next } });
  return <>
    <Form.Item label="派生来源"><Select aria-label="派生来源" value={source.type}
      options={[{ value: 'input', label: '启动方输入' }, { value: 'step_output', label: '上游结构化输出' }]}
      onChange={(type) => update({ type, pointer: source.pointer, ...(type === 'step_output' ? { step: '' } : {}) })} /></Form.Item>
    {source.type === 'step_output' && <Form.Item label="上游步骤" extra="请在画布上连接该步骤作为依赖。">
      <Select aria-label="上游步骤" value={source.step || undefined} options={(allNames || []).map((name) => ({ value: name, label: name }))}
        onChange={(name) => update({ ...source, step: name })} /></Form.Item>}
    <Form.Item label="数组路径 (JSON pointer)"><Input aria-label="数组路径" value={source.pointer}
      onChange={(e) => update({ ...source, pointer: e.target.value })} /></Form.Item>
    <Form.Item label="实例模板类型"><Select aria-label="实例模板类型" value={step.kind.template?.type || 'agent'}
      options={[{ value: 'agent', label: 'Agent' }, { value: 'wasm', label: 'Wasm' }]}
      onChange={(type) => onChange({ ...step, kind: { ...step.kind, template: type === 'agent' ? { type, prompt: '' } : { type, command: '' } } })} /></Form.Item>
  </>;
}
