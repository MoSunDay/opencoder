import { Alert, Button, Space } from 'antd';
import { useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { PayloadWindows } from '../../fleet/detail/fields.jsx';

export function RunEvents({ id }) {
  const generation = useRef(0);
  const [page, setPage] = useState(null);
  const [cursors, setCursors] = useState([0]);
  const [index, setIndex] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const load = async (after, target) => {
    const current = ++generation.current;
    setBusy(true);
    try {
      const result = await apiGet(`/api/executions/${encodeURIComponent(id)}/events-page?after=${after}`);
      if (current !== generation.current) return;
      setPage(result);
      setIndex(target); setError('');
    } catch (e) { if (current === generation.current) setError(e.message); }
    finally { if (current === generation.current) setBusy(false); }
  };
  useEffect(() => { setCursors([0]); setIndex(0); setPage(null); load(0, 0); return () => { generation.current += 1; }; }, [id]);
  const next = () => {
    const after = page?.events?.at(-1)?.seq;
    if (!after) return;
    setCursors((old) => old.slice(0, index + 1).concat(after)); load(after, index + 1);
  };
  return <div>
    {error && <Alert type="error" title={error} />}
    <Space>
      <Button disabled={busy || index === 0} onClick={() => load(cursors[index - 1], index - 1)}>上一页事件</Button>
      <Button disabled={busy || !page?.more} onClick={next}>下一页事件</Button>
      <Button loading={busy} onClick={() => load(cursors[index], index)}>刷新事件</Button>
    </Space>
    {(page?.events || []).map((event) => <div key={event.seq}>
      <pre>#{event.seq} {event.kind}{'\n'}{event.data?.omitted ? '内容按段加载' : JSON.stringify(event.data, null, 2)}</pre>
      {event.data?.omitted && <PayloadWindows id={id} marker={event.data} seq={event.seq} label="读取完整事件" />}
    </div>)}
  </div>;
}
