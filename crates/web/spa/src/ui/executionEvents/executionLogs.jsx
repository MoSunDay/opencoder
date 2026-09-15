import { Alert, Button, Input, Select, Space, Spin, Switch, Tag, Typography } from 'antd';
import { useEffect, useMemo, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { logRows, pageFrames } from './model.js';

const { Text } = Typography;
const CONNECTION = { connecting: '加载日志中', open: '已连接', live: '实时', reconnecting: '重连中', closed: '已结束', failed: '连接失败' };
export function ExecutionLogs({ id, frames = [], steps = [], step = '', onStepChange, connection, trimmed = false, error, retry }) {
  const [query, setQuery] = useState('');
  const [historical, setHistorical] = useState(null);
  const [historyCursor, setHistoryCursor] = useState(0);
  const [historyMore, setHistoryMore] = useState(false);
  const [historyLoading, setHistoryLoading] = useState(false);
  const [historyError, setHistoryError] = useState('');
  const [autoScroll, setAutoScroll] = useState(true);
  const log = useRef(null);
  const request = useRef(null);
  const source = historical ?? frames;
  const rows = useMemo(() => logRows(source, step, query), [source, step, query]);
  useEffect(() => () => request.current?.abort(), [id]);
  useEffect(() => {
    if (autoScroll && !historical && log.current) log.current.scrollTop = log.current.scrollHeight;
  }, [rows, autoScroll, historical]);
  const loadHistory = async (after = 0) => {
    if (!id || historyLoading) return;
    const controller = new AbortController();
    request.current = controller;
    setHistoryLoading(true); setHistoryError('');
    try {
      const page = await apiGet(`/api/executions/${encodeURIComponent(id)}/events-page?after=${after}`, { signal: controller.signal });
      if (controller.signal.aborted) return;
      const incoming = pageFrames(page);
      const cursor = incoming.at(-1)?.seq || after;
      if (page.more && cursor <= after) throw new Error('日志分页未向前推进');
      // Each history page replaces the previous window, keeping memory bounded.
      setHistorical(incoming); setHistoryCursor(cursor); setHistoryMore(Boolean(page.more));
    } catch (e) { if (!controller.signal.aborted) setHistoryError(e.message); }
    finally { if (!controller.signal.aborted) setHistoryLoading(false); }
  };
  const empty = query ? '没有匹配的日志' : trimmed && !historical ? '当前窗口没有此步骤的日志，可从头查看历史' : connection === 'closed' || historical ? '此步骤暂无日志' : '等待步骤日志…';
  return <div className="execution-logs">
    <Space wrap>
      <Select aria-label="切换步骤日志" value={step} onChange={(v) => onStepChange?.(v)}
        options={[{ label: '全部步骤', value: '' }, ...steps.map((s) => ({ label: s, value: s }))]} style={{ minWidth: 150 }} />
      <Input aria-label="搜索日志" allowClear placeholder="搜索日志" value={query} onChange={(e) => setQuery(e.target.value)} style={{ width: 220 }} />
      <Button size="small" onClick={() => setQuery('')}>清除</Button>
      <Switch checked={autoScroll} onChange={setAutoScroll} aria-label="自动滚动" /><Text>自动滚动</Text>
      <Tag>{CONNECTION[connection] || '连接中'}</Tag>
      {trimmed && !historical && <Button size="small" loading={historyLoading} onClick={() => loadHistory(0)}>从头查看历史</Button>}
      {historical && <>
        <Button size="small" onClick={() => loadHistory(0)} loading={historyLoading}>从头查看历史</Button>
        <Button size="small" onClick={() => loadHistory(historyCursor)} disabled={!historyMore || historyLoading}>下一页</Button>
        <Button size="small" onClick={() => { setHistorical(null); setHistoryError(''); }}>返回实时日志</Button>
      </>}
    </Space>
    {(error || connection === 'failed') && <Alert type="error" title={error || '日志连接失败，请重新连接'} action={retry && <Button size="small" onClick={retry}>重新连接</Button>} />}
    {historyError && <Alert type="error" title={historyError} action={<Button size="small" onClick={() => loadHistory(historical ? historyCursor : 0)}>重试分页</Button>} />}
    {connection === 'connecting' && !frames.length && <Spin size="small" />}
    <div ref={log} role="log" className="execution-log-lines">
      {rows.length ? rows.map((r, i) => <div key={`${r.seq}:${i}`}><Text type="secondary">[{r.step || '-'}] </Text>{r.text}</div>) : <Text type="secondary">{empty}</Text>}
    </div>
  </div>;
}
