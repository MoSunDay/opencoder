import { Alert, Button, Form, Select } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../api.js';
import { parseResource, resourceToken, versionOptions } from './model.js';
import { readHistory, readPools } from './read.js';

export function BinaryResourceField({ value, onChange }) {
  const resource = parseResource(value);
  const name = resource?.name || '';
  const [pools, setPools] = useState([]);
  const [pool, setPool] = useState(null);
  const [error, setError] = useState('');
  const [loading, setLoading] = useState(true);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    const controller = new AbortController();
    setLoading(true); setError(''); setPool(null);
    Promise.all([apiGet('/api/dag/binaries', { signal: controller.signal }), name ? apiGet(`/api/dag/binaries/${encodeURIComponent(name)}`, { signal: controller.signal }) : null])
      .then(([list, selected]) => { if (!controller.signal.aborted) { setPools(readPools(list)); setPool(selected ? readHistory(selected) : null); } })
      .catch((failure) => { if (!controller.signal.aborted) setError(failure.message); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    return () => controller.abort();
  }, [name, revision]);
  return <>
    <Form.Item label="二进制资源" extra="在 DAG 页的二进制资源中上传 Linux 可执行文件。">
      <Select aria-label="二进制资源" showSearch optionFilterProp="label" loading={loading} disabled={loading || !!error} value={name || undefined}
        placeholder="选择二进制资源" options={pools.map((item) => ({ value: item.name, label: item.name }))} onChange={(selected) => onChange(selected)} />
    </Form.Item>
    {error && <Alert type="error" showIcon title={`资源读取失败：${error}`} action={<Button size="small" onClick={() => setRevision((current) => current + 1)}>重试资源</Button>} />}
    {!loading && !error && !pools.length && <Alert type="info" title="暂无二进制资源，请先上传" />}
    {name && <Form.Item label="资源版本" extra="受理后保存实际版本；资源更新不会改变这次运行。">
      <Select aria-label="二进制资源版本" disabled={!pool || !!error || loading} value={resource?.version || 0} options={versionOptions(pool)} onChange={(version) => onChange(resourceToken(name, version))} />
    </Form.Item>}
  </>;
}
