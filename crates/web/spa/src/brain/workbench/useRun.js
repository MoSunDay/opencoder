import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { openStream } from '../../sse.js';
import { LAYERED_SCHEMA_VERSION, isLayeredView, schemaVersionOf } from './layered/model.js';
import { normalizeEvent, terminalV3 } from './v3Model.js';

/// v4 runs carry their whole view in GET /api/brain/runs/:id/layered; v3 runs
/// 404 there and keep the v3 snapshot + /view path. Nothing else is shared:
/// the two schema versions must never be served a foreign payload.
async function readLayeredView(id) {
  try {
    const view = await apiGet(`/api/brain/runs/${encodeURIComponent(id)}/layered`);
    if (!isLayeredView(view)) throw new Error('v4 brain view is missing its layered run');
    return view;
  } catch (error) {
    if (error?.status === 404) return null;
    throw error;
  }
}

async function readView(id) {
  let snapshot = null; let snapshotError = null;
  try { snapshot = await apiGet(`/api/brain/runs/${encodeURIComponent(id)}`); }
  catch (error) { if (error?.status !== 404) throw error; snapshotError = error; }
  const version = schemaVersionOf(snapshot);
  if (version === LAYERED_SCHEMA_VERSION || snapshot === null) {
    const layered = await readLayeredView(id);
    if (layered) return layered;
    if (snapshot === null) throw snapshotError;
  }
  if (version === 3 || (snapshot?.run && snapshot?.operations)) {
    const view = await apiGet(`/api/brain/runs/${encodeURIComponent(id)}/view`);
    if (!view?.run) throw new Error('v3 brain view is missing its scheduler run');
    return view;
  }
  return snapshot;
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
    // v4 has no dedicated run event stream in the locked contract, so a layered
    // view polls while it is not terminal (a v3 view is never polled and keeps
    // its exact previous behaviour).
    let layered = false;
    const alive = () => lifetime.current === epoch;
    setRun(null); setEvents([]); setError('');
    const load = async () => {
      if (loading) { pending = true; return null; } loading = true;
      try { const value = await readView(id); if (alive()) { layered = isLayeredView(value); finished = terminalV3(value); setRun(value); setError(''); } return watermarkOf(value); }
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
          onFrame: (frame) => { if (!alive()) return; const event = normalizeEvent(frame); if (!event) return; setEvents((old) => old.some((e) => e.seq === event.seq) ? old : [...old, event].slice(-200)); load().catch(() => {}); },
        });
      } catch { if (alive()) retryTimer = setTimeout(start, 2000); }
    };
    start();
    pollTimer = setInterval(() => { if (alive() && layered && !finished && !loading) load().catch(() => {}); }, 3000);
    return () => { lifetime.current++; clearTimeout(timer); clearTimeout(retryTimer); clearInterval(pollTimer); stream?.abort(); };
  }, [id]);
  const refresh = useCallback(() => reload.current?.(), []);
  return { run, events, error, connection, refresh };
}
