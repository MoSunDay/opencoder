import { Alert, Button, Descriptions, Pagination, Select, Space, Spin, Typography } from 'antd';
import { useEffect, useState } from 'react';
import { apiGet } from '../../api.js';
import { StatusTag } from '../../ui/statusTag.jsx';
import { StepPanel } from '../step/stepPanel.jsx';
import { progressLabel } from './model.js';

export function Instances({ runId, step }) {
  const [page, setPage] = useState(1);
  const [selected, setSelected] = useState(null);
  const [listing, setListing] = useState(null);
  const [receipt, setReceipt] = useState(null);
  const [listError, setListError] = useState('');
  const [detailError, setDetailError] = useState('');
  const error = [listError, detailError].filter(Boolean).join('；');
  const [retry, setRetry] = useState(0);
  const base = `/api/dag/runs/${encodeURIComponent(runId)}/steps/${encodeURIComponent(step)}/instances`;
  useEffect(() => {
    const controller = new AbortController();
    let pending = false;
    setListing(null);
    const load = async () => {
      if (pending) return;
      pending = true;
      try {
        const value = await apiGet(`${base}?offset=${(page - 1) * 100}&limit=100`, { signal: controller.signal });
        if (controller.signal.aborted) return;
        setListing(value); setListError('');
        setSelected((old) => old ?? value.instances?.[0]?.index ?? null);
      } catch (e) { if (!controller.signal.aborted) setListError(e.message); }
      finally { pending = false; }
    };
    void load();
    const timer = setInterval(load, 3000);
    return () => { controller.abort(); clearInterval(timer); };
  }, [base, page, retry]);
  useEffect(() => {
    setReceipt(null); setDetailError('');
    if (selected === null) return undefined;
    const controller = new AbortController();
    let pending = false;
    const load = async () => {
      if (pending) return;
      pending = true;
      try {
        const value = await apiGet(`${base}/${selected}`, { signal: controller.signal });
        if (!controller.signal.aborted) { setReceipt(value); setDetailError(''); }
      } catch (e) { if (!controller.signal.aborted) setDetailError(e.message); }
      finally { pending = false; }
    };
    void load();
    const timer = setInterval(load, 2000);
    return () => { controller.abort(); clearInterval(timer); };
  }, [base, selected, retry]);
  return <Space orientation="vertical" style={{ width: '100%' }} size={12}>
    <Typography.Text>{progressLabel(listing?.expanded ? listing.progress || { total: listing.total, done: 0 } : null)}</Typography.Text>
    {listing?.progress && <Typography.Text type="secondary">运行 {listing.progress.running} · 失败 {listing.progress.error} · 取消 {listing.progress.cancelled} · 待执行 {listing.progress.pending}</Typography.Text>}
    {error && <Alert type="error" title={error} action={<Button onClick={() => setRetry((n) => n + 1)}>重试</Button>} />}
    {!listing && !error && <Spin size="small" />}
    {!!listing?.total && <>
      <Select aria-label="选择实例" style={{ width: '100%' }} value={selected}
        options={(listing.instances || []).map((i) => ({ value: i.index, label: `实例 ${i.index} · ${i.status}` }))}
        onChange={setSelected} />
      <Pagination current={page} pageSize={100} total={listing.total} showSizeChanger={false}
        onChange={(next) => { setPage(next); setSelected((next - 1) * 100); }} />
    </>}
    {receipt && <>
      <Descriptions size="small" items={[
        { key: 'status', label: '实例状态', children: <StatusTag status={receipt.status} /> },
        { key: 'session', label: '会话', children: receipt.session_id || '—' },
      ]} />
      <Typography.Text strong>实例输入</Typography.Text>
      <pre style={{ whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' }}>{JSON.stringify(receipt.input, null, 2)}</pre>
      {receipt.error && <Alert type="error" title={receipt.error} />}
      <StepPanel key={`${base}/${selected}/${receipt.started_at_ms || 0}`} runId={runId} step={step} index={selected} kind={receipt.kind} />
    </>}
  </Space>;
}
