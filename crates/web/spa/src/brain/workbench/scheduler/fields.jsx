import { Alert, Button, Checkbox, Collapse, Empty, Form, Input, Select, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { KIND_LABELS } from '../../../fleet/model.js';
import { available, capabilityId } from './model.js';

export function CapabilityPicker({ capabilities = [], value = [], onChange, readOnly = false }) {
  const [query, setQuery] = useState(''); const [kind, setKind] = useState('all');
  const selected = new Set(value);
  const rows = capabilities.filter((cap) => (kind === 'all' || cap.kind === kind)
    && `${cap.summary || ''} ${cap.target} ${capabilityId(cap)}`.toLowerCase().includes(query.toLowerCase()));
  const missing = value.filter((id) => !capabilities.some((cap) => capabilityId(cap) === id));
  return <section className="brain-capability-picker" aria-label="计划能力范围">
    <Space wrap><Input aria-label="筛选计划能力" placeholder="搜索能力名称或 ID" value={query} onChange={(event) => setQuery(event.target.value)} />
      <Select aria-label="能力类型" value={kind} onChange={setKind} options={['all', 'agent', 'team', 'dag', 'todos', 'operator'].map((k) => ({ value: k, label: k === 'all' ? '全部类型' : KIND_LABELS[k] || k }))} />
      <Typography.Text>已选 {value.length} 项</Typography.Text></Space>
    {!!missing.length && <Alert type="error" title={`所选能力已不存在：${missing.join('、')}`} action={!readOnly && <Button onClick={() => onChange?.(value.filter((id) => !missing.includes(id)))}>移除失效能力</Button>} />}
    <div className="brain-capability-options">{rows.map((cap) => {
      const id = capabilityId(cap); const valid = available(cap);
      return <div className="brain-capability-option" key={id}>
        <Checkbox checked={selected.has(id)} disabled={readOnly || (!valid && !selected.has(id))} onChange={(event) => onChange?.(event.target.checked ? [...value, id] : value.filter((item) => item !== id))}>
          <Tag>{KIND_LABELS[cap.kind] || cap.kind}</Tag><strong>{cap.summary || cap.target || id}</strong><small>{id}</small>
        </Checkbox>
        {!valid && <Typography.Text type="danger">{cap.unavailable_reason || "能力描述或定义不完整"}</Typography.Text>}
        <Collapse ghost size="small" items={[{ key: 'description', label: '能力说明', children: <><p>目标：{cap.target}</p><p>输入：{cap.input_desc}</p><p>输出：{cap.output_desc}</p></> }]} />
      </div>;
    })}{!rows.length && <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="没有匹配的能力" />}</div>
  </section>;
}

export function EngineeringFields() {
  return <Form.Item label="工程输入（可选）"><Form.List name="engineering">{(fields, { add, remove }) => <>
    {fields.map((field) => <div className="brain-engineering-row" key={field.key}>
      <Form.Item name={[field.name, 'key']}><Input aria-label="工程参数名" placeholder="名称" /></Form.Item>
      <Form.Item name={[field.name, 'value']}><Input.TextArea aria-label="工程参数值" autoSize={{ minRows: 1, maxRows: 5 }} placeholder="文本或 JSON 值" /></Form.Item>
      <Button onClick={() => remove(field.name)}>移除</Button>
    </div>)}<Button type="dashed" onClick={() => add({ key: '', value: '' })}>添加工程参数</Button>
  </>}</Form.List></Form.Item>;
}
