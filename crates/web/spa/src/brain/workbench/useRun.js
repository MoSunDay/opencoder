import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { openStream } from '../../sse.js';
import { isLayeredView, terminalPhase } from './layered/model.js';
async function readView(id) {
  const view = await apiGet(`/api/brain/runs/${encodeURIComponent(id)}/layered`);
  if (!isLayeredView(view)) throw new Error('运行缺少分层调度数据');
  return view;
}

function watermarkOf(value) {
  return value?.run?.last_event_seq ?? value?.watermark ?? 0;
}

export function useBrainRun(id) {
  const [run, setRun] = useState(null); const [error, setError] = useState(''); const [connection, setConnection] = useState('connecting');
  const [events, setEvents] = useState([]); const lifetime = useRef(0);
  const reload = useRef(null);
  useEffect(() => {
    const epoch = ++lifetime.current; let stream; let timer; let retryTimer; let pollTimer; let loading = false; let pending = false; let finished = false;
    const alive = () => lifetime.current === epoch;
    setRun(null); setEvents([]); setError('');
    const load = async () => {
      if (loading) { pending = true; return null; } loading = true;
      try { const value = await readView(id); if (alive()) { finished = terminalPhase(value.run.phase); setRun(value); setError(''); } return watermarkOf(value); }
      catch (e) { if (alive()) setError(e.message); throw e; }
      finally { loading = false; if (pending && alive()) { pending = false; timer = setTimeout(() => load().catch(() => {}), 50); } }
    };
    reload.current = load;
    const start = async () => {
      if (!alive()) return;
      try {
        const after = await load(); if (!alive()) return;
        if (finished) { setConnection('closed'); return; }
        stream = openStream({ path: `/api/brain/runs/${encodeURIComponent(id)}/events`, after, executionHistory: true,
          onResync: async () => { const watermark = await load(); if (watermark === null) throw new Error('快照更新中，请重试同步'); return watermark; },
          onStatus: (status) => { if (alive()) { setConnection(status); if (status === 'closed' && !finished) { clearTimeout(retryTimer); retryTimer = setTimeout(start, 2000); } } },
          onFrame: (frame) => { if (!alive()) return; const event = frame.data || frame; if (!event) return; setEvents((old) => old.some((e) => e.seq === event.seq) ? old : [...old, event].slice(-200)); load().catch(() => {}); },
        });
      } catch { if (alive()) retryTimer = setTimeout(start, 2000); }
    };
    start();
    pollTimer = setInterval(() => { if (alive() && !finished && !loading) load().catch(() => {}); }, 3000);
    return () => { lifetime.current++; clearTimeout(timer); clearTimeout(retryTimer); clearInterval(pollTimer); stream?.abort(); };
  }, [id]);
  const refresh = useCallback(() => reload.current?.(), []);
  return { run, events, error, connection, refresh };
}
