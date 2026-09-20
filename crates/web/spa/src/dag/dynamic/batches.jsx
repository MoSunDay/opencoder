import { Form, Input, Typography } from 'antd';
import { inputNodes } from './model.js';

export function DynamicBatches({ spec, batches, onChange }) {
  return <Form layout="vertical">
    {inputNodes(spec).map((s) => <Form.Item key={s.name}
      label={`${s.name} · ${s.kind.template.type === 'agent' ? '文本批次' : 'argv 批次'} (${s.kind.source.pointer || '/'})`}
      extra={s.kind.template.type === 'agent' ? '每项文本追加到对应实例的 how.md；空数组无需执行。' : '每项为一组参数；含空格的字符串仍为一个参数。'}>
      <Input.TextArea aria-label={`${s.name} 批次`} rows={4} value={batches[s.name] ?? '[]'}
        placeholder={s.kind.template.type === 'agent' ? '["审查 api", "审查 web"]' : '[["--target", "api"], ["--title", "hello world"]]'}
        onChange={(e) => onChange({ ...batches, [s.name]: e.target.value })} />
    </Form.Item>)}
    {(spec?.steps || []).some((s) => s.kind?.source?.type === 'step_output') && <Typography.Text type="secondary">上游来源的实例将在依赖成功后自动派生。</Typography.Text>}
  </Form>;
}
