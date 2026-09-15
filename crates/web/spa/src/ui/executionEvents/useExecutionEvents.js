import { useCallback, useEffect, useRef, useState } from 'react';
import { apiGet } from '../../api.js';
import { openStream } from '../../sse.js';
import { appendLog, initialLogs, pageFrames } from './model.js';

export function useExecutionEvents({ id, status, onFrame, enabled = true, hydrate = false }) {
  const [connection, setConnection] = useState('connecting');
  const [error, setError] = useState('');
  const [logs, setLogs] = useState(initialLogs);
  const state = useRef(initialLogs());
  const previousId = useRef(id);
  const callbacks = useRef(onFrame);
  callbacks.current = onFrame;
  const [retryNonce, setRetryNonce] = useState(0);
  useEffect(() => {
    if (!id || !enabled) return undefined;
    const controller = new AbortController();
    let stream;
    if (previousId.current !== id) { state.current = initialLogs(); previousId.current = id; }
    setLogs(state.current); setError(''); setConnection('connecting');
    const connect = () => {
      stream = openStream({ path: `/api/executions/${encodeURIComponent(id)}/events`,
        after: state.current.cursor, executionHistory: true, requireEnd: true,
        onStatus: (next) => {
          if (!controller.signal.aborted) setConnection((old) => old === 'failed' && next === 'closed' ? old : next);
        },
        onFrame: (frame) => {
          if (controller.signal.aborted) return;
          const next = appendLog(state.current, frame);
          if (next === state.current) return;
          state.current = next; setLogs(next); callbacks.current?.(frame);
        },
      });
    };
    if (!hydrate) connect();
    else (async () => {
      try {
        let batch = state.current;
        let head;
        let finished = false;
        for (;;) {
          const page = await apiGet(`/api/executions/${encodeURIComponent(id)}/events-page?after=${batch.cursor}`, { signal: controller.signal });
          if (controller.signal.aborted) return;
          if (head === undefined) {
            head = page.head_seq;
            if (!Number.isSafeInteger(head) || head < 0) throw new Error('节点未返回有效的日志位置');
          }
          const before = batch.cursor;
          for (const frame of pageFrames(page)) batch = appendLog(batch, frame);
          finished = page.finished === true && !page.more;
          if (batch.cursor >= head) break;
          if (batch.cursor <= before || !page.more) throw new Error('日志分页未到达当前结果位置');
        }
        // Commit history once. Only subsequent live frames render incrementally.
        state.current = batch; setLogs(batch);
        if (finished) setConnection('closed'); else connect();
      } catch (e) {
        if (!controller.signal.aborted) { setError(e.message); setConnection('failed'); }
      }
    })();
    return () => { controller.abort(); stream?.abort(); };
  }, [id, enabled, hydrate, status, retryNonce]);
  const retry = useCallback(() => setRetryNonce((value) => value + 1), []);
  return { ...logs, connection, error, retry, status };
}
