import { Alert, Button, Form, Input, Popconfirm, Select, Space, Typography } from 'antd';
import { PlusOutlined } from '@ant-design/icons';
import { useState } from 'react';
import { apiDel, apiPatch, apiPost } from '../api.js';
import { PageShell } from '../shell/pageShell.jsx';
import { TimeText } from '../ui/timeText.jsx';
import { useJsonQuery } from '../ui/requests/query.js';
import { AdminHint, AdminTable, AdminToolbar, RoleTag } from './layout.jsx';
import { AdminFormModal } from './formModal.jsx';
import { matchesSearch, readUsers, ROLE_LABELS, ROLE_OPTIONS } from './model.js';

export function UsersPanel() {
  const { data, loading, error, reload } = useJsonQuery('/api/users', readUsers);
  const [editor, setEditor] = useState(null);
  const [search, setSearch] = useState('');
  const [role, setRole] = useState();
  const [deleting, setDeleting] = useState('');
  const [actionError, setActionError] = useState('');
  const rows = (data || []).filter((user) => (!role || user.role === role) && matchesSearch(search, user.name));
  const save = async (body) => {
    if (editor.name) await apiPatch(`/api/users/${encodeURIComponent(editor.name)}`, { role: body.role });
    else await apiPost('/api/users', body);
    setEditor(null); setActionError(''); await reload();
  };
  const remove = async (name) => {
    setDeleting(name); setActionError('');
    try { await apiDel(`/api/users/${encodeURIComponent(name)}`); await reload(); }
    catch (failure) { setActionError(failure.message); }
    finally { setDeleting(''); }
  };
  return <PageShell page="users">
    <AdminToolbar search={search} onSearch={setSearch} placeholder="搜索用户名" loading={loading} onRefresh={reload}
      filters={<Select aria-label="筛选用户角色" placeholder="全部角色" allowClear value={role} onChange={setRole}
        options={Object.entries(ROLE_LABELS).map(([value, label]) => ({ value, label }))} />}
      actions={<Button type="primary" icon={<PlusOutlined aria-hidden />} onClick={() => setEditor({ role: 'viewer' })}>创建用户</Button>} />
    {(error || actionError) && <Alert className="oc-admin-feedback" type="error" showIcon title={error || actionError}
      action={error && <Button onClick={reload}>重试</Button>} />}
    <AdminTable rowKey="name" loading={loading} dataSource={rows} scroll={{ x: 800 }}
      locale={{ emptyText: error ? '用户列表读取失败，请重试' : search || role ? '没有匹配的用户' : '暂无用户，点击“创建用户”添加' }} columns={[
        { title: '用户', dataIndex: 'name', width: 220, ellipsis: true, sorter: (a, b) => a.name.localeCompare(b.name), render: (name) => <Typography.Text strong>{name}</Typography.Text> },
        { title: '角色', dataIndex: 'role', width: 190, render: (value) => <RoleTag role={value} /> },
        { title: '创建时间', dataIndex: 'created_at', width: 170, sorter: (a, b) => (a.created_at || 0) - (b.created_at || 0), render: (ts) => <TimeText ts={ts} /> },
        { title: '操作', key: 'actions', width: 220, fixed: 'right', render: (_, user) => user.role === 'admin'
          ? <Typography.Text type="secondary">启动配置管理</Typography.Text>
          : <Space size={12}>
            <Button type="link" disabled={!!deleting} onClick={() => setEditor(user)}>编辑权限</Button>
            <Popconfirm title={`删除用户 ${user.name}？`} description="该用户的全部 Token 将失效。" okText="确认删除" cancelText="取消"
              okButtonProps={{ danger: true, loading: deleting === user.name }} onConfirm={() => remove(user.name)}>
              <Button type="link" danger disabled={!!deleting}>删除用户</Button>
            </Popconfirm>
          </Space> },
      ]} />
    <AdminHint>管理员由启动配置管理；可编辑用户能修改平台资源，只读用户仅能查看。角色变更对该用户的全部 Token 生效。</AdminHint>
    {editor && <AdminFormModal title={editor.name ? `编辑权限 · ${editor.name}` : '创建用户'} initialValues={editor}
      submitLabel={editor.name ? '保存权限' : '创建用户'} onSubmit={save} onClose={() => setEditor(null)}>
      <Form.Item name="name" label="用户名" rules={[{ required: true, whitespace: true, message: '请输入用户名' }]}>
        <Input aria-label="用户名" placeholder="输入用户名" autoComplete="off" disabled={editor.name ? true : undefined} autoFocus />
      </Form.Item>
      <Form.Item name="role" label="用户角色" extra="新用户默认为只读，可按需要授予编辑权限。" rules={[{ required: true }]}>
        <Select aria-label="用户角色" options={ROLE_OPTIONS} />
      </Form.Item>
    </AdminFormModal>}
  </PageShell>;
}
