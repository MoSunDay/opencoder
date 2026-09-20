import { Alert, Button, Space, Tag, Typography } from 'antd';
import { useState } from 'react';
import { apiGet } from '../../../api.js';
import { normalizeEvent } from '../v3Model.js';

export function Timeline({ id, liveEvents }) {
  const [history, setHistory] = useState([]); const [cursor, setCursor] = useState(0); const [more, setMore] = useState(true); const [error, setError] = useState(''); const [historical, setHistorical] = useState(false);
  const load = async () => { try { const page = await apiGet(`/api/brain/runs/${encodeURIComponent(id)}/events-page?after=${cursor}`); const events = (page.events || []).map(normalizeEvent).filter(Boolean); setHistory(events); setCursor(events.at(-1)?.seq || cursor); setMore(page.more); setHistorical(true); } catch (event) { setError(event.message); } };
  const events = historical ? history : liveEvents;
  return <><Space><Typography.Text strong>{historical ? '历史调度事件' : '实时调度事件'}</Typography.Text><Button size="small" disabled={historical && !more} onClick={load}>{historical ? '下一页' : '从头查看'}</Button>{historical && <Button size="small" onClick={() => { setHistorical(false); setCursor(0); setMore(true); }}>回到实时</Button>}</Space>
    {error && <Alert type="error" title={error} />}<div className="brain-events">{events.map((event) => <details key={event.seq}><summary><span>#{event.seq}</span><Tag>{event.event}</Tag><span>{event.data?.reason_summary || event.data?.execution_id || ''}</span></summary><pre className="brain-json">{JSON.stringify(event.data, null, 2)}</pre></details>)}</div></>;
}
