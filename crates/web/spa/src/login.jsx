// login.jsx — shared-secret login Modal.

import { Alert, Button, Form, Input, Modal, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from './api.js';
import { BASE_KEY, embeddedBase, setCredentials, clearToken } from './store.js';

const { Text } = Typography;

/// Shown whenever no token is stored (`oc_token`). Closable: false — without
/// a shared key every protected call 401s, so there is nothing to render behind.
export function LoginModal({ open, onConnected }) {
  const [form] = Form.useForm();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');

  // Link login (?token= / #token=) is adopted in boot.js BEFORE mount;
  // this modal only handles interactive logins.
  // Prefill: stored base, else the build-time embedded base (VITE_OC_BASE).
  useEffect(() => {
    if (!open) {
      return;
    }
    form.setFieldsValue({
      base: localStorage.getItem(BASE_KEY) ?? embeddedBase(),
      token: '',
    });
  }, [open, form]);

  const submit = async (values) => {
    setBusy(true);
    setErr('');
    const token = (values.token || '').trim();
    const base = (values.base || '').trim();
    if (!token) {
      setErr('共享密钥不能为空');
      setBusy(false);
      return;
    }
    setCredentials(token, base);
    try {
      await apiGet('/api/nodes'); // protected probe proves reachability + token
      onConnected?.();
      setBusy(false);
    } catch (e) {
      // Failed probe: drop the token but keep the base the user/URL gave —
      // retrying with the same address and a fixed token is the common path.
      clearToken();
      setErr('连接失败: ' + (e && e.message));
      setBusy(false);
    }
  };

  return (
    <Modal
      title="Opencoder Fleet · 登录"
      open={open}
      closable={false}
      mask={{ closable: false }}
      keyboard={false}
      footer={null}
      destroyOnHidden={false}
    >
      <Form form={form} layout="vertical" onFinish={submit} initialValues={{ base: '' }}>
        <Form.Item
          name="base"
          label="服务器地址"
          extra={(
            <Text type="secondary">
              留空 = 同源 (same-origin)。示例: https://fleet.example.com
            </Text>
          )}
        >
          <Input placeholder="留空 = 同源 (same-origin)" allowClear autoComplete="off" />
        </Form.Item>
        <Form.Item name="token" label="共享密钥 (Token)">
          <Input.Password placeholder="共享密钥" autoFocus />
        </Form.Item>
        {err ? <Alert type="error" showIcon title={err} style={{ marginBottom: 16 }} /> : null}
        <Button type="primary" htmlType="submit" loading={busy} block>
          连接
        </Button>
      </Form>
    </Modal>
  );
}
