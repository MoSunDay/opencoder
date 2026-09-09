// usersDrawer.jsx — 后台管理抽屉：平台用户表（GET /api/users）+ 创建用户
// （POST /api/users，令牌只在响应里出现一次 ⇒ Modal 独占展示 + 复制）+
// 吊销（DELETE /api/users/:name）。仅 admin 身份可调这些端点；服务端
// 403/409/400（删自己 / 最后一个管理员等）经 onNotice(err) 透出。

import {
  Alert, Button, Drawer, Form, Input, Modal, Popconfirm, Select, Space, Table, Typography, message,
} from 'antd';
import { useCallback, useEffect, useState } from 'react';
import { apiDel, apiGet, apiPost } from '../api.js';
import { err } from '../notice.js';
import { TimeText } from '../ui/timeText.jsx';

const { Paragraph, Text } = Typography;

const ROLE_OPTIONS = [
  { value: 'admin', label: '管理员' },
  { value: 'root', label: 'Root' },
  { value: 'user', label: '普通用户' },
];
const ROLE_LABELS = Object.fromEntries(ROLE_OPTIONS.map((o) => [o.value, o.label]));

/// 复制令牌：优先 navigator.clipboard；不可用（非安全上下文 / jsdom）时
/// 退回隐藏 textarea 全选，浏览器复制命令仍可用。
function copyText(value) {
  return new Promise((resolve) => {
    const done = (ok) => resolve(ok);
    if (navigator.clipboard?.writeText) {
      navigator.clipboard.writeText(value).then(() => done(true), () => done(fallbackCopy(value)));
      return;
    }
    done(fallbackCopy(value));
  });
}

function fallbackCopy(value) {
  const area = document.createElement('textarea');
  area.value = value;
  area.style.position = 'fixed';
  area.style.opacity = '0';
  document.body.appendChild(area);
  area.focus();
  area.select();
  let ok = false;
  try { ok = document.execCommand('copy'); } catch { ok = false; }
  document.body.removeChild(area);
  return ok;
}

/// 新令牌的一次性展示：Modal + Alert 提醒「不会再显示」+ 复制按钮。
function IssuedTokenModal({ issued, onClose }) {
  if (!issued) {
    return null;
  }
  return (
    <Modal
      title={`用户 ${issued.name} 的访问令牌`}
      open
      onCancel={onClose}
      footer={<Button type="primary" onClick={onClose}>我已保存</Button>}
      destroyOnHidden
    >
      <Alert
        type="warning"
        showIcon
        title="令牌仅此一次显示，关闭后无法找回"
        style={{ marginBottom: 16 }}
      />
      <Paragraph copyable={{ text: issued.token }} style={{ marginBottom: 16 }}>
        <Text code style={{ wordBreak: 'break-all' }}>{issued.token}</Text>
      </Paragraph>
      <Button onClick={() => copyText(issued.token).then((ok) => (ok ? message.success('已复制') : message.error('复制失败，请手动选择令牌复制')))}>复制</Button>
    </Modal>
  );
}

export function UsersDrawer({ open, onClose, onNotice }) {
  const [rows, setRows] = useState([]);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);
  const [issued, setIssued] = useState(null); // {name, token} — 只显示一次
  const [form] = Form.useForm();

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const j = await apiGet('/api/users');
      setRows(j.users || []);
    } catch (e) {
      onNotice(err('加载用户失败: ' + (e && e.message)));
    } finally {
      setLoading(false);
    }
  }, [onNotice]);

  useEffect(() => {
    if (open) {
      load();
    }
  }, [open, load]);

  const submit = async (values) => {
    setSaving(true);
    try {
      const j = await apiPost('/api/users', { name: values.name, role: values.role || 'user' });
      message.success('已创建用户');
      form.resetFields();
      setIssued({ name: (j.user && j.user.name) || values.name, token: (j && j.token) || '' });
      await load();
    } catch (e) {
      onNotice(err('创建用户失败: ' + (e && e.message)));
    } finally {
      setSaving(false);
    }
  };

  const revoke = async (name) => {
    try {
      await apiDel(`/api/users/${encodeURIComponent(name)}`);
      message.success('已吊销');
      await load();
    } catch (e) {
      onNotice(err(e && e.message));
    }
  };

  const columns = [
    { title: '用户', dataIndex: 'name', render: (v) => <Text strong>{v}</Text> },
    { title: '角色', dataIndex: 'role', render: (v) => ROLE_LABELS[v] || v || '—' },
    { title: '创建时间', dataIndex: 'created_at', render: (v) => <TimeText ts={v} /> },
    {
      title: '操作',
      render: (_, r) => (
        <Popconfirm title={`吊销用户 ${r.name}？其令牌立即失效。`} okText="确认吊销" onConfirm={() => revoke(r.name)}>
          <Button size="small" danger>吊销</Button>
        </Popconfirm>
      ),
    },
  ];

  return (
    <Drawer title="后台管理 · 平台用户" open={open} onClose={onClose} size="720px">
      <Space orientation="vertical" size={16} style={{ width: '100%' }}>
        <Table
          rowKey="name"
          size="small"
          columns={columns}
          dataSource={rows}
          loading={loading}
          pagination={false}
          locale={{ emptyText: '暂无用户' }}
        />
        <Form form={form} layout="inline" onFinish={submit} initialValues={{ role: 'user' }}>
          <Form.Item name="name" rules={[{ required: true, message: '请输入用户名' }]}>
            <Input placeholder="用户名" style={{ width: 220 }} autoComplete="off" />
          </Form.Item>
          <Form.Item name="role">
            <Select options={ROLE_OPTIONS} style={{ width: 140 }} aria-label="user-role" />
          </Form.Item>
          <Form.Item>
            <Button type="primary" htmlType="submit" loading={saving}>创建用户</Button>
          </Form.Item>
        </Form>
      </Space>
      <IssuedTokenModal issued={issued} onClose={() => setIssued(null)} />
    </Drawer>
  );
}
