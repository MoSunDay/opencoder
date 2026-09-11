import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { openStream } from '../../sse.js';
export function useBrainRun(id) {
  const [run, setRun] = useState(null); const [error, setError] = useState(''); const [connection, setConnection] = useState('connecting');
  const [events, setEvents] = useState([]); const lifetime = useRef(0);
  const reload = useRef(null);
  useEffect(() => {
    const epoch = ++lifetime.current; let stream; let timer; let loading = false; let pending = false;
    const alive = () => lifetime.current === epoch;
    setRun(null); setEvents([]); setError('');
    const load = async () => {
      if (loading) { pending = true; return null; } loading = true;
      try { const value = await apiGet(`/api/brain/runs/${encodeURIComponent(id)}`); if (alive()) { setRun(value); setError(''); } return value.watermark || 0; }
      catch (e) { if (alive()) setError(e.message); throw e; }
      finally { loading = false; if (pending && alive()) { pending = false; timer = setTimeout(() => load().catch(() => {}), 50); } }
    };
    reload.current = load;
    const start = async () => {
      try {
        const after = await load(); if (!alive()) return;
        stream = openStream({ path: `/api/brain/runs/${encodeURIComponent(id)}/events`, after, executionHistory: true,
          onResync: async () => { const watermark = await load(); if (watermark === null) throw new Error('快照更新中，请重试同步'); return watermark; },
          onStatus: (status) => { if (alive()) setConnection(status); },
          onFrame: (frame) => { if (!alive()) return; setEvents((old) => old.some((e) => e.seq === frame.seq) ? old : [...old, frame].slice(-200)); load().catch(() => {}); },
        });
      } catch { if (alive()) timer = setTimeout(start, 2000); }
    };
    start();
    return () => { lifetime.current++; clearTimeout(timer); stream?.abort(); };
  }, [id]);
  const refresh = useCallback(() => reload.current?.(), []);
  return { run, events, error, connection, refresh };
}
