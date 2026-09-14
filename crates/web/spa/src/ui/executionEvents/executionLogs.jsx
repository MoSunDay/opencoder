import { Button, Card, Input, Select, Space, Switch, Tag, Typography } from 'antd';
import { useMemo, useState } from 'react';
import { apiGet } from '../../api.js';
import { logRows } from './model.js';

const { Text } = Typography;
export function ExecutionLogs({ id, frames = [], steps = [], step = '', onStepChange, connection, trimmed = false }) {
  const [query, setQuery] = useState('');
  const [historical, setHistorical] = useState(null);
  const [historyCursor, setHistoryCursor] = useState(0);
  const [historyMore, setHistoryMore] = useState(false);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [autoScroll, setAutoScroll] = useState(true);
  const source = historical ? historical : frames;
  const rows = useMemo(() => logRows(source, step, query), [source, step, query]);
  const loadHistory = async (after = 0, append = false) => {
    if (!id || historyLoading) return;
    setHistoryLoading(true);
    try {
      const page = await apiGet(`/api/executions/${encodeURIComponent(id)}/events-page?after=${after}`);
      const incoming = (page.events || []).map((event) => ({ seq: event.seq, event: event.kind || event.event, data: event.data || event.payload || {} }));
      setHistorical((old) => append ? [...(old || []), ...incoming] : incoming);
      setHistoryCursor(incoming.at(-1)?.seq || after);
      setHistoryMore(Boolean(page.more));
    } finally { setHistoryLoading(false); }
  };
  return <Card size="small" title="实时日志" extra={<Tag>{connection || '连接中'}</Tag>}>
    <Space wrap style={{ marginBottom: 8 }}>
      <Select allowClear placeholder="全部步骤" value={step || undefined} onChange={(v) => onStepChange?.(v || '')} options={steps.map((s) => ({ label: s, value: s }))} style={{ minWidth: 150 }} />
      <Input aria-label="搜索日志" allowClear placeholder="搜索日志" value={query} onChange={(e) => setQuery(e.target.value)} style={{ width: 220 }} />
      <Button size="small" onClick={() => setQuery('')}>清除</Button>
      <Switch checked={autoScroll} onChange={setAutoScroll} aria-label="自动滚动" />
      {trimmed && !historical && <Button size="small" onClick={() => loadHistory(0)}>从头查看历史</Button>}
      {historical && <>
        <Button size="small" onClick={() => loadHistory(historyCursor, true)} disabled={!historyMore || historyLoading}>下一页</Button>
        <Button size="small" onClick={() => setHistorical(null)}>返回实时日志</Button>
      </>}
    </Space>
    {connection === 'failed' && <Text type="danger">日志连接失败，请重新连接</Text>}
    <div role="log" style={{ maxHeight: 300, overflow: 'auto', background: '#111827', color: '#d1d5db', padding: 10, fontFamily: 'monospace', whiteSpace: 'pre-wrap' }}>
      {rows.length ? rows.map((r, i) => <div key={`${r.seq}:${i}`}><Text type="secondary">[{r.step || '-'}] </Text>{r.text}</div>) : <Text type="secondary">{query ? '没有匹配的日志' : '等待步骤日志…'}</Text>}
    </div>
  </Card>;
}
