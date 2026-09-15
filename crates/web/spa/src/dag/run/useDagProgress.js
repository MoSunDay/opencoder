import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { openStream } from '../../sse.js';
import { applyDagFrame, checkedSnapshot, isActive } from './model.js';

export function useDagProgress({ id, status, onStatus }) {
  const [snapshot, setSnapshot] = useState(null);
  const [error, setError] = useState('');
  const [connection, setConnection] = useState('connecting');
  const [revision, setRevision] = useState(0);
  const state = useRef(null);
  const position = useRef(0);
  const notify = useRef(onStatus);
  notify.current = onStatus;
  const reload = useRef(null);
  const retry = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    const controller = new AbortController();
    let stream;
    let pending;
    let streamActive = false;
    state.current = null;
    position.current = 0;
    setSnapshot(null);
    setError('');
    setConnection('connecting');
    const publish = (value) => {
      state.current = value;
      setSnapshot(value);
      notify.current?.(value.execution_status, value.execution_error);
    };
    const connect = () => {
      if (streamActive || !isActive(state.current?.execution_status)) return;
      streamActive = true;
      stream = openStream({ path: `/api/executions/${encodeURIComponent(id)}/events`,
        after: position.current, executionHistory: true, requireEnd: true,
        onResync: async () => { await load(); return position.current; },
        onFrame: (frame) => {
          if (controller.signal.aborted) return;
          position.current = Math.max(position.current, frame.seq || 0);
          const next = applyDagFrame(state.current, frame);
          if (next !== state.current) publish(next);
          if (frame.event === 'run_finished') void load().catch(() => {});
        },
        onStatus: (value) => {
          if (controller.signal.aborted) return;
          setConnection(value);
          if (value === 'failed') setError('节点状态连接失败，请重试');
          if (value === 'closed') streamActive = false;
        },
      });
    };
    const load = () => {
      if (pending) return pending;
      pending = (async () => {
        try {
          let value = checkedSnapshot(await apiGet(`/api/dag/runs/${encodeURIComponent(id)}/progress`, { signal: controller.signal }));
          if (controller.signal.aborted) return;
          // run_finished is flushed before the worker journal settles. The
          // same watermark cannot start another attempt; wait for newer data.
          if (state.current && !isActive(state.current.execution_status) && isActive(value.execution_status)
            && value.head_seq === position.current) {
            value = { ...value, execution_status: state.current.execution_status, execution_error: state.current.execution_error };
          }
          // Do not overwrite frames delivered while the snapshot was in flight.
          if (value.head_seq >= position.current) { position.current = value.head_seq; publish(value); }
          setError('');
          if (isActive(state.current?.execution_status)) connect();
          else { stream?.abort(); streamActive = false; setConnection('closed'); }
        } catch (e) {
          if (!controller.signal.aborted) { setError(e.message); setConnection('failed'); }
          throw e;
        } finally { pending = null; }
      })();
      return pending;
    };
    reload.current = load;
    void load().catch(() => {});
    const timer = setInterval(() => { void load().catch(() => {}); }, 3000);
    return () => { controller.abort(); clearInterval(timer); stream?.abort(); reload.current = null; };
  }, [id, revision]);

  useEffect(() => { void reload.current?.().catch(() => {}); }, [status]);
  return { snapshot, error, connection, retry };
}
