import { Alert, Button, DatePicker, Form, Input, Modal, Popconfirm, Select, Tag, Typography } from 'antd';
import { PlusOutlined } from '@ant-design/icons';
import { useEffect, useState } from 'react';
import { apiDel, apiPost } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { TimeText } from '../ui/timeText.jsx';
import { useJsonQuery } from '../ui/requests/query.js';
import { AdminHint, AdminTable, AdminToolbar, RoleTag } from './layout.jsx';
import { AdminFormModal } from './formModal.jsx';
import { matchesSearch, readTokens, readUsers, tokenStatus } from './model.js';

export function TokensPanel() {
  const tokens = useJsonQuery('/api/tokens', readTokens);
  const directory = useJsonQuery('/api/users', readUsers);
  const [creating, setCreating] = useState(false);
  const [issued, setIssued] = useState(null);
  const [search, setSearch] = useState('');
  const [status, setStatus] = useState();
  const [revoking, setRevoking] = useState('');
  const [actionError, setActionError] = useState('');
  const [now, setNow] = useState(Date.now);
  useEffect(() => { const timer = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(timer); }, []);
  const users = directory.data || [];
  const roles = new Map(users.map((user) => [user.name, user.role]));
  const availableUsers = users.filter((user) => user.role !== 'admin');
  const reload = () => Promise.all([tokens.reload(), directory.reload()]);
  const error = tokens.error || directory.error;
  const rows = (tokens.data || []).filter((row) => (!status || tokenStatus(row, now).value === status)
    && matchesSearch(search, row.name, row.user_name, row.id));
  const submit = async ({ expires_at, ...body }) => {
    if (expires_at && expires_at.valueOf() <= Date.now()) throw new Error('请选择未来的到期时间');
    const result = await apiPost('/api/tokens', { ...body, expires_at: expires_at?.valueOf() ?? null });
    setCreating(false); setIssued(result); setActionError(''); await tokens.reload();
  };
  const revoke = async (id) => {
    setRevoking(id); setActionError('');
    try { await apiDel(`/api/tokens/${encodeURIComponent(id)}`); await tokens.reload(); }
    catch (failure) { setActionError(failure.message); }
    finally { setRevoking(''); }
  };
  return <PageShell page="tokens">
    <AdminToolbar search={search} onSearch={setSearch} placeholder="搜索名称、用户或 Token ID"
      loading={tokens.loading || directory.loading} onRefresh={reload}
      filters={<Select aria-label="筛选 Token 状态" placeholder="全部状态" allowClear value={status} onChange={setStatus}
        options={[{ value: 'active', label: '有效' }, { value: 'expired', label: '已过期' }, { value: 'revoked', label: '已撤销' }]} />}
      actions={<Button type="primary" icon={<PlusOutlined aria-hidden />} disabled={directory.loading || !!directory.error || !availableUsers.length}
        onClick={() => setCreating(true)}>创建 Token</Button>} />
    {(error || actionError) && <Alert className="oc-admin-feedback" type="error" showIcon title={error || actionError}
      action={error && <Button onClick={reload}>重试</Button>} />}
    {!directory.loading && !directory.error && !availableUsers.length && <Alert className="oc-admin-feedback" type="info" showIcon title="请先在“用户权限”中创建用户，再为其签发 Token。" />}
    <AdminTable rowKey="id" dataSource={rows} loading={tokens.loading} scroll={{ x: 1120 }}
      locale={{ emptyText: tokens.error ? 'Token 列表读取失败，请重试' : search || status ? '没有匹配的 Token' : '暂无 Token，点击“创建 Token”签发' }} columns={[
        { title: '名称 / Token ID', dataIndex: 'name', width: 280, ellipsis: true, render: (name, row) => <>
          <Typography.Text strong>{name}</Typography.Text>
          <Typography.Text className="oc-admin-cell-secondary" copyable={{ text: row.id }} title={row.id}>{row.id}</Typography.Text>
        </> },
        { title: '所属用户', dataIndex: 'user_name', width: 160, ellipsis: true },
        { title: '角色', width: 170, render: (_, row) => <RoleTag role={roles.get(row.user_name)} /> },
        { title: '状态', width: 100, render: (_, row) => { const value = tokenStatus(row, now); return <Tag color={value.color}>{value.label}</Tag>; } },
        { title: '创建时间', dataIndex: 'created_at', width: 140, sorter: (a, b) => a.created_at - b.created_at, render: (ts) => <TimeText ts={ts} /> },
        { title: '到期时间', dataIndex: 'expires_at', width: 140, render: (ts) => ts != null ? <TimeText ts={ts} /> : '不过期' },
        { title: '操作', key: 'actions', width: 130, fixed: 'right', render: (_, row) => roles.get(row.user_name) === 'admin'
          ? <Typography.Text type="secondary">启动配置管理</Typography.Text>
          : <Popconfirm title="撤销该 Token？" description="使用它的后续请求将被拒绝。" okText="确认撤销" cancelText="取消"
            okButtonProps={{ danger: true, loading: revoking === row.id }} onConfirm={() => revoke(row.id)}>
            <Button type="link" danger disabled={!!revoking || row.revoked_at != null || !roles.has(row.user_name) || !!directory.error}>撤销</Button>
          </Popconfirm> },
      ]} />
    <AdminHint>Token 继承所属用户的当前角色；启动 Token 由启动配置管理。Token 明文仅在创建成功后显示一次。</AdminHint>
    {creating && <AdminFormModal title="创建 Token" submitLabel="创建 Token" onSubmit={submit} onClose={() => setCreating(false)}>
      <Form.Item name="user_name" label="所属用户" rules={[{ required: true, message: '请选择用户' }]}>
        <Select aria-label="Token 所属用户" placeholder="选择用户" showSearch={{ optionFilterProp: 'label' }}
          options={availableUsers.map((user) => ({ value: user.name, label: `${user.name} · ${user.role}` }))} />
      </Form.Item>
      <Form.Item name="name" label="Token 名称" rules={[{ required: true, whitespace: true, message: '请输入名称' }]}>
        <Input aria-label="Token 名称" placeholder="例如：本地 CLI、自动化任务" maxLength={128} />
      </Form.Item>
      <Form.Item name="expires_at" label="到期时间" extra="留空表示不过期。">
        <DatePicker aria-label="到期时间" showTime placeholder="选择到期时间" />
      </Form.Item>
    </AdminFormModal>}
    {issued && <Modal title="Token 仅此一次显示" open onCancel={() => setIssued(null)} mask={{ closable: false }}
      footer={<Button type="primary" onClick={() => setIssued(null)}>我已保存</Button>} destroyOnHidden>
      <Alert type="warning" showIcon title="关闭后无法找回，请保存到安全位置。" />
      <Typography.Paragraph copyable={{ text: issued.token }} style={{ wordBreak: 'break-all', marginTop: 16 }}>{issued.token}</Typography.Paragraph>
    </Modal>}
  </PageShell>;
}
