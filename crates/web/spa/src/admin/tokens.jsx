import { Alert, Button, DatePicker, Form, Input, Modal, Popconfirm, Select, Space, Table, Tag, Typography } from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPost } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { TimeText } from '../ui/timeText.jsx';

export function TokensPanel() {
  const [rows, setRows] = useState([]);
  const [users, setUsers] = useState([]);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [issued, setIssued] = useState(null);
  const [form] = Form.useForm();
  const load = useCallback(async () => {
    setLoading(true);
    try {
      const [tokens, directory] = await Promise.all([apiGet('/api/tokens'), apiGet('/api/users')]);
      setRows(tokens.tokens || []); setUsers(directory.users || []); setError('');
    } catch (error) { setError(error.message); }
    finally { setLoading(false); }
  }, []);
  useEffect(() => { load(); }, [load]);
  const protectedToken = (row) => users.find((user) => user.name === row.user_name)?.role === 'admin';
  const submit = async ({ expires_at, ...body }) => {
    setBusy(true);
    try {
      const result = await apiPost('/api/tokens', { ...body, expires_at: expires_at?.valueOf() ?? null });
      setIssued(result); form.resetFields(); await load();
    } catch (error) { setError(error.message); }
    finally { setBusy(false); }
  };
  const revoke = async (id) => {
    setBusy(true);
    try { await apiDel(`/api/tokens/${encodeURIComponent(id)}`); await load(); }
    catch (error) { setError(error.message); }
    finally { setBusy(false); }
  };
  return <PageShell page="tokens">
    <Space orientation="vertical" style={{ width: '100%' }}>
      <Alert type="info" title="Token 继承所属用户的当前角色。启动 Token 由启动配置管理。" />
      {error && <Alert type="error" title={error} action={<Button onClick={load}>重试</Button>} />}
      <Table rowKey="id" dataSource={rows} loading={loading} scroll={{ x: 'max-content' }} columns={[
        { title: '名称', dataIndex: 'name' }, { title: '用户', dataIndex: 'user_name' },
        { title: '角色', render: (_, row) => users.find((user) => user.name === row.user_name)?.role || '—' },
        { title: 'Token ID', dataIndex: 'id', render: (id) => <Typography.Text copyable>{id}</Typography.Text> },
        { title: '创建时间', dataIndex: 'created_at', render: (ts) => <TimeText ts={ts} /> },
        { title: '到期时间', dataIndex: 'expires_at', render: (ts) => ts ? <TimeText ts={ts} /> : '不过期' },
        { title: '状态', render: (_, row) => <Tag>{row.revoked_at ? '已撤销' : row.expires_at && row.expires_at <= Date.now() ? '已过期' : '有效'}</Tag> },
        { title: '操作', render: (_, row) => protectedToken(row) ? '启动配置管理' : <Popconfirm title="撤销该 Token？使用它的后续请求将被拒绝。" onConfirm={() => revoke(row.id)}><Button danger disabled={busy || !!row.revoked_at}>撤销</Button></Popconfirm> },
      ]} />
      <Form form={form} layout="vertical" onFinish={submit} style={{ maxWidth: 520 }}>
        <Form.Item name="user_name" label="所属用户" rules={[{ required: true, message: '请选择用户' }]}><Select aria-label="Token 所属用户" options={users.filter((user) => user.role !== 'admin').map((user) => ({ value: user.name, label: `${user.name} · ${user.role}` }))} /></Form.Item>
        <Form.Item name="name" label="Token 名称" rules={[{ required: true, message: '请输入名称' }]}><Input aria-label="Token 名称" maxLength={128} /></Form.Item>
        <Form.Item name="expires_at" label="到期时间（留空不过期）"><DatePicker showTime /></Form.Item>
        <Button htmlType="submit" type="primary" loading={busy}>创建 Token</Button>
      </Form>
    </Space>
    {issued && <Modal title="Token 仅此一次显示" open={!!issued} onCancel={() => setIssued(null)} footer={<Button type="primary" onClick={() => setIssued(null)}>我已保存</Button>} destroyOnHidden>
      <Alert type="warning" title="关闭后无法找回，请保存到安全位置。" />
      <Typography.Paragraph copyable={{ text: issued?.token }} style={{ wordBreak: 'break-all', marginTop: 16 }}>{issued?.token}</Typography.Paragraph>
    </Modal>}
  </PageShell>;
}
