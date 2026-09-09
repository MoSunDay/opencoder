import { Alert, Form, Input, Select } from 'antd';

export const HARNESS_OPTIONS = [
  { value: 'opencoder', label: 'OpenCoder' },
  { value: 'codex', label: 'Codex' },
];

export function parseEnvs(text = '') {
  const envs = Object.create(null);
  for (const line of text.split('\n')) {
    if (!line.trim()) { continue; }
    const equal = line.indexOf('=');
    if (equal < 1 || line.includes('\0')) {
      throw new Error('环境变量必须为 KEY=VALUE，每行一个');
    }
    envs[line.slice(0, equal)] = line.slice(equal + 1);
  }
  return envs;
}

export function HarnessFields({ initialHarness = 'opencoder', environments = true, inherit = false }) {
  const form = Form.useFormInstance();
  const harness = Form.useWatch('harness', form) || initialHarness;
  return <>
    <Form.Item name="harness" label="Harness" initialValue={initialHarness}>
      <Select options={inherit ? [{ value: 'default', label: '跟随 Agent 配置' }, ...HARNESS_OPTIONS] : HARNESS_OPTIONS} aria-label="agent-harness" />
    </Form.Item>
    {environments && harness === 'codex' && <Alert type="info" showIcon title="Codex 参数已统一管理" description="模型、权限策略和环境变量请在 Agent 配置 → Harness 管理中修改；新任务将固定使用该配置。" style={{ marginBottom: 16 }} />}
    {environments && harness === 'opencoder' && <Form.Item name="envs" preserve={false} label="环境变量" extra="每行一个 KEY=VALUE，仅传给执行进程；启动后固定。"
      rules={[{ validator: (_, value) => {
        try { parseEnvs(value); return Promise.resolve(); } catch (e) { return Promise.reject(e); }
      } }]}>
      <Input.TextArea rows={3} autoComplete="off" spellCheck={false} aria-label="agent-envs" />
    </Form.Item>}
  </>;
}
