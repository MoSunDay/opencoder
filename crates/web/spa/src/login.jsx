// login.jsx — token-only login Modal. The server base is NOT asked here: it
// comes from the URL link (?base=), the stored oc_base, or the build-time
// VITE_OC_BASE embed (store.js embeddedBase) — the probe below reuses the
// CURRENT stored base untouched.

import { Alert, Button, Form, Input, Modal } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from './api.js';
import { clearToken, getState, setCredentials, setIdentity } from './store.js';

/// Shown whenever no token is stored (`oc_token`). Closable: false — without
/// a shared key every protected call 401s, so there is nothing to render behind.
export function LoginModal({ open, onConnected }) {
  const [form] = Form.useForm();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');

  // Link login (?token= / #token=) is adopted in boot.js BEFORE mount;
  // this modal only handles interactive logins. Only the token is editable.
  useEffect(() => {
    if (!open) {
      return;
    }
    form.setFieldsValue({ token: '' });
  }, [open, form]);

  const submit = async (values) => {
    setBusy(true);
    setErr('');
    const token = (values.token || '').trim();
    if (!token) {
      setErr('访问令牌不能为空');
      setBusy(false);
      return;
    }
    // Keep the CURRENT base (stored / URL-delivered / embedded) — login is
    // token-only, the address never changes hands here.
    setCredentials(token, getState().base);
    try {
      const j = await apiGet('/api/me'); // protected probe: reachability + identity
      setIdentity(j);
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
      <Form form={form} layout="vertical" onFinish={submit}>
        <Form.Item name="token" label="访问令牌 (Token)">
          <Input.Password placeholder="访问令牌" autoFocus />
        </Form.Item>
        {err ? <Alert type="error" showIcon title={err} style={{ marginBottom: 16 }} /> : null}
        <Button type="primary" htmlType="submit" loading={busy} block>
          连接
        </Button>
      </Form>
    </Modal>
  );
}
