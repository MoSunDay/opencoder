import { Alert, Button, Modal, Typography } from 'antd';
import { useState } from 'react';
import { apiGet } from '../../api.js';

export function CatalogConflict({ error }) {
  const [detail, setDetail] = useState(null);
  const [loading, setLoading] = useState(false);
  const [readError, setReadError] = useState('');
  if (!error) return null;
  const existing = error.body?.details;
  const url = existing?.url;
  const canRead = typeof url === 'string' && /^\/api\/dag\/(defs|binaries)\/[^/]+$/.test(url);
  const inspect = async () => {
    setLoading(true); setReadError('');
    try { setDetail(await apiGet(url)); }
    catch (failure) { setReadError(failure.message); }
    finally { setLoading(false); }
  };
  return <>
    <Alert type="error" showIcon title={error.message || String(error)} style={{ overflowWrap: 'anywhere' }}
      description={canRead && <Button type="link" loading={loading} onClick={inspect}>查看已有项：{existing.name}</Button>} />
    {readError && <Alert type="error" title={readError} />}
    <Modal open={detail !== null} title={`已有项 · ${existing?.name || ''}`} footer={null} onCancel={() => setDetail(null)}>
      <Typography.Paragraph><pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{JSON.stringify(detail, null, 2)}</pre></Typography.Paragraph>
    </Modal>
  </>;
}
