import { Alert, Button, Drawer, Form, Input, Select, Space, Typography } from 'antd';
import { useEffect } from 'react';
import { EditButton, useCanEdit } from '../../ui/permissions.jsx';
import { nodeOptions } from '../model.js';
import { useTeamSubmission } from './useSubmission.js';

export function TeamLaunch({ team, nodes, initialPrompt, onClose, onAccepted, onHistory }) {
  const canEdit = useCanEdit();
  const [form] = Form.useForm();
  const { busy, error, confirming, attempt, registered, submit } = useTeamSubmission(team, onAccepted);
  useEffect(() => {
    if (team) { form.resetFields(); form.setFieldsValue({ node: '', prompt: initialPrompt }); }
  }, [team, form, initialPrompt]);
  useEffect(() => {
    const request = attempt?.request.target === team?.name ? attempt?.request : null;
    if (request) form.setFieldsValue({ node: request.node_id || '', prompt: request.input.prompt });
  }, [attempt, team, form]);
  const run = (values) => { if (canEdit) submit(values); };
  return <Drawer open={!!team} title={`启动 ${team?.name || ''}`} onClose={() => { if (!busy) onClose(); }}
    closable={!busy} maskClosable={!busy} keyboard={!busy} size={560} destroyOnHidden>
    <Alert type="info" showIcon title="整个 Team 会在同一个执行节点内完成，成员不会跨节点运行"
      description="提交成功后会打开执行过程，并保存到执行记录；讨论进展会自动更新。" style={{ marginBottom: 12 }} />
    {error && <Alert type="error" showIcon title="启动请求未确认" description={error} style={{ marginBottom: 12 }} />}
    {confirming && <Alert type="warning" showIcon title="提交较慢，正在自动确认启动结果"
      description={<Space orientation="vertical">
        <span>{registered ? '服务端已保存请求，正在等待节点受理。' : '尚未收到受理结果，正在查询服务端。'}确认成功后会自动打开讨论过程。</span>
        <span>你可以关闭此窗口，稍后在执行记录中查看。重试沿用同一个编号。</span>
        <Space wrap><EditButton onClick={() => run({ node: attempt.request.node_id, prompt: attempt.request.input.prompt })}>重试原请求</EditButton>
          <Button onClick={onHistory}>查看执行记录</Button></Space>
      </Space>} style={{ marginBottom: 12 }} />}
    {attempt && <Typography.Paragraph style={{ overflowWrap: 'anywhere' }}>
      执行编号：<Typography.Text copyable>{attempt.id}</Typography.Text>
    </Typography.Paragraph>}
    <Form form={form} disabled={busy || confirming} onFinish={run} layout="vertical">
      <Form.Item name="node" label="执行节点"><Select options={nodeOptions(nodes, 'team')} /></Form.Item>
      <Form.Item name="prompt" label="任务要求" rules={[{ required: true, whitespace: true, message: '请填写任务要求' }]}><Input.TextArea rows={5} /></Form.Item>
      {!confirming && <EditButton type="primary" htmlType="submit" loading={busy}>启动</EditButton>}
      {busy && <p role="status">正在提交；若响应较慢，15 秒后会自动查询启动结果。</p>}
    </Form>
  </Drawer>;
}
