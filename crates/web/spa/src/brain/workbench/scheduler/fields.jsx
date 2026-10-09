import { EditButton } from '../../../ui/permissions.jsx';
import { Button, Form, Input } from 'antd';

export function EngineeringFields() {
  return <Form.Item label="工程输入（可选）"><Form.List name="engineering">{(fields, { add, remove }) => <>
    {fields.map((field) => <div className="brain-engineering-row" key={field.key}>
      <Form.Item name={[field.name, 'key']}><Input aria-label="工程参数名" placeholder="名称" /></Form.Item>
      <Form.Item name={[field.name, 'value']}><Input.TextArea aria-label="工程参数值" autoSize={{ minRows: 1, maxRows: 5 }} placeholder="文本或 JSON 值" /></Form.Item>
      <EditButton onClick={() => remove(field.name)}>移除</EditButton>
    </div>)}<EditButton type="dashed" onClick={() => add({ key: '', value: '' })}>添加工程参数</EditButton>
  </>}</Form.List></Form.Item>;
}
