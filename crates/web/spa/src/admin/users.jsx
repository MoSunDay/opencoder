import { Alert, Button, Form, Input, Popconfirm, Select, Space, Table, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPatch, apiPost } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { TimeText } from '../ui/timeText.jsx';

export const ROLE_OPTIONS = [{ value: 'editor', label: 'editor · 可编辑' }, { value: 'viewer', label: 'viewer · 只读' }];
export function UsersPanel() {
  const [rows, setRows] = useState([]);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [form] = Form.useForm();
  const load = useCallback(async () => {
    setLoading(true);
    try { setRows((await apiGet('/api/users')).users || []); setError(''); }
    catch (error) { setError(error.message); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => { load(); }, [load]);
  const change = async (work) => {
    setBusy(true);
    try { await work(); await load(); return true; }
    catch (error) { setError(error.message); return false; }
    finally { setBusy(false); }
  };
  return <PageShell page="users">
    <Space orientation="vertical" style={{ width: '100%' }}>
      <Alert type="info" showIcon title="admin 使用启动 Token；editor 可修改平台资源；viewer 只读。" />
      {error && <Alert type="error" showIcon title={error} action={<Button onClick={load}>重试</Button>} />}
      <Table rowKey="name" loading={loading} dataSource={rows} scroll={{ x: 'max-content' }} columns={[
        { title: '用户', dataIndex: 'name' },
        { title: '角色', dataIndex: 'role', render: (role, user) => role === 'admin' ? <Typography.Text>admin · 启动管理员</Typography.Text> : <Select aria-label={`${user.name}角色`} value={role} disabled={busy} options={ROLE_OPTIONS} style={{ width: 170 }} onChange={(role) => change(() => apiPatch(`/api/users/${encodeURIComponent(user.name)}`, { role }))} /> },
        { title: '创建时间', dataIndex: 'created_at', render: (ts) => <TimeText ts={ts} /> },
        { title: '操作', render: (_, user) => user.role === 'admin' ? '启动配置管理' : <Popconfirm title={`删除 ${user.name}？该用户的全部 Token 将失效。`} onConfirm={() => change(() => apiDel(`/api/users/${encodeURIComponent(user.name)}`))}><Button danger disabled={busy}>删除用户</Button></Popconfirm> },
      ]} />
      <Form form={form} layout="inline" initialValues={{ role: 'viewer' }} onFinish={async (body) => { if (await change(() => apiPost('/api/users', body))) form.resetFields(); }}>
        <Form.Item name="name" rules={[{ required: true, message: '请输入用户名' }]}><Input aria-label="用户名" placeholder="用户名" autoComplete="off" /></Form.Item>
        <Form.Item name="role"><Select aria-label="用户角色" options={ROLE_OPTIONS} style={{ width: 170 }} /></Form.Item>
        <Form.Item><Button type="primary" htmlType="submit" loading={busy}>创建用户</Button></Form.Item>
      </Form>
    </Space>
  </PageShell>;
}
