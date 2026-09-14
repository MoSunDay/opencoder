import { Alert, Button, Select, Space, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet, apiPut } from '../api.js';
import { err } from '../notice.js';
import { useMessage } from '../ui/appMessage.js';
import { HARNESS_OPTIONS } from './fields.jsx';

export function AgentHarnessFields({ meta, onNotice, onSaved }) {
  const msg = useMessage();
  const [profiles, setProfiles] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [retry, setRetry] = useState(0);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    if (meta.harness !== 'codex') return;
    let active = true;
    setLoading(true);
    apiGet('/api/harnesses/codex/profiles').then(({ items }) => {
      if (!Array.isArray(items)) throw new Error('配置档案列表无效');
      if (active) { setProfiles(items); setError(''); }
    }).catch((e) => {
      if (active) setError(`读取 Codex 配置档案失败：${e.message}`);
    }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [meta.harness, retry]);

  const save = async (update) => {
    setSaving(true);
    try {
      await apiPut(`/api/agents/${encodeURIComponent(meta.name)}`, update);
      await onSaved();
      msg.success('Agent 执行方式已更新，仅影响新任务');
    } catch (e) { onNotice(err(`更新 Agent 执行方式失败：${e.message}`)); }
    finally { setSaving(false); }
  };
  return <div style={{ marginBottom: 16 }}>
    <Space wrap>
      <Typography.Text>执行方式</Typography.Text>
      <Select aria-label="agent-default-harness" value={meta.harness || 'opencoder'}
        options={HARNESS_OPTIONS} disabled={saving} style={{ minWidth: 150 }}
        onChange={(harness) => save({ harness })} />
      {meta.harness === 'codex' && <>
        <Typography.Text>参数配置</Typography.Text>
        <Select aria-label="agent-harness-profile" value={meta.harness_profile || ''}
          loading={loading} disabled={loading || !!error || saving} style={{ minWidth: 220 }}
          options={[{ value: '', label: '默认 Codex 配置' }, ...profiles.map((p) => ({ value: p.name, label: `${p.name} · v${p.revision}` }))]}
          onChange={(profile) => save({ harness_profile: profile || null })} />
      </>}
    </Space>
    {meta.harness === 'codex' && error && <Alert type="error" showIcon title={error}
      action={<Button size="small" onClick={() => setRetry((v) => v + 1)}>重试</Button>} />}
  </div>;
}
