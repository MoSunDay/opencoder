import { Alert, Button, Spin } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../../api.js';
import { PayloadWindows } from '../fields.jsx';
import { decodeRecord } from './model.js';

export function TeamRecord({ id, field, label, children }) {
  const [state, setState] = useState({ busy: true });
  const [retry, setRetry] = useState(0);
  const key = JSON.stringify([id, field]);
  useEffect(() => {
    const controller = new AbortController();
    setState({ key, busy: true });
    const query = new URLSearchParams({ field, offset: '0' });
    apiGet(`/api/executions/${encodeURIComponent(id)}/detail-field?${query}`, { signal: controller.signal })
      .then((chunk) => { if (!controller.signal.aborted) setState({ key, ...decodeRecord(chunk) }); })
      .catch((error) => { if (!controller.signal.aborted) setState({ key, error: error.message || '读取讨论记录失败' }); });
    return () => controller.abort();
  }, [id, field, key, retry]);
  if (state.key !== key || state.busy) return <Spin size="small" aria-label={`正在读取${label}`} />;
  if (state.error) return <Alert type="error" showIcon title={`${label}读取失败`} description={state.error}
    action={<Button size="small" onClick={() => setRetry((value) => value + 1)}>重试</Button>} />;
  if (state.large) return <div>
    <p>这份{label}较长，可分段查看完整内容。</p>
    <PayloadWindows id={id} marker={{ omitted: true, field, read_via: 'detail_field' }} label={`分段查看${label}`} />
  </div>;
  return children(state.value);
}
