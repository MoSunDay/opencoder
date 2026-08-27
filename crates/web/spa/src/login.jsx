// login.jsx — shared-secret login Modal + the plaintext-HTTP warning Alert.

import { Alert, Button, Form, Input, Modal, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from './api.js';
import { setCredentials, clearCredentials } from './store.js';
import { syncTime } from './time.js';

const { Text } = Typography;

export const HTTP_WARNING = '明文 HTTP：共享密钥可被窃听，生产请走 TLS 反代';

/// Prominent banner whenever the console itself is served over plain HTTP
/// from a non-local host (localhost / 127.0.0.1 are exempt).
export function isInsecureOrigin() {
  return typeof location !== 'undefined'
    && location.protocol === 'http:'
    && !['localhost', '127.0.0.1', '[::1]', '::1'].includes(location.hostname);
}

export function InsecureHttpAlert() {
  if (!isInsecureOrigin()) {
    return null;
  }
  return (
    <Alert
      banner
      type="warning"
      showIcon
      message={HTTP_WARNING}
      style={{ marginBottom: 0 }}
    />
  );
}

/// Shown whenever no token is stored (`oc_token`). Closable: false — without
/// a shared key every signed call 401s, so there is nothing to render behind.
export function LoginModal({ open }) {
  const [form] = Form.useForm();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState('');

  // The vanilla frontend authenticates via ?token=… in the page URL; prefill
  // from it so pasting a bookmarked link just works.
  useEffect(() => {
    if (!open) {
      return;
    }
    const q = new URLSearchParams(location.search);
    form.setFieldsValue({
      base: localStorage.getItem('oc_base') ?? '',
      token: q.get('token') || '',
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
      await syncTime(); // unsigned bootstrap; also proves reachability
      await apiGet('/api/nodes'); // signed probe — proves the token works
      setBusy(false);
    } catch (e) {
      clearCredentials();
      setErr('连接失败: ' + (e && e.message));
      setBusy(false);
    }
  };

  return (
    <Modal
      title="Opencoder Fleet · 登录"
      open={open}
      closable={false}
      maskClosable={false}
      keyboard={false}
      footer={null}
      destroyOnClose={false}
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
        {err ? <Alert type="error" showIcon message={err} style={{ marginBottom: 16 }} /> : null}
        <Button type="primary" htmlType="submit" loading={busy} block>
          连接
        </Button>
      </Form>
    </Modal>
  );
}
